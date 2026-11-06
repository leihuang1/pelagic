#![forbid(unsafe_code)]

//! Synchronous HTTP facade for modern, self-contained BELLHOP cases.

use std::io::{self, Write};
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::error_handling::HandleErrorLayer;
use axum::extract::{FromRequest, Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bellhop::Case;
use bellhop::diagnostic::{Diagnostic, DiagnosticReport, LoadOutcome, Severity};
use bellhop::json::document::{BoundaryDocument, VolumeAttenuationDocument};
use bellhop::json::{CaseDocument, DocumentErrorKind};
use bellhop::model::RunKind;
use bellhop::solver::{
    Arrival, FieldSample, ReceiverArrivals, SimulationLimits, SimulationResult, SourceArrivals,
    SourceField, run as run_simulation,
};
use serde::Serialize;
use serde::ser::Serializer;
use tokio::sync::Semaphore;
use tower::limit::ConcurrencyLimitLayer;
use tower::timeout::TimeoutLayer;
use tower::{BoxError, ServiceBuilder};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

pub const HDF5_MEDIA_TYPE: &str = "application/x-hdf5";
const JSON_MEDIA_TYPE: &str = "application/json";

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub workers: usize,
    pub request_timeout: Duration,
    pub max_body_bytes: usize,
    pub max_json_response_bytes: usize,
    pub simulation_limits: SimulationLimits,
    /// Threads for each admitted simulation; results are identical for any count.
    pub solver_threads: NonZeroUsize,
    pub auth_token: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            workers: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
            request_timeout: Duration::from_secs(120),
            max_body_bytes: 16 * 1024 * 1024,
            max_json_response_bytes: 64 * 1024 * 1024,
            simulation_limits: SimulationLimits::default(),
            solver_threads: NonZeroUsize::MIN,
            auth_token: None,
        }
    }
}

#[derive(Clone)]
struct AppState {
    simulation_limits: SimulationLimits,
    solver_threads: NonZeroUsize,
    max_json_response_bytes: usize,
    auth_token: Option<Arc<str>>,
    worker_slots: Arc<Semaphore>,
}

/// Builds the complete service, including public health and `OpenAPI` routes.
///
/// # Panics
///
/// Panics when `workers`, `max_body_bytes`, or `max_json_response_bytes` is
/// zero. The executable validates these values before constructing the service.
pub fn app(config: ServerConfig) -> Router {
    assert!(config.workers > 0, "workers must be positive");
    assert!(config.max_body_bytes > 0, "max_body_bytes must be positive");
    assert!(
        config.max_json_response_bytes > 0,
        "max_json_response_bytes must be positive"
    );

    let state = AppState {
        simulation_limits: config.simulation_limits,
        solver_threads: config.solver_threads,
        max_json_response_bytes: config.max_json_response_bytes,
        auth_token: config.auth_token.map(Arc::from),
        worker_slots: Arc::new(Semaphore::new(config.workers)),
    };
    let api = Router::new()
        .route("/v1/validate", post(validate_case))
        .route("/v1/run", post(run_case))
        .route("/v1/arrivals", post(arrivals_case))
        .route("/v1/field", post(field_case))
        .layer(axum::extract::DefaultBodyLimit::max(config.max_body_bytes))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(handle_middleware_error))
                .layer(TimeoutLayer::new(config.request_timeout))
                .layer(ConcurrencyLimitLayer::new(config.workers)),
        )
        .with_state(state);

    Router::new()
        .route("/healthz", get(healthz))
        .merge(api)
        .merge(SwaggerUi::new("/docs").url("/openapi.json", ApiDoc::openapi()))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(tracing::Level::INFO))
                .on_response(DefaultOnResponse::new().level(tracing::Level::INFO)),
        )
}

#[derive(OpenApi)]
#[openapi(
    paths(healthz, validate_case, run_case, arrivals_case, field_case),
    components(schemas(
        CaseDocument,
        // Referenced from custom "name or object" field schemas, so not collected automatically.
        BoundaryDocument,
        VolumeAttenuationDocument,
        HealthResponse,
        ValidationResponse,
        DiagnosticResponse,
        DiagnosticLocationResponse,
        DiagnosticSeverityResponse,
        ErrorResponse,
        ErrorDetail,
        Hdf5Response,
        ArrivalsResponse,
        ArrivalSourceResponse,
        ArrivalReceiverResponse,
        ArrivalResponse,
        FieldResponse,
        FieldRunKindResponse,
        FieldSourceResponse,
        FieldReceiverResponse,
        PressureResponse
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "service", description = "Service health"),
        (name = "simulation", description = "BELLHOP validation and simulation")
    )
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .description(Some(
                            "Static bearer token when BELLHOP_AUTH_TOKEN is configured",
                        ))
                        .build(),
                ),
            );
        }
    }
}

#[derive(Serialize, ToSchema)]
struct HealthResponse {
    status: &'static str,
}

#[utoipa::path(
    get,
    path = "/healthz",
    tag = "service",
    responses((status = 200, description = "Service is ready", body = HealthResponse))
)]
async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[derive(Serialize, ToSchema)]
struct ValidationResponse {
    valid: bool,
    warnings: Vec<DiagnosticResponse>,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum DiagnosticSeverityResponse {
    Warning,
    Error,
}

#[derive(Clone, Serialize, ToSchema)]
struct DiagnosticLocationResponse {
    path: String,
    line: usize,
    column: usize,
}

#[derive(Clone, Serialize, ToSchema)]
struct DiagnosticResponse {
    severity: DiagnosticSeverityResponse,
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<String>,
    location: DiagnosticLocationResponse,
}

impl From<&Diagnostic> for DiagnosticResponse {
    fn from(diagnostic: &Diagnostic) -> Self {
        Self {
            severity: match diagnostic.severity {
                Severity::Warning => DiagnosticSeverityResponse::Warning,
                Severity::Error => DiagnosticSeverityResponse::Error,
            },
            code: diagnostic.code.to_owned(),
            message: diagnostic.message.clone(),
            field: diagnostic.field.clone(),
            location: DiagnosticLocationResponse {
                path: diagnostic.location.path.to_string_lossy().into_owned(),
                line: diagnostic.location.line,
                column: diagnostic.location.column,
            },
        }
    }
}

#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
struct Hdf5Response(Vec<u8>);

impl IntoResponse for Hdf5Response {
    fn into_response(self) -> Response {
        let mut response = self.0.into_response();
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(HDF5_MEDIA_TYPE));
        response.headers_mut().insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"bellhop-result.h5\""),
        );
        response
    }
}

/// Serializes a result slice as a JSON array of response views, each made from
/// its item as it is written, so a large result is never cloned or collected.
struct Views<'a, T, V>(&'a [T], PhantomData<fn() -> V>);

impl<'a, T, V> Views<'a, T, V> {
    fn new(items: &'a [T]) -> Self {
        Self(items, PhantomData)
    }
}

impl<'a, T, V: From<&'a T> + Serialize> Serialize for Views<'a, T, V> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(self.0.iter().map(V::from))
    }
}

#[derive(Serialize, ToSchema)]
struct ArrivalsResponse<'a> {
    schema_version: u32,
    title: &'a str,
    frequency_hz: f64,
    warnings: Vec<DiagnosticResponse>,
    #[schema(value_type = Vec<ArrivalSourceResponse>)]
    sources: Views<'a, SourceArrivals, ArrivalSourceResponse<'a>>,
}

#[derive(Serialize, ToSchema)]
struct ArrivalSourceResponse<'a> {
    source_depth_m: f64,
    #[schema(value_type = Vec<ArrivalReceiverResponse>)]
    receivers: Views<'a, ReceiverArrivals, ArrivalReceiverResponse<'a>>,
}

impl<'a> From<&'a SourceArrivals> for ArrivalSourceResponse<'a> {
    fn from(source: &'a SourceArrivals) -> Self {
        Self {
            source_depth_m: source.source_depth_m,
            receivers: Views::new(&source.receivers),
        }
    }
}

#[derive(Serialize, ToSchema)]
struct ArrivalReceiverResponse<'a> {
    range_m: f64,
    depth_m: f64,
    #[schema(value_type = Vec<ArrivalResponse>)]
    arrivals: Views<'a, Arrival, ArrivalResponse>,
}

impl<'a> From<&'a ReceiverArrivals> for ArrivalReceiverResponse<'a> {
    fn from(receiver: &'a ReceiverArrivals) -> Self {
        Self {
            range_m: receiver.range_m,
            depth_m: receiver.depth_m,
            arrivals: Views::new(&receiver.arrivals),
        }
    }
}

#[derive(Serialize, ToSchema)]
struct ArrivalResponse {
    amplitude: f32,
    phase_radians: f32,
    travel_time_s: f32,
    attenuation_time_s: f32,
    source_angle_degrees: f32,
    receiver_angle_degrees: f32,
    top_bounces: u32,
    bottom_bounces: u32,
}

impl From<&Arrival> for ArrivalResponse {
    fn from(arrival: &Arrival) -> Self {
        Self {
            amplitude: arrival.amplitude,
            phase_radians: arrival.phase_radians,
            travel_time_s: arrival.travel_time_s,
            attenuation_time_s: arrival.attenuation_time_s,
            source_angle_degrees: arrival.source_angle_degrees,
            receiver_angle_degrees: arrival.receiver_angle_degrees,
            top_bounces: arrival.top_bounces,
            bottom_bounces: arrival.bottom_bounces,
        }
    }
}

#[derive(Serialize, ToSchema)]
struct FieldResponse<'a> {
    schema_version: u32,
    title: &'a str,
    frequency_hz: f64,
    run_kind: FieldRunKindResponse,
    warnings: Vec<DiagnosticResponse>,
    #[schema(value_type = Vec<FieldSourceResponse>)]
    sources: Views<'a, SourceField, FieldSourceResponse<'a>>,
}

#[derive(Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum FieldRunKindResponse {
    Coherent,
    SemiCoherent,
    Incoherent,
}

#[derive(Serialize, ToSchema)]
struct FieldSourceResponse<'a> {
    source_depth_m: f64,
    #[schema(value_type = Vec<FieldReceiverResponse>)]
    receivers: Views<'a, FieldSample, FieldReceiverResponse>,
}

impl<'a> From<&'a SourceField> for FieldSourceResponse<'a> {
    fn from(source: &'a SourceField) -> Self {
        Self {
            source_depth_m: source.source_depth_m,
            receivers: Views::new(&source.samples),
        }
    }
}

#[derive(Serialize, ToSchema)]
struct FieldReceiverResponse {
    range_m: f64,
    depth_m: f64,
    pressure: PressureResponse,
}

impl From<&FieldSample> for FieldReceiverResponse {
    fn from(sample: &FieldSample) -> Self {
        Self {
            range_m: sample.range_m,
            depth_m: sample.depth_m,
            pressure: PressureResponse {
                real: sample.pressure.re,
                imaginary: sample.pressure.im,
            },
        }
    }
}

#[derive(Serialize, ToSchema)]
struct PressureResponse {
    real: f32,
    imaginary: f32,
}

#[derive(Debug, Eq, PartialEq)]
struct NonFiniteOutput {
    field: &'static str,
}

fn validate_arrival_result(result: &SimulationResult) -> Result<(), NonFiniteOutput> {
    finite_f64(result.frequency_hz, "frequency_hz")?;
    for source in &result.arrival_sources {
        finite_f64(source.source_depth_m, "sources.source_depth_m")?;
        for receiver in &source.receivers {
            finite_f64(receiver.range_m, "receivers.range_m")?;
            finite_f64(receiver.depth_m, "receivers.depth_m")?;
            for arrival in &receiver.arrivals {
                finite_f32(arrival.amplitude, "arrivals.amplitude")?;
                finite_f32(arrival.phase_radians, "arrivals.phase_radians")?;
                finite_f32(arrival.travel_time_s, "arrivals.travel_time_s")?;
                finite_f32(arrival.attenuation_time_s, "arrivals.attenuation_time_s")?;
                finite_f32(
                    arrival.source_angle_degrees,
                    "arrivals.source_angle_degrees",
                )?;
                finite_f32(
                    arrival.receiver_angle_degrees,
                    "arrivals.receiver_angle_degrees",
                )?;
            }
        }
    }
    Ok(())
}

fn validate_field_result(result: &SimulationResult) -> Result<(), NonFiniteOutput> {
    finite_f64(result.frequency_hz, "frequency_hz")?;
    for source in &result.field_sources {
        finite_f64(source.source_depth_m, "sources.source_depth_m")?;
        for receiver in &source.samples {
            finite_f64(receiver.range_m, "receivers.range_m")?;
            finite_f64(receiver.depth_m, "receivers.depth_m")?;
            finite_f32(receiver.pressure.re, "receivers.pressure.real")?;
            finite_f32(receiver.pressure.im, "receivers.pressure.imaginary")?;
        }
    }
    Ok(())
}

fn finite_f32(value: f32, field: &'static str) -> Result<(), NonFiniteOutput> {
    value
        .is_finite()
        .then_some(())
        .ok_or(NonFiniteOutput { field })
}

fn finite_f64(value: f64, field: &'static str) -> Result<(), NonFiniteOutput> {
    value
        .is_finite()
        .then_some(())
        .ok_or(NonFiniteOutput { field })
}

struct LimitedJsonBuffer {
    bytes: Vec<u8>,
    limit: usize,
    limit_exceeded: bool,
}

impl LimitedJsonBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(64 * 1024)),
            limit,
            limit_exceeded: false,
        }
    }
}

impl Write for LimitedJsonBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.limit_exceeded = true;
            return Err(io::Error::other("JSON response byte limit exceeded"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
enum JsonWorkerError {
    Simulation(DiagnosticReport),
    NonFinite(NonFiniteOutput),
    ResponseTooLarge,
    Serialization(String),
}

fn serialize_arrivals(
    result: &SimulationResult,
    warnings: &[Diagnostic],
    max_bytes: usize,
) -> Result<Vec<u8>, JsonWorkerError> {
    validate_arrival_result(result).map_err(JsonWorkerError::NonFinite)?;
    serialize_json(
        &ArrivalsResponse {
            schema_version: 1,
            title: &result.title,
            frequency_hz: result.frequency_hz,
            warnings: warnings.iter().map(DiagnosticResponse::from).collect(),
            sources: Views::new(&result.arrival_sources),
        },
        max_bytes,
    )
}

fn serialize_field(
    result: &SimulationResult,
    warnings: &[Diagnostic],
    run_kind: FieldRunKindResponse,
    max_bytes: usize,
) -> Result<Vec<u8>, JsonWorkerError> {
    validate_field_result(result).map_err(JsonWorkerError::NonFinite)?;
    serialize_json(
        &FieldResponse {
            schema_version: 1,
            title: &result.title,
            frequency_hz: result.frequency_hz,
            run_kind,
            warnings: warnings.iter().map(DiagnosticResponse::from).collect(),
            sources: Views::new(&result.field_sources),
        },
        max_bytes,
    )
}

fn serialize_json(response: &impl Serialize, max_bytes: usize) -> Result<Vec<u8>, JsonWorkerError> {
    let mut output = LimitedJsonBuffer::new(max_bytes);
    if let Err(error) = serde_json::to_writer(&mut output, response) {
        return if output.limit_exceeded {
            Err(JsonWorkerError::ResponseTooLarge)
        } else {
            Err(JsonWorkerError::Serialization(error.to_string()))
        };
    }
    Ok(output.bytes)
}

#[derive(Serialize, ToSchema)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Serialize, ToSchema)]
struct ErrorDetail {
    code: String,
    message: String,
    diagnostics: Vec<DiagnosticResponse>,
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    diagnostics: Vec<DiagnosticResponse>,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            diagnostics: Vec::new(),
        }
    }

    fn from_report(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        report: &DiagnosticReport,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            diagnostics: report
                .diagnostics()
                .iter()
                .map(DiagnosticResponse::from)
                .collect(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: ErrorDetail {
                    code: self.code.to_owned(),
                    message: self.message,
                    diagnostics: self.diagnostics,
                },
            }),
        )
            .into_response()
    }
}

#[utoipa::path(
    post,
    path = "/v1/validate",
    tag = "simulation",
    request_body(content = CaseDocument, content_type = "application/json"),
    security((), ("bearer_auth" = [])),
    responses(
        (status = 200, description = "The case is semantically valid", body = ValidationResponse),
        (status = 400, description = "Malformed JSON", body = ErrorResponse),
        (status = 401, description = "Bearer authentication failed", body = ErrorResponse),
        (status = 413, description = "Request body exceeds the configured limit", body = ErrorResponse),
        (status = 415, description = "Request is not JSON", body = ErrorResponse),
        (status = 422, description = "Semantically invalid case", body = ErrorResponse),
        (status = 504, description = "Request timed out", body = ErrorResponse)
    )
)]
async fn validate_case(case: CaseRequest) -> Result<Json<ValidationResponse>, ApiError> {
    Ok(Json(ValidationResponse {
        valid: true,
        warnings: case
            .outcome
            .warnings
            .iter()
            .map(DiagnosticResponse::from)
            .collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/v1/run",
    tag = "simulation",
    request_body(content = CaseDocument, content_type = "application/json"),
    security((), ("bearer_auth" = [])),
    responses(
        (status = 200, description = "Versioned HDF5 simulation result", body = Hdf5Response, content_type = "application/x-hdf5"),
        (status = 400, description = "Malformed JSON", body = ErrorResponse),
        (status = 401, description = "Bearer authentication failed", body = ErrorResponse),
        (status = 413, description = "Request body exceeds the configured limit", body = ErrorResponse),
        (status = 415, description = "Request is not JSON", body = ErrorResponse),
        (status = 422, description = "Case or run option is unsupported", body = ErrorResponse),
        (status = 429, description = "Simulation resource limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal execution or HDF5 error", body = ErrorResponse),
        (status = 504, description = "Request timed out", body = ErrorResponse)
    )
)]
async fn run_case(
    State(state): State<AppState>,
    CaseRequest {
        bytes: request_bytes,
        outcome,
    }: CaseRequest,
) -> Result<Hdf5Response, ApiError> {
    let limits = state.simulation_limits;
    let bytes = match blocking_worker(&state, move || {
        simulate_to_hdf5(&outcome.value, &request_bytes, &outcome.warnings, limits)
    })
    .await?
    {
        Ok(bytes) => bytes,
        Err(WorkerError::Simulation(report)) => return Err(simulation_error(&report)),
        Err(WorkerError::Output(message)) => {
            tracing::error!(%message, "failed to build HDF5 response");
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "output_failed",
                "unable to create the HDF5 result",
            ));
        }
    };
    Ok(Hdf5Response(bytes))
}

#[utoipa::path(
    post,
    path = "/v1/arrivals",
    tag = "simulation",
    request_body(content = CaseDocument, content_type = "application/json"),
    security((), ("bearer_auth" = [])),
    responses(
        (status = 200, description = "Versioned JSON arrival result", body = ArrivalsResponse, content_type = "application/json"),
        (status = 400, description = "Malformed JSON", body = ErrorResponse),
        (status = 401, description = "Bearer authentication failed", body = ErrorResponse),
        (status = 413, description = "Request body exceeds the configured limit", body = ErrorResponse),
        (status = 415, description = "Request is not JSON", body = ErrorResponse),
        (status = 422, description = "Case is invalid, unsupported, or not an arrival run", body = ErrorResponse),
        (status = 429, description = "Simulation or JSON response resource limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal execution or JSON output error", body = ErrorResponse),
        (status = 504, description = "Request timed out", body = ErrorResponse)
    )
)]
async fn arrivals_case(
    State(state): State<AppState>,
    CaseRequest { outcome, .. }: CaseRequest,
) -> Result<Response, ApiError> {
    if outcome.value.environment.run.kind != RunKind::Arrivals {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported_run_kind",
            "the /v1/arrivals endpoint requires run.kind to be arrivals",
        ));
    }
    simulate_to_json(state, outcome, JsonOutput::Arrivals).await
}

#[utoipa::path(
    post,
    path = "/v1/field",
    tag = "simulation",
    request_body(content = CaseDocument, content_type = "application/json"),
    security((), ("bearer_auth" = [])),
    responses(
        (status = 200, description = "Versioned JSON pressure-field result", body = FieldResponse, content_type = "application/json"),
        (status = 400, description = "Malformed JSON", body = ErrorResponse),
        (status = 401, description = "Bearer authentication failed", body = ErrorResponse),
        (status = 413, description = "Request body exceeds the configured limit", body = ErrorResponse),
        (status = 415, description = "Request is not JSON", body = ErrorResponse),
        (status = 422, description = "Case is invalid, unsupported, or not a pressure-field run", body = ErrorResponse),
        (status = 429, description = "Simulation or JSON response resource limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal execution or JSON output error", body = ErrorResponse),
        (status = 504, description = "Request timed out", body = ErrorResponse)
    )
)]
async fn field_case(
    State(state): State<AppState>,
    CaseRequest { outcome, .. }: CaseRequest,
) -> Result<Response, ApiError> {
    let run_kind = match outcome.value.environment.run.kind {
        RunKind::Coherent => FieldRunKindResponse::Coherent,
        RunKind::SemiCoherent => FieldRunKindResponse::SemiCoherent,
        RunKind::Incoherent => FieldRunKindResponse::Incoherent,
        _ => {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "unsupported_run_kind",
                "the /v1/field endpoint requires a coherent, semi_coherent, or incoherent run.kind",
            ));
        }
    };
    simulate_to_json(state, outcome, JsonOutput::Field(run_kind)).await
}

#[derive(Clone, Copy)]
enum JsonOutput {
    Arrivals,
    Field(FieldRunKindResponse),
}

async fn simulate_to_json(
    state: AppState,
    outcome: LoadOutcome<Case>,
    output: JsonOutput,
) -> Result<Response, ApiError> {
    let limits = state.simulation_limits;
    let max_json_response_bytes = state.max_json_response_bytes;
    let warnings = outcome.warnings;
    let bytes = match blocking_worker(&state, move || {
        let result = run_simulation(&outcome.value, limits).map_err(JsonWorkerError::Simulation)?;
        match output {
            JsonOutput::Arrivals => serialize_arrivals(&result, &warnings, max_json_response_bytes),
            JsonOutput::Field(run_kind) => {
                serialize_field(&result, &warnings, run_kind, max_json_response_bytes)
            }
        }
    })
    .await?
    {
        Ok(bytes) => bytes,
        Err(JsonWorkerError::Simulation(report)) => return Err(simulation_error(&report)),
        Err(JsonWorkerError::NonFinite(error)) => {
            tracing::error!(
                field = error.field,
                "simulation produced non-finite JSON output"
            );
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "output_failed",
                "simulation produced a non-finite JSON output value",
            ));
        }
        Err(JsonWorkerError::ResponseTooLarge) => {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "resource_limit_exceeded",
                "JSON response exceeded the configured byte limit",
            ));
        }
        Err(JsonWorkerError::Serialization(message)) => {
            tracing::error!(%message, "failed to serialize JSON simulation response");
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "output_failed",
                "unable to serialize the JSON simulation result",
            ));
        }
    };

    let mut response = bytes.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(JSON_MEDIA_TYPE));
    Ok(response)
}

async fn blocking_worker<T: Send + 'static>(
    state: &AppState,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    let permit = state
        .worker_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|error| {
            tracing::error!(%error, "simulation worker semaphore closed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "worker_pool_failed",
                "simulation worker pool failed",
            )
        })?;
    let threads = state.solver_threads;
    // Dropping the request (including timeout) does not cancel blocking work.
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        bellhop::parallelism::with_threads(threads, work)
    })
    .await
    .map_err(|error| {
        tracing::error!(%error, "blocking simulation task failed");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "execution_failed",
            "simulation worker failed",
        )
    })
}

fn simulation_error(report: &DiagnosticReport) -> ApiError {
    let (status, code, message) = if report.diagnostics().iter().any(|d| d.code == "BH0303") {
        (
            StatusCode::TOO_MANY_REQUESTS,
            "resource_limit_exceeded",
            "simulation exceeded a server resource limit",
        )
    } else {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "simulation_failed",
            "the case could not be simulated",
        )
    };
    ApiError::from_report(status, code, message, report)
}

/// A request body accepted as one validated case, with the bytes it was parsed from.
struct CaseRequest {
    bytes: Bytes,
    outcome: LoadOutcome<Case>,
}

impl<S: Send + Sync> FromRequest<S> for CaseRequest {
    type Rejection = ApiError;

    /// Reads the whole body, then checks its media type, size and case in that
    /// order. The order decides which error a request gets: a stalled body times
    /// out whatever its media type, and a body that is not JSON is refused for
    /// that before its size.
    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        let media_type = request.headers().get(CONTENT_TYPE).cloned();
        let body = Bytes::from_request(request, state).await;
        let media_type = media_type
            .as_ref()
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        if !matches!(media_type, Some("application/json"))
            && !media_type.is_some_and(|value| value.ends_with("+json"))
        {
            return Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "Content-Type must be application/json",
            ));
        }
        let bytes = body.map_err(|rejection| {
            ApiError::new(
                rejection.status(),
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    "body_too_large"
                } else {
                    "body_read_failed"
                },
                rejection.body_text(),
            )
        })?;
        let outcome = bellhop::json::load_case_document(&bytes).map_err(|error| {
            let (status, code, message) = match error.kind() {
                DocumentErrorKind::Malformed => (
                    StatusCode::BAD_REQUEST,
                    "malformed_json",
                    "request body is not a valid JSON case",
                ),
                DocumentErrorKind::Semantic => (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_case",
                    "request body describes an invalid case",
                ),
            };
            ApiError::from_report(status, code, message, error.report())
        })?;
        Ok(Self { bytes, outcome })
    }
}

enum WorkerError {
    Simulation(DiagnosticReport),
    Output(String),
}

fn simulate_to_hdf5(
    case: &Case,
    request_bytes: &[u8],
    warnings: &[Diagnostic],
    limits: SimulationLimits,
) -> Result<Vec<u8>, WorkerError> {
    let result = run_simulation(case, limits).map_err(WorkerError::Simulation)?;
    let directory = tempfile::tempdir()
        .map_err(|error| WorkerError::Output(format!("temporary directory failed: {error}")))?;
    let path = directory.path().join("result.h5");
    output::bellhop::write_hdf5(&path, "request.json", request_bytes, &result, warnings)
        .map_err(WorkerError::Output)?;
    std::fs::read(path)
        .map_err(|error| WorkerError::Output(format!("unable to read HDF5 result: {error}")))
}

async fn require_auth(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let Some(expected) = state.auth_token.as_deref() else {
        return next.run(request).await;
    };
    let valid = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected);
    if valid {
        next.run(request).await
    } else {
        let mut response = ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "a valid bearer token is required",
        )
        .into_response();
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        response
    }
}

async fn handle_middleware_error(error: BoxError) -> ApiError {
    if error.is::<tower::timeout::error::Elapsed>() {
        ApiError::new(
            StatusCode::GATEWAY_TIMEOUT,
            "request_timeout",
            "the request exceeded the server timeout",
        )
    } else {
        tracing::error!(%error, "HTTP middleware failed");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "middleware_failed",
            "request processing failed",
        )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use axum::response::IntoResponse;
    use bellhop::solver::{Arrival, ReceiverArrivals, SimulationResult, SourceArrivals};
    use serde_json::Value;
    use tower::ServiceExt;

    use super::{HDF5_MEDIA_TYPE, ServerConfig, app};

    const CASE: &str = include_str!("../../../examples/cases/bellhop/field-g/field-g.json");
    const VALIDATE: &str = "/v1/validate";
    const RUN: &str = "/v1/run";
    const ARRIVALS: &str = "/v1/arrivals";
    const FIELD: &str = "/v1/field";

    fn json_request(path: &str, body: impl Into<Body>) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.into())
            .unwrap()
    }

    #[tokio::test]
    async fn dropped_requests_keep_worker_slots_until_blocking_work_finishes() {
        let state = super::AppState {
            simulation_limits: super::SimulationLimits::default(),
            solver_threads: std::num::NonZeroUsize::MIN,
            max_json_response_bytes: 1024,
            auth_token: None,
            worker_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
        };
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let request_state = state.clone();
        let request = tokio::spawn(async move {
            super::blocking_worker(&request_state, move || {
                started.send(()).unwrap();
                wait.recv().unwrap();
            })
            .await
            .is_ok()
        });
        ready.await.unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        assert!(state.worker_slots.clone().try_acquire_owned().is_err());
        release.send(()).unwrap();
        let permit = state.worker_slots.clone().acquire_owned().await.unwrap();
        drop(permit);
        assert_eq!(super::blocking_worker(&state, || 7).await.ok(), Some(7));
        let error = super::blocking_worker(&state, || panic!("worker failure"))
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "execution_failed");
        assert_eq!(state.worker_slots.available_permits(), 1);
        state.worker_slots.close();
        let error = super::blocking_worker(&state, || ()).await.err().unwrap();
        assert_eq!(error.code, "worker_pool_failed");
    }

    /// Checks `value` against the subset of JSON Schema that utoipa emits.
    fn conforms(value: &Value, schema: &Value, root: &Value) -> bool {
        let object = match schema {
            Value::Bool(accepts) => return *accepts,
            Value::Object(object) => object,
            other => panic!("unsupported schema {other}"),
        };
        // A keyword this check does not apply would let any value pass. The
        // last three only annotate, as `format` does by default in JSON Schema.
        let keywords = "$ref oneOf anyOf allOf enum type minimum maximum properties \
                        required items description example format";
        for (keyword, argument) in object {
            assert!(
                keywords.split(' ').any(|known| known == keyword)
                    || (keyword == "additionalProperties" && argument.is_boolean()),
                "unsupported schema keyword {keyword}"
            );
        }
        if let Some(reference) = schema["$ref"].as_str() {
            let name = reference.strip_prefix("#/components/schemas/").unwrap();
            let target = &root["components"]["schemas"][name];
            assert!(target.is_object(), "unregistered component {name}");
            if !conforms(value, target, root) {
                return false;
            }
        }
        if let Some(options) = schema["oneOf"].as_array()
            && options.iter().filter(|o| conforms(value, o, root)).count() != 1
        {
            return false;
        }
        if let Some(options) = schema["anyOf"].as_array()
            && !options.iter().any(|o| conforms(value, o, root))
        {
            return false;
        }
        if let Some(options) = schema["allOf"].as_array()
            && !options.iter().all(|o| conforms(value, o, root))
        {
            return false;
        }
        if let Some(allowed) = schema["enum"].as_array()
            && !allowed.contains(value)
        {
            return false;
        }
        let types: Vec<&str> = match &schema["type"] {
            Value::String(name) => vec![name],
            Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !types.is_empty()
            && !types.iter().any(|name| match *name {
                "null" => value.is_null(),
                "boolean" => value.is_boolean(),
                "string" => value.is_string(),
                "number" => value.is_number(),
                "integer" => value.is_i64() || value.is_u64(),
                "array" => value.is_array(),
                "object" => value.is_object(),
                other => panic!("unexpected schema type {other}"),
            })
        {
            return false;
        }
        if let Some(number) = value.as_f64()
            && (schema["minimum"]
                .as_f64()
                .is_some_and(|minimum| number < minimum)
                || schema["maximum"]
                    .as_f64()
                    .is_some_and(|maximum| number > maximum))
        {
            return false;
        }
        if let Value::Object(object) = value {
            let properties = schema["properties"].as_object();
            let required = schema["required"].as_array().into_iter().flatten();
            if required
                .filter_map(Value::as_str)
                .any(|key| !object.contains_key(key))
            {
                return false;
            }
            for (key, item) in object {
                match properties.and_then(|properties| properties.get(key)) {
                    Some(property) if !conforms(item, property, root) => return false,
                    None if schema["additionalProperties"] == Value::Bool(false) => return false,
                    _ => {}
                }
            }
        }
        if let (Value::Array(items), Some(item)) = (value, schema.get("items")) {
            return items.iter().all(|value| conforms(value, item, root));
        }
        true
    }

    #[tokio::test]
    async fn published_schema_accepts_every_example_case() {
        let openapi = app(ServerConfig::default())
            .oneshot(Request::get("/openapi.json").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let openapi: Value =
            serde_json::from_slice(&to_bytes(openapi.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let schema = &openapi["components"]["schemas"]["CaseDocument"];
        let cases =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cases/bellhop");
        let mut count = 0;
        for entry in std::fs::read_dir(cases).unwrap() {
            let folder = entry.unwrap().path();
            let name = folder.file_name().unwrap().to_string_lossy().into_owned();
            let path = folder.join(format!("{name}.json"));
            let case: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert!(conforms(&case, schema, &openapi), "{}", path.display());
            count += 1;
        }
        assert!(count >= 12);
        // Exported shorthand names are in the schema; other names are not.
        let mut case: Value = serde_json::from_str(CASE).unwrap();
        case["attenuation"] = serde_json::json!({"volume": "thorp"});
        assert!(conforms(&case, schema, &openapi));
        case["surface"] = serde_json::json!("sand");
        assert!(!conforms(&case, schema, &openapi));
    }

    /// The published document, with every object declared to have no unlisted
    /// properties: a response field the document leaves out is unpublished,
    /// although JSON Schema would allow it.
    async fn published_responses() -> Value {
        fn close_objects(schema: &mut Value) {
            let Value::Object(object) = schema else {
                return;
            };
            if let Some(Value::Object(properties)) = object.get_mut("properties") {
                properties.values_mut().for_each(close_objects);
                object
                    .entry("additionalProperties")
                    .or_insert(Value::Bool(false));
            }
            if let Some(item) = object.get_mut("items") {
                close_objects(item);
            }
            for keyword in ["oneOf", "anyOf", "allOf"] {
                if let Some(Value::Array(options)) = object.get_mut(keyword) {
                    options.iter_mut().for_each(close_objects);
                }
            }
        }

        let openapi = app(ServerConfig::default())
            .oneshot(Request::get("/openapi.json").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let mut openapi: Value =
            serde_json::from_slice(&to_bytes(openapi.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        for schema in openapi["components"]["schemas"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            close_objects(schema);
        }
        openapi
    }

    /// Checks `response` against what `openapi` publishes for `method path` and its
    /// status: a listed status, the one listed media type and, for JSON, the schema.
    async fn assert_published(
        openapi: &Value,
        method: &str,
        path: &str,
        response: axum::response::Response,
    ) -> (StatusCode, Value) {
        let status = response.status();
        let published = &openapi["paths"][path][method]["responses"][status.as_str()];
        assert!(published.is_object(), "{method} {path} returned {status}");
        let media_type = response.headers()[header::CONTENT_TYPE].to_str().unwrap();
        let content = published["content"].as_object().unwrap();
        assert_eq!(
            content.keys().collect::<Vec<_>>(),
            [media_type],
            "{method} {path} {status}"
        );
        let schema = content[media_type]["schema"].clone();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        if schema["$ref"] == "#/components/schemas/Hdf5Response" {
            assert_eq!(&body[..8], b"\x89HDF\r\n\x1a\n");
            return (status, Value::Null);
        }
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(
            conforms(&body, &schema, openapi),
            "{method} {path} {status}"
        );
        (status, body)
    }

    async fn published_body(openapi: &Value, path: &str, body: impl Into<Body>) -> Value {
        let response = app(ServerConfig::default())
            .oneshot(json_request(path, body))
            .await
            .unwrap();
        assert_published(openapi, "post", path, response).await.1
    }

    fn with_run_kind(case: &[u8], run_kind: &str) -> Vec<u8> {
        let mut case: Value = serde_json::from_slice(case).unwrap();
        case["bellhop"]["run"] = serde_json::json!(run_kind);
        serde_json::to_vec(&case).unwrap()
    }

    fn without_angles() -> Vec<u8> {
        let mut case: Value = serde_json::from_str(CASE).unwrap();
        case["bellhop"]["angles"] = serde_json::json!([]);
        serde_json::to_vec(&case).unwrap()
    }

    #[tokio::test]
    async fn every_published_response_conforms_to_its_schema() {
        // No request here reaches these; they are checked as constructed `ApiError`s.
        const CONSTRUCTED: [(&str, u16); 2] = [(RUN, 500), (FIELD, 500)];
        let openapi = published_responses().await;
        let mut seen = std::collections::BTreeSet::new();
        let mut check = async |service: &axum::Router, path: &str, request: Request<Body>| {
            let method = request.method().as_str().to_ascii_lowercase();
            let response = service.clone().oneshot(request).await.unwrap();
            let (status, _) = assert_published(&openapi, &method, path, response).await;
            seen.insert((path.to_owned(), status.as_u16()));
            status
        };
        let service = app(ServerConfig::default());
        let health = Request::get("/healthz").body(Body::empty()).unwrap();
        check(&service, "/healthz", health).await;

        // Every example as written, the JSON result kinds that they lack, and an
        // arrivals run whose non-finite amplitude is the one real 500.
        let examples =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cases/bellhop");
        let mut cases = Vec::new();
        for entry in std::fs::read_dir(&examples).unwrap() {
            let folder = entry.unwrap().path();
            let name = folder.file_name().unwrap().to_string_lossy().into_owned();
            cases.push(std::fs::read(folder.join(format!("{name}.json"))).unwrap());
        }
        let beam_pattern = std::fs::read(examples.join("beam-pattern/beam-pattern.json")).unwrap();
        cases.push(with_run_kind(&beam_pattern, "arrivals"));
        cases.push(with_run_kind(CASE.as_bytes(), "semi_coherent"));
        cases.push(with_run_kind(CASE.as_bytes(), "incoherent"));
        let arrivals = with_run_kind(CASE.as_bytes(), "arrivals");
        let invalid = without_angles();

        let configured = |change: fn(&mut ServerConfig)| {
            let mut config = ServerConfig::default();
            change(&mut config);
            app(config)
        };
        let auth = configured(|config| config.auth_token = Some("secret".to_owned()));
        let small_body = configured(|config| config.max_body_bytes = 8);
        let small_response = configured(|config| config.max_json_response_bytes = 8);
        let few_rays = configured(|config| config.simulation_limits.max_rays = 1);
        let no_time = configured(|config| config.request_timeout = Duration::ZERO);
        for route in [VALIDATE, RUN, ARRIVALS, FIELD] {
            let own_kind = if route == ARRIVALS {
                arrivals.as_slice()
            } else {
                CASE.as_bytes()
            };
            let mut requests: Vec<(&axum::Router, &[u8])> = cases
                .iter()
                .map(|case| (&service, case.as_slice()))
                .collect();
            requests.extend([
                (&service, b"{".as_slice()),
                (&service, &invalid),
                (&auth, CASE.as_bytes()),
                (&small_body, CASE.as_bytes()),
                (&small_response, own_kind),
                (&few_rays, own_kind),
            ]);
            for (service, body) in requests {
                check(service, route, json_request(route, body.to_vec())).await;
            }
            let untyped = Request::post(route).body(Body::from(CASE)).unwrap();
            check(&service, route, untyped).await;
            // A ready response can beat a zero timeout; keep body extraction pending instead.
            let pending = Request::post(route)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from_stream(futures_util::stream::pending::<
                    Result<axum::body::Bytes, std::io::Error>,
                >()))
                .unwrap();
            check(&no_time, route, pending).await;
        }
        let non_finite = json_request(ARRIVALS, with_run_kind(&beam_pattern, "arrivals"));
        assert_eq!(
            check(&service, ARRIVALS, non_finite).await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "beam-pattern as arrivals no longer has a non-finite amplitude; \
             find another real 500 for /v1/arrivals"
        );
        for (route, status) in CONSTRUCTED {
            let error = super::ApiError::new(
                StatusCode::from_u16(status).unwrap(),
                "output_failed",
                "unable to create the result",
            );
            assert_published(&openapi, "post", route, error.into_response()).await;
        }

        let mut published = std::collections::BTreeSet::new();
        for (path, item) in openapi["paths"].as_object().unwrap() {
            for operation in item.as_object().unwrap().values() {
                for status in operation["responses"].as_object().unwrap().keys() {
                    published.insert((path.clone(), status.parse::<u16>().unwrap()));
                }
            }
        }
        seen.extend(CONSTRUCTED.map(|(route, status)| (route.to_owned(), status)));
        assert_eq!(seen, published);
    }

    #[tokio::test]
    async fn response_conformance_rejects_unpublished_bodies() {
        let openapi = published_responses().await;
        let schema = |name: &str| openapi["components"]["schemas"][name].clone();
        let arrivals_case = with_run_kind(CASE.as_bytes(), "arrivals");
        let arrivals = published_body(&openapi, "/v1/arrivals", arrivals_case).await;
        let receiver = arrivals["sources"][0]["receivers"]
            .as_array()
            .unwrap()
            .iter()
            .position(|receiver| !receiver["arrivals"].as_array().unwrap().is_empty())
            .unwrap();
        let unpublished: [fn(&mut Value); 6] = [
            |arrival| {
                arrival.as_object_mut().unwrap().remove("travel_time_s");
            },
            |arrival| {
                let time = arrival.as_object_mut().unwrap().remove("travel_time_s");
                arrival["travel_time"] = time.unwrap();
            },
            |arrival| arrival["unpublished"] = serde_json::json!(0),
            |arrival| arrival["top_bounces"] = serde_json::json!(-1),
            |arrival| arrival["top_bounces"] = serde_json::json!(0.5),
            |arrival| arrival["amplitude"] = serde_json::json!("1"),
        ];
        for change in unpublished {
            let mut body = arrivals.clone();
            change(&mut body["sources"][0]["receivers"][receiver]["arrivals"][0]);
            assert!(!conforms(&body, &schema("ArrivalsResponse"), &openapi));
        }

        let mut field = published_body(&openapi, "/v1/field", CASE).await;
        field["run_kind"] = serde_json::json!("arrivals");
        assert!(!conforms(&field, &schema("FieldResponse"), &openapi));
        let mut validation = published_body(&openapi, "/v1/validate", CASE).await;
        validation["warnings"][0]["severity"] = serde_json::json!("note");
        assert!(!conforms(
            &validation,
            &schema("ValidationResponse"),
            &openapi
        ));
        let mut invalid = published_body(&openapi, "/v1/validate", without_angles()).await;
        invalid["error"]["diagnostics"][0]
            .as_object_mut()
            .unwrap()
            .remove("location");
        assert!(!conforms(&invalid, &schema("ErrorResponse"), &openapi));
        assert!(conforms(&invalid, &Value::Bool(true), &openapi));
        assert!(!conforms(&invalid, &Value::Bool(false), &openapi));
    }

    /// Wire names are Rust identifiers, so a rename changes bytes and schema
    /// together; this table holds them. A component lists its enum values or its
    /// required properties in order, then optional ones marked `?`.
    #[tokio::test]
    async fn response_wire_names_are_unchanged() {
        const SURFACE: &str = "\
HealthResponse: status
ValidationResponse: valid warnings
DiagnosticResponse: severity code message location field?
DiagnosticLocationResponse: path line column
DiagnosticSeverityResponse: warning error
ErrorResponse: error
ErrorDetail: code message diagnostics
ArrivalsResponse: schema_version title frequency_hz warnings sources
ArrivalSourceResponse: source_depth_m receivers
ArrivalReceiverResponse: range_m depth_m arrivals
ArrivalResponse: amplitude phase_radians travel_time_s attenuation_time_s source_angle_degrees receiver_angle_degrees top_bounces bottom_bounces
FieldResponse: schema_version title frequency_hz run_kind warnings sources
FieldRunKindResponse: coherent semi_coherent incoherent
FieldSourceResponse: source_depth_m receivers
FieldReceiverResponse: range_m depth_m pressure
PressureResponse: real imaginary
/healthz application/json: 200
/v1/arrivals application/json: 200 400 401 413 415 422 429 500 504
/v1/field application/json: 200 400 401 413 415 422 429 500 504
/v1/run application/json: 400 401 413 415 422 429 500 504
/v1/run application/x-hdf5: 200
/v1/validate application/json: 200 400 401 413 415 422 504";
        let openapi = published_responses().await;
        let words = |list: &Value| -> Vec<String> {
            let list = list.as_array().into_iter().flatten();
            list.map(|word| word.as_str().unwrap().to_owned()).collect()
        };
        let mut surface = Vec::new();
        for line in SURFACE.lines().filter(|line| !line.starts_with('/')) {
            let name = line.split_once(": ").unwrap().0;
            let schema = &openapi["components"]["schemas"][name];
            let mut names = [words(&schema["enum"]), words(&schema["required"])].concat();
            for key in schema["properties"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|p| p.0)
            {
                if !names.contains(key) {
                    names.push(format!("{key}?"));
                }
            }
            surface.push(format!("{name}: {}", names.join(" ")));
        }
        let mut statuses = std::collections::BTreeMap::<String, Vec<&str>>::new();
        for (path, item) in openapi["paths"].as_object().unwrap() {
            for operation in item.as_object().unwrap().values() {
                for (status, response) in operation["responses"].as_object().unwrap() {
                    for media_type in response["content"].as_object().unwrap().keys() {
                        let route = format!("{path} {media_type}");
                        statuses.entry(route).or_default().push(status);
                    }
                }
            }
        }
        surface.extend(
            statuses
                .iter()
                .map(|(route, list)| format!("{route}: {}", list.join(" "))),
        );
        assert_eq!(surface.join("\n"), SURFACE);
    }

    #[tokio::test]
    async fn public_routes_and_validation_contract() {
        let service = app(ServerConfig::default());
        let health = service
            .clone()
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);

        let openapi = service
            .clone()
            .oneshot(Request::get("/openapi.json").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(openapi.status(), StatusCode::OK);
        let openapi: Value =
            serde_json::from_slice(&to_bytes(openapi.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert!(openapi["paths"]["/v1/run"].is_object());
        assert!(openapi["paths"]["/v1/arrivals"].is_object());
        assert!(openapi["paths"]["/v1/field"].is_object());
        assert_eq!(
            openapi["components"]["schemas"]["Hdf5Response"]["format"],
            "binary"
        );
        assert!(openapi["components"]["schemas"]["ArrivalsResponse"].is_object());
        assert!(openapi["components"]["schemas"]["FieldResponse"].is_object());

        let valid = service
            .clone()
            .oneshot(json_request("/v1/validate", CASE))
            .await
            .unwrap();
        assert_eq!(valid.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(valid.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["valid"], true);
        assert_eq!(body["warnings"].as_array().unwrap().len(), 1);

        let malformed = service
            .clone()
            .oneshot(json_request("/v1/validate", "{"))
            .await
            .unwrap();
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

        let mut semantic: Value = serde_json::from_str(CASE).unwrap();
        semantic["bellhop"]["angles"] = serde_json::json!([]);
        let semantic = service
            .oneshot(json_request(
                "/v1/validate",
                serde_json::to_vec(&semantic).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(semantic.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn run_returns_hdf5_directly() {
        let response = app(ServerConfig::default())
            .oneshot(json_request("/v1/run", CASE))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], HDF5_MEDIA_TYPE);
        assert_eq!(
            response.headers()[header::CONTENT_DISPOSITION],
            "attachment; filename=\"bellhop-result.h5\""
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..8], b"\x89HDF\r\n\x1a\n");
    }

    #[tokio::test]
    async fn arrivals_returns_versioned_json_and_rejects_other_run_kinds() {
        let mut arrivals_case: Value = serde_json::from_str(CASE).unwrap();
        arrivals_case["bellhop"]["run"] = serde_json::json!("arrivals");
        let response = app(ServerConfig::default())
            .oneshot(json_request(
                "/v1/arrivals",
                serde_json::to_vec(&arrivals_case).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["schema_version"], 1);
        assert_eq!(body["frequency_hz"], 1000.0);
        assert!(body["warnings"].is_array());
        assert_eq!(body["sources"].as_array().unwrap().len(), 1);
        assert_eq!(
            body["sources"][0]["receivers"].as_array().unwrap().len(),
            33
        );
        assert!(
            body["sources"][0]["receivers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|receiver| !receiver["arrivals"].as_array().unwrap().is_empty())
        );

        let rejected = app(ServerConfig::default())
            .oneshot(json_request("/v1/arrivals", CASE))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body: Value =
            serde_json::from_slice(&to_bytes(rejected.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["error"]["code"], "unsupported_run_kind");

        let output_limited = app(ServerConfig {
            max_json_response_bytes: 8,
            ..ServerConfig::default()
        })
        .oneshot(json_request(
            "/v1/arrivals",
            serde_json::to_vec(&arrivals_case).unwrap(),
        ))
        .await
        .unwrap();
        assert_eq!(output_limited.status(), StatusCode::TOO_MANY_REQUESTS);
        let body: Value = serde_json::from_slice(
            &to_bytes(output_limited.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["error"]["code"], "resource_limit_exceeded");

        let limits = bellhop::solver::SimulationLimits {
            max_arrivals_per_receiver: 1,
            max_total_arrivals: 1,
            ..bellhop::solver::SimulationLimits::default()
        };
        let limited = app(ServerConfig {
            simulation_limits: limits,
            ..ServerConfig::default()
        })
        .oneshot(json_request(
            "/v1/arrivals",
            serde_json::to_vec(&arrivals_case).unwrap(),
        ))
        .await
        .unwrap();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        let body: Value =
            serde_json::from_slice(&to_bytes(limited.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["error"]["code"], "resource_limit_exceeded");
    }

    #[tokio::test]
    async fn solver_threads_do_not_change_responses() {
        let mut responses = Vec::new();
        for threads in [1, 3] {
            let response = app(ServerConfig {
                solver_threads: std::num::NonZeroUsize::new(threads).unwrap(),
                ..ServerConfig::default()
            })
            .oneshot(json_request("/v1/field", CASE))
            .await
            .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            responses.push(to_bytes(response.into_body(), usize::MAX).await.unwrap());
        }
        assert_eq!(responses[0], responses[1]);
    }

    #[tokio::test]
    async fn field_returns_all_pressure_modes_and_rejects_other_run_kinds() {
        for run_kind in ["coherent", "semi_coherent", "incoherent"] {
            let mut field_case: Value = serde_json::from_str(CASE).unwrap();
            field_case["bellhop"]["run"] = serde_json::json!(run_kind);
            let response = app(ServerConfig::default())
                .oneshot(json_request(
                    "/v1/field",
                    serde_json::to_vec(&field_case).unwrap(),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                    .unwrap();
            assert_eq!(body["schema_version"], 1);
            assert_eq!(body["run_kind"], run_kind);
            assert_eq!(body["frequency_hz"], 1000.0);
            assert!(body["warnings"].is_array());
            assert_eq!(body["sources"].as_array().unwrap().len(), 1);
            assert_eq!(
                body["sources"][0]["receivers"].as_array().unwrap().len(),
                33
            );
            let receiver = &body["sources"][0]["receivers"][0];
            assert!(receiver["range_m"].is_number());
            assert!(receiver["depth_m"].is_number());
            assert!(receiver["pressure"]["real"].is_number());
            assert!(receiver["pressure"]["imaginary"].is_number());
        }

        let mut arrivals_case: Value = serde_json::from_str(CASE).unwrap();
        arrivals_case["bellhop"]["run"] = serde_json::json!("arrivals");
        let rejected = app(ServerConfig::default())
            .oneshot(json_request(
                "/v1/field",
                serde_json::to_vec(&arrivals_case).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body: Value =
            serde_json::from_slice(&to_bytes(rejected.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["error"]["code"], "unsupported_run_kind");

        let output_limited = app(ServerConfig {
            max_json_response_bytes: 8,
            ..ServerConfig::default()
        })
        .oneshot(json_request("/v1/field", CASE))
        .await
        .unwrap();
        assert_eq!(output_limited.status(), StatusCode::TOO_MANY_REQUESTS);
        let body: Value = serde_json::from_slice(
            &to_bytes(output_limited.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["error"]["code"], "resource_limit_exceeded");
    }

    #[test]
    fn non_finite_arrivals_must_not_serialize_as_null() {
        let result = SimulationResult {
            title: "non-finite arrival".to_owned(),
            frequency_hz: 1_000.0,
            legacy_run_options: String::new(),
            sources: Vec::new(),
            arrival_sources: vec![SourceArrivals {
                source_depth_m: 10.0,
                receivers: vec![ReceiverArrivals {
                    range_m: 1.0,
                    depth_m: 10.0,
                    arrivals: vec![Arrival {
                        amplitude: f32::INFINITY,
                        phase_radians: 0.0,
                        travel_time_s: 0.1,
                        attenuation_time_s: 0.0,
                        source_angle_degrees: 0.0,
                        receiver_angle_degrees: 0.0,
                        top_bounces: 0,
                        bottom_bounces: 0,
                    }],
                }],
            }],
            eigenray_sources: Vec::new(),
            field_sources: Vec::new(),
        };

        let validation = super::validate_arrival_result(&result);
        assert!(matches!(
            validation,
            Err(super::NonFiniteOutput {
                field: "arrivals.amplitude"
            })
        ));
    }

    #[test]
    fn non_finite_pressure_must_not_serialize_as_null() {
        let case = bellhop::json::load_case_document(CASE.as_bytes())
            .unwrap()
            .value;
        let mut result =
            bellhop::solver::run(&case, bellhop::solver::SimulationLimits::default()).unwrap();
        result.field_sources[0].samples[0].pressure.re = f32::INFINITY;

        let validation = super::validate_field_result(&result);
        assert!(matches!(
            validation,
            Err(super::NonFiniteOutput {
                field: "receivers.pressure.real"
            })
        ));
    }

    #[tokio::test]
    async fn transport_limits_use_the_documented_statuses() {
        let too_large = app(ServerConfig {
            max_body_bytes: 8,
            ..ServerConfig::default()
        })
        .oneshot(json_request("/v1/validate", CASE))
        .await
        .unwrap();
        assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            too_large.headers()[header::CONTENT_TYPE],
            "application/json"
        );

        let wrong_media_type = app(ServerConfig::default())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/validate")
                    .body(Body::from(CASE))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            wrong_media_type.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );

        let timed_out = app(ServerConfig {
            request_timeout: Duration::ZERO,
            ..ServerConfig::default()
        })
        // A ready response can beat a zero timeout; keep body extraction pending instead.
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/run")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from_stream(futures_util::stream::pending::<
                    Result<axum::body::Bytes, std::io::Error>,
                >()))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(timed_out.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(
            timed_out.headers()[header::CONTENT_TYPE],
            "application/json"
        );
    }

    #[tokio::test]
    async fn authentication_and_resource_limit_errors_are_json() {
        let config = ServerConfig {
            auth_token: Some("secret".to_owned()),
            ..ServerConfig::default()
        };
        let service = app(config);
        let unauthorized = service
            .clone()
            .oneshot(json_request("/v1/validate", CASE))
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(unauthorized.headers()[header::WWW_AUTHENTICATE], "Bearer");

        let authorized = service
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/validate")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, "Bearer secret")
                    .body(Body::from(CASE))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);

        let limits = bellhop::solver::SimulationLimits {
            max_rays: 1,
            ..bellhop::solver::SimulationLimits::default()
        };
        let limited = app(ServerConfig {
            simulation_limits: limits,
            ..ServerConfig::default()
        })
        .oneshot(json_request("/v1/run", CASE))
        .await
        .unwrap();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(limited.headers()[header::CONTENT_TYPE], "application/json");
    }
}

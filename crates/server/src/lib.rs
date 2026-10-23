#![forbid(unsafe_code)]

//! Synchronous HTTP facade for modern, self-contained BELLHOP cases.

use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::error_handling::HandleErrorLayer;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bellhop::Case;
use bellhop::diagnostic::{Diagnostic, DiagnosticReport, LoadOutcome, Severity};
use bellhop::json::{CaseDocument, DocumentErrorKind};
use bellhop::model::RunKind;
use bellhop::solver::{
    Arrival, FieldSample, ReceiverArrivals, SimulationLimits, SimulationResult, SourceArrivals,
    SourceField, run as run_simulation,
};
use serde::Serialize;
use serde::ser::{SerializeStruct, Serializer};
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
            auth_token: None,
        }
    }
}

#[derive(Clone)]
struct AppState {
    simulation_limits: SimulationLimits,
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
#[allow(dead_code)]
struct Hdf5Response(Vec<u8>);

#[derive(ToSchema)]
#[allow(dead_code)]
struct ArrivalsResponse {
    schema_version: u32,
    title: String,
    frequency_hz: f64,
    warnings: Vec<DiagnosticResponse>,
    sources: Vec<ArrivalSourceResponse>,
}

#[derive(ToSchema)]
#[allow(dead_code)]
struct ArrivalSourceResponse {
    source_depth_m: f64,
    receivers: Vec<ArrivalReceiverResponse>,
}

#[derive(ToSchema)]
#[allow(dead_code)]
struct ArrivalReceiverResponse {
    range_m: f64,
    depth_m: f64,
    arrivals: Vec<ArrivalResponse>,
}

#[derive(ToSchema)]
#[allow(dead_code)]
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

#[derive(ToSchema)]
#[allow(dead_code)]
struct FieldResponse {
    schema_version: u32,
    title: String,
    frequency_hz: f64,
    run_kind: FieldRunKindResponse,
    warnings: Vec<DiagnosticResponse>,
    sources: Vec<FieldSourceResponse>,
}

#[derive(Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum FieldRunKindResponse {
    Coherent,
    SemiCoherent,
    Incoherent,
}

#[derive(ToSchema)]
#[allow(dead_code)]
struct FieldSourceResponse {
    source_depth_m: f64,
    receivers: Vec<FieldReceiverResponse>,
}

#[derive(ToSchema)]
#[allow(dead_code)]
struct FieldReceiverResponse {
    range_m: f64,
    depth_m: f64,
    pressure: PressureResponse,
}

#[derive(ToSchema)]
#[allow(dead_code)]
struct PressureResponse {
    real: f32,
    imaginary: f32,
}

#[derive(Serialize)]
struct BorrowedArrivalsResponse<'a> {
    schema_version: u32,
    title: &'a str,
    frequency_hz: f64,
    warnings: Vec<DiagnosticResponse>,
    sources: BorrowedArrivalSources<'a>,
}

struct BorrowedArrivalSources<'a>(&'a [SourceArrivals]);
struct BorrowedArrivalSource<'a>(&'a SourceArrivals);
struct BorrowedArrivalReceivers<'a>(&'a [ReceiverArrivals]);
struct BorrowedArrivalReceiver<'a>(&'a ReceiverArrivals);
struct BorrowedArrivals<'a>(&'a [Arrival]);
struct BorrowedArrival<'a>(&'a Arrival);

#[derive(Serialize)]
struct BorrowedFieldResponse<'a> {
    schema_version: u32,
    title: &'a str,
    frequency_hz: f64,
    run_kind: FieldRunKindResponse,
    warnings: Vec<DiagnosticResponse>,
    sources: BorrowedFieldSources<'a>,
}

struct BorrowedFieldSources<'a>(&'a [SourceField]);
struct BorrowedFieldSource<'a>(&'a SourceField);
struct BorrowedFieldReceivers<'a>(&'a [FieldSample]);
struct BorrowedFieldReceiver<'a>(&'a FieldSample);
struct BorrowedPressure<'a>(&'a FieldSample);

impl Serialize for BorrowedArrivalSources<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(self.0.iter().map(BorrowedArrivalSource))
    }
}

impl Serialize for BorrowedArrivalSource<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ArrivalSourceResponse", 2)?;
        state.serialize_field("source_depth_m", &self.0.source_depth_m)?;
        state.serialize_field("receivers", &BorrowedArrivalReceivers(&self.0.receivers))?;
        state.end()
    }
}

impl Serialize for BorrowedArrivalReceivers<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(self.0.iter().map(BorrowedArrivalReceiver))
    }
}

impl Serialize for BorrowedArrivalReceiver<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ArrivalReceiverResponse", 3)?;
        state.serialize_field("range_m", &self.0.range_m)?;
        state.serialize_field("depth_m", &self.0.depth_m)?;
        state.serialize_field("arrivals", &BorrowedArrivals(&self.0.arrivals))?;
        state.end()
    }
}

impl Serialize for BorrowedArrivals<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(self.0.iter().map(BorrowedArrival))
    }
}

impl Serialize for BorrowedArrival<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ArrivalResponse", 8)?;
        state.serialize_field("amplitude", &self.0.amplitude)?;
        state.serialize_field("phase_radians", &self.0.phase_radians)?;
        state.serialize_field("travel_time_s", &self.0.travel_time_s)?;
        state.serialize_field("attenuation_time_s", &self.0.attenuation_time_s)?;
        state.serialize_field("source_angle_degrees", &self.0.source_angle_degrees)?;
        state.serialize_field("receiver_angle_degrees", &self.0.receiver_angle_degrees)?;
        state.serialize_field("top_bounces", &self.0.top_bounces)?;
        state.serialize_field("bottom_bounces", &self.0.bottom_bounces)?;
        state.end()
    }
}

impl Serialize for BorrowedFieldSources<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(self.0.iter().map(BorrowedFieldSource))
    }
}

impl Serialize for BorrowedFieldSource<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("FieldSourceResponse", 2)?;
        state.serialize_field("source_depth_m", &self.0.source_depth_m)?;
        state.serialize_field("receivers", &BorrowedFieldReceivers(&self.0.samples))?;
        state.end()
    }
}

impl Serialize for BorrowedFieldReceivers<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(self.0.iter().map(BorrowedFieldReceiver))
    }
}

impl Serialize for BorrowedFieldReceiver<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("FieldReceiverResponse", 3)?;
        state.serialize_field("range_m", &self.0.range_m)?;
        state.serialize_field("depth_m", &self.0.depth_m)?;
        state.serialize_field("pressure", &BorrowedPressure(self.0))?;
        state.end()
    }
}

impl Serialize for BorrowedPressure<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("PressureResponse", 2)?;
        state.serialize_field("real", &self.0.pressure.re)?;
        state.serialize_field("imaginary", &self.0.pressure.im)?;
        state.end()
    }
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
        &BorrowedArrivalsResponse {
            schema_version: 1,
            title: &result.title,
            frequency_hz: result.frequency_hz,
            warnings: warnings.iter().map(DiagnosticResponse::from).collect(),
            sources: BorrowedArrivalSources(&result.arrival_sources),
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
        &BorrowedFieldResponse {
            schema_version: 1,
            title: &result.title,
            frequency_hz: result.frequency_hz,
            run_kind,
            warnings: warnings.iter().map(DiagnosticResponse::from).collect(),
            sources: BorrowedFieldSources(&result.field_sources),
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
async fn validate_case(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<ValidationResponse>, ApiError> {
    let body = extract_json_body(&headers, body)?;
    let outcome = parse_document(&body)?;
    Ok(Json(ValidationResponse {
        valid: true,
        warnings: outcome
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
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, ApiError> {
    let body = extract_json_body(&headers, body)?;
    let outcome = parse_document(&body)?;
    let request_bytes = body.to_vec();
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

    let mut response = bytes.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(HDF5_MEDIA_TYPE));
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"bellhop-result.h5\""),
    );
    Ok(response)
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
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, ApiError> {
    let body = extract_json_body(&headers, body)?;
    let outcome = parse_document(&body)?;
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
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, ApiError> {
    let body = extract_json_body(&headers, body)?;
    let outcome = parse_document(&body)?;
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
    // Dropping the request (including timeout) does not cancel blocking work.
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
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

fn extract_json_body(
    headers: &HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Bytes, ApiError> {
    let media_type = headers
        .get(CONTENT_TYPE)
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
    body.map_err(|rejection| {
        ApiError::new(
            rejection.status(),
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                "body_too_large"
            } else {
                "body_read_failed"
            },
            rejection.body_text(),
        )
    })
}

fn parse_document(bytes: &[u8]) -> Result<LoadOutcome<Case>, ApiError> {
    bellhop::json::load_case_document(bytes).map_err(|error| {
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
    })
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
    use bellhop::solver::{Arrival, ReceiverArrivals, SimulationResult, SourceArrivals};
    use serde_json::Value;
    use tower::ServiceExt;

    use super::{HDF5_MEDIA_TYPE, ServerConfig, app};

    const CASE: &str = include_str!("../../../examples/cases/bellhop/field-g/field-g.json");

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
        semantic["trace"]["launch_angles_degrees"] = serde_json::json!([]);
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
        arrivals_case["run"]["kind"] = serde_json::json!("arrivals");
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
    async fn field_returns_all_pressure_modes_and_rejects_other_run_kinds() {
        for run_kind in ["coherent", "semi_coherent", "incoherent"] {
            let mut field_case: Value = serde_json::from_str(CASE).unwrap();
            field_case["run"]["kind"] = serde_json::json!(run_kind);
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
        arrivals_case["run"]["kind"] = serde_json::json!("arrivals");
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

//! Versioned, self-contained JSON input format.
//!
//! The document contains all data that legacy inputs keep in same-stem
//! auxiliary files. It maps to the validated [`crate::Case`] and uses
//! the same solver and numerical validation path.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use num_complex::Complex64;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::diagnostic::{Diagnostic, DiagnosticReport, LoadOutcome, SourceLocation};
use crate::model::{
    AttenuationUnit, BeamComponent, BeamFamily, BeamWidth, BiologicalLayer, Boundary,
    BoundaryCondition, BoundaryInterpolation, BoundaryMaterial, BoundaryShape, BoundaryShapePoint,
    CervenyOptions, CurvatureCondition, EnvironmentCase, HalfSpace,
    InternalReflectionCoefficientPoint, InternalReflectionCoefficientTable, LegacyArrivalEncoding,
    Positions, RangeDependentSoundSpeed, ReceiverGrid, ReflectionCoefficientPoint,
    ReflectionCoefficientTable, RunKind, RunOptions, SoundSpeedInput, SoundSpeedPoint,
    SourceBeamPattern, SourceBeamPatternPoint, SourceGeometry, SspInterpolation, TopOptions,
    TraceOptions, VolumeAttenuation,
};
use crate::{Case, CaseDefinition};

pub const SCHEMA_VERSION: u32 = 1;

/// Complete modern BELLHOP input. All quantities use SI units except fields
/// whose names explicitly end in `_degrees`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseDocument {
    #[schema(minimum = 1, maximum = 1, example = 1)]
    pub schema_version: u32,
    pub title: String,
    pub frequency_hz: f64,
    pub sound_speed: SoundSpeedDocument,
    pub top_boundary: BoundaryDocument,
    pub bottom_boundary: BoundaryDocument,
    pub positions: PositionsDocument,
    pub run: RunDocument,
    pub trace: TraceDocument,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_beam_pattern: Option<SourceBeamPatternDocument>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundSpeedDocument {
    pub interpolation: SspInterpolationDocument,
    pub attenuation_unit: AttenuationUnitDocument,
    pub volume_attenuation: VolumeAttenuationDocument,
    pub nominal_point_count: i32,
    pub top_depth_m: f64,
    pub bottom_depth_m: f64,
    pub points: Vec<SoundSpeedPointDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_dependent: Option<RangeDependentSoundSpeedDocument>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SspInterpolationDocument {
    N2Linear,
    CLinear,
    Pchip,
    CubicSpline,
    Quadrilateral,
    AnalyticMunk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttenuationUnitDocument {
    NepersPerMeter,
    DbPerMeterKhz,
    DbPerMeter,
    DbPerWavelength,
    QualityFactor,
    LossParameter,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VolumeAttenuationDocument {
    None,
    Thorp,
    FrancoisGarrison {
        temperature_c: f64,
        salinity_psu: f64,
        ph: f64,
        mean_depth_m: f64,
    },
    Biological {
        layers: Vec<BiologicalLayerDocument>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BiologicalLayerDocument {
    pub top_depth_m: f64,
    pub bottom_depth_m: f64,
    pub resonance_frequency_hz: f64,
    pub quality_factor: f64,
    pub attenuation: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundSpeedPointDocument {
    pub depth_m: f64,
    pub compressional_speed_mps: f64,
    pub shear_speed_mps: f64,
    pub density_kg_m3: f64,
    pub compressional_attenuation: f64,
    pub shear_attenuation: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeDependentSoundSpeedDocument {
    pub ranges_m: Vec<f64>,
    pub depths_m: Vec<f64>,
    /// Matrix indexed as `[depth_index][range_index]`.
    pub speeds_mps: Vec<Vec<f64>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundaryDocument {
    pub roughness_m: f32,
    pub condition: BoundaryConditionDocument,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<BoundaryShapeDocument>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoundaryConditionDocument {
    Vacuum,
    Rigid,
    AcoustoElastic {
        half_space: HalfSpaceDocument,
    },
    GrainSize {
        depth_m: f64,
        phi: f64,
    },
    ReflectionCoefficients {
        table: ReflectionCoefficientTableDocument,
    },
    PrecalculatedInternalReflection {
        table: InternalReflectionCoefficientTableDocument,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HalfSpaceDocument {
    pub depth_m: f64,
    pub compressional_speed_mps: f64,
    pub shear_speed_mps: f64,
    pub density_kg_m3: f64,
    pub compressional_attenuation: f64,
    pub shear_attenuation: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundaryShapeDocument {
    pub interpolation: BoundaryInterpolationDocument,
    pub points: Vec<BoundaryShapePointDocument>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryInterpolationDocument {
    PiecewiseLinear,
    Curvilinear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundaryShapePointDocument {
    pub range_m: f64,
    pub depth_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<BoundaryMaterialDocument>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundaryMaterialDocument {
    pub compressional_speed_mps: f64,
    pub shear_speed_mps: f64,
    pub density_kg_m3: f64,
    pub compressional_attenuation: f64,
    pub shear_attenuation: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflectionCoefficientTableDocument {
    pub points: Vec<ReflectionCoefficientPointDocument>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflectionCoefficientPointDocument {
    pub angle_degrees: f64,
    pub magnitude: f64,
    pub phase_degrees: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InternalReflectionCoefficientTableDocument {
    pub title: String,
    pub frequency_hz: f64,
    pub points: Vec<InternalReflectionCoefficientPointDocument>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InternalReflectionCoefficientPointDocument {
    pub horizontal_wavenumber_squared: f64,
    pub f: ComplexValue,
    pub g: ComplexValue,
    pub decimal_power: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ComplexValue {
    pub real: f64,
    pub imaginary: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PositionsDocument {
    pub source_depths_m: Vec<f64>,
    pub receiver_depths_m: Vec<f64>,
    pub receiver_ranges_m: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RunDocument {
    pub kind: RunKindDocument,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beam_family: Option<BeamFamilyDocument>,
    pub source_geometry: SourceGeometryDocument,
    pub receiver_grid: ReceiverGridDocument,
    pub beam_shift: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunKindDocument {
    Rays,
    Eigenrays,
    Coherent,
    SemiCoherent,
    Incoherent,
    Arrivals,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BeamFamilyDocument {
    GeometricHatCartesian,
    GeometricHatRayCentered,
    GeometricGaussianCartesian,
    GeometricGaussianRayCentered,
    SimpleGaussian,
    CervenyCartesian,
    CervenyRayCentered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceGeometryDocument {
    Point,
    Line,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiverGridDocument {
    Rectilinear,
    Irregular,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceDocument {
    pub launch_angles_degrees: Vec<f64>,
    /// One-based index, matching legacy BELLHOP's selected-beam option.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_launch_angle: Option<usize>,
    /// Zero requests the reference automatic step size.
    pub step_m: f64,
    pub max_depth_m: f64,
    pub max_range_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cerveny: Option<CervenyDocument>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CervenyDocument {
    pub width: BeamWidthDocument,
    pub curvature: CurvatureConditionDocument,
    pub epsilon_multiplier: f64,
    pub loop_range: f64,
    pub image_count: i32,
    pub beam_window: i32,
    pub component: BeamComponentDocument,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BeamWidthDocument {
    SpaceFilling,
    Minimum,
    Wkb,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CurvatureConditionDocument {
    Double,
    Standard,
    Zero,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BeamComponentDocument {
    Pressure,
    Vertical,
    Horizontal,
    Displacement,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceBeamPatternDocument {
    pub points: Vec<SourceBeamPatternPointDocument>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceBeamPatternPointDocument {
    pub angle_degrees: f64,
    pub level_db: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentErrorKind {
    Malformed,
    Semantic,
}

#[derive(Clone, Debug)]
pub struct DocumentError {
    kind: DocumentErrorKind,
    report: DiagnosticReport,
}

impl DocumentError {
    #[must_use]
    pub fn kind(&self) -> DocumentErrorKind {
        self.kind
    }

    #[must_use]
    pub fn report(&self) -> &DiagnosticReport {
        &self.report
    }

    #[must_use]
    pub fn into_report(self) -> DiagnosticReport {
        self.report
    }
}

impl fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.report.fmt(formatter)
    }
}

impl Error for DocumentError {}

/// Parses and validates a self-contained JSON case.
///
/// # Errors
///
/// Returns [`DocumentErrorKind::Malformed`] for JSON syntax/schema failures and
/// [`DocumentErrorKind::Semantic`] when the decoded case is inconsistent.
pub fn load_case_document(bytes: &[u8]) -> Result<LoadOutcome<Case>, DocumentError> {
    load_case_document_named(bytes, Path::new("request.json"))
}

/// Parses and validates a self-contained JSON case with a diagnostic source name.
///
/// # Errors
///
/// Returns [`DocumentErrorKind::Malformed`] for JSON syntax/schema failures and
/// [`DocumentErrorKind::Semantic`] when the decoded case is inconsistent.
pub fn load_case_document_named(
    bytes: &[u8],
    source_path: &Path,
) -> Result<LoadOutcome<Case>, DocumentError> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let document: CaseDocument =
        serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
            let diagnostic = Diagnostic::error(
                "BH0103",
                format!("invalid JSON case: {error}"),
                error.path().to_string(),
                SourceLocation::new(source_path, error.inner().line(), error.inner().column()),
            );
            DocumentError {
                kind: DocumentErrorKind::Malformed,
                report: DiagnosticReport::from_diagnostic(diagnostic),
            }
        })?;
    decoder.end().map_err(|error| DocumentError {
        kind: DocumentErrorKind::Malformed,
        report: DiagnosticReport::from_diagnostic(Diagnostic::error(
            "BH0103",
            format!("invalid JSON case: {error}"),
            "json",
            SourceLocation::new(source_path, error.line(), error.column()),
        )),
    })?;
    document.into_case(source_path)
}

/// Serializes a loaded legacy or JSON case into the canonical modern document.
///
/// # Errors
///
/// Returns diagnostics if a loaded case contains unsupported legacy-only
/// behavior or is missing data required by its boundary condition.
pub fn export_case_document(case: &Case) -> Result<CaseDocument, DiagnosticReport> {
    CaseDocument::try_from(case)
}

impl CaseDocument {
    #[allow(clippy::too_many_lines)]
    fn into_case(self, source_path: &Path) -> Result<LoadOutcome<Case>, DocumentError> {
        let mut diagnostics = DiagnosticReport::default();
        if self.schema_version != SCHEMA_VERSION {
            diagnostics.push(error(
                source_path,
                "BH0202",
                format!(
                    "unsupported JSON case schema version {}; expected {SCHEMA_VERSION}",
                    self.schema_version
                ),
                "schema_version",
            ));
        }

        let top = import_boundary(self.top_boundary);
        let bottom = import_boundary(self.bottom_boundary);
        let source_beam_pattern = self.source_beam_pattern.map(import_source_pattern);
        let run_kind = import_run_kind(self.run.kind);
        let beam_family = self.run.beam_family.map(import_beam_family);
        let interpolation = import_ssp_interpolation(self.sound_speed.interpolation);

        let (top_boundary, altimetry, top_reflection, _) = top;
        let (bottom_boundary, bathymetry, bottom_reflection, internal_reflection) = bottom;
        let surface_roughness_m = top_boundary.roughness_m;
        let definition = CaseDefinition {
            environment: EnvironmentCase {
                source_path: source_path.to_path_buf(),
                title: self.title,
                frequency_hz: self.frequency_hz,
                top_options: TopOptions {
                    legacy: String::new(),
                    interpolation,
                    attenuation_unit: import_attenuation_unit(self.sound_speed.attenuation_unit),
                    volume_attenuation: import_volume_attenuation(
                        self.sound_speed.volume_attenuation,
                    ),
                    has_altimetry: altimetry.is_some(),
                    development_options: self.trace.selected_launch_angle.is_some(),
                },
                top_boundary,
                sound_speed: SoundSpeedInput {
                    nominal_point_count: self.sound_speed.nominal_point_count,
                    surface_roughness_m,
                    top_depth_m: self.sound_speed.top_depth_m,
                    bottom_depth_m: self.sound_speed.bottom_depth_m,
                    points: self
                        .sound_speed
                        .points
                        .into_iter()
                        .map(import_sound_speed_point)
                        .collect(),
                },
                bottom_boundary,
                positions: Positions {
                    source_depths_m: self.positions.source_depths_m,
                    receiver_depths_m: self.positions.receiver_depths_m,
                    receiver_ranges_m: self.positions.receiver_ranges_m,
                },
                run: RunOptions {
                    legacy: String::new(),
                    kind: run_kind,
                    arrival_encoding: (run_kind == RunKind::Arrivals)
                        .then_some(LegacyArrivalEncoding::Ascii),
                    beam_family,
                    has_source_beam_pattern: source_beam_pattern.is_some(),
                    source_geometry: import_source_geometry(self.run.source_geometry),
                    receiver_grid: import_receiver_grid(self.run.receiver_grid),
                    beam_shift: self.run.beam_shift,
                },
                trace: TraceOptions {
                    launch_angles_degrees: self.trace.launch_angles_degrees,
                    selected_launch_angle: self.trace.selected_launch_angle,
                    step_m: self.trace.step_m,
                    max_depth_m: self.trace.max_depth_m,
                    max_range_m: self.trace.max_range_m,
                    cerveny: self.trace.cerveny.map(import_cerveny),
                },
            },
            range_dependent_sound_speed: self
                .sound_speed
                .range_dependent
                .map(import_range_dependent_sound_speed),
            altimetry,
            bathymetry,
            bottom_reflection,
            top_reflection,
            internal_reflection,
            source_beam_pattern,
        };
        Case::from_definition_with(definition, &HashMap::new(), diagnostics).map_err(|report| {
            DocumentError {
                kind: DocumentErrorKind::Semantic,
                report,
            }
        })
    }
}

impl TryFrom<&Case> for CaseDocument {
    type Error = DiagnosticReport;

    fn try_from(case: &Case) -> Result<Self, Self::Error> {
        let top_boundary = export_boundary(
            &case.environment.top_boundary,
            case.altimetry.as_ref(),
            case.top_reflection.as_ref(),
            None,
            true,
            &case.environment.source_path,
        )?;
        let bottom_boundary = export_boundary(
            &case.environment.bottom_boundary,
            case.bathymetry.as_ref(),
            case.bottom_reflection.as_ref(),
            case.internal_reflection.as_ref(),
            false,
            &case.environment.source_path,
        )?;
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            title: case.environment.title.clone(),
            frequency_hz: case.environment.frequency_hz,
            sound_speed: SoundSpeedDocument {
                interpolation: export_ssp_interpolation(case.environment.top_options.interpolation),
                attenuation_unit: export_attenuation_unit(
                    case.environment.top_options.attenuation_unit,
                ),
                volume_attenuation: export_volume_attenuation(
                    &case.environment.top_options.volume_attenuation,
                ),
                nominal_point_count: case.environment.sound_speed.nominal_point_count,
                top_depth_m: case.environment.sound_speed.top_depth_m,
                bottom_depth_m: case.environment.sound_speed.bottom_depth_m,
                points: case
                    .environment
                    .sound_speed
                    .points
                    .iter()
                    .map(export_sound_speed_point)
                    .collect(),
                range_dependent: case
                    .range_dependent_sound_speed
                    .as_ref()
                    .map(export_range_dependent_sound_speed),
            },
            top_boundary,
            bottom_boundary,
            positions: PositionsDocument {
                source_depths_m: case.environment.positions.source_depths_m.clone(),
                receiver_depths_m: case.environment.positions.receiver_depths_m.clone(),
                receiver_ranges_m: case.environment.positions.receiver_ranges_m.clone(),
            },
            run: RunDocument {
                kind: export_run_kind(case.environment.run.kind),
                beam_family: case.environment.run.beam_family.map(export_beam_family),
                source_geometry: export_source_geometry(case.environment.run.source_geometry),
                receiver_grid: export_receiver_grid(case.environment.run.receiver_grid),
                beam_shift: case.environment.run.beam_shift,
            },
            trace: TraceDocument {
                launch_angles_degrees: case.environment.trace.launch_angles_degrees.clone(),
                selected_launch_angle: case.environment.trace.selected_launch_angle,
                step_m: case.environment.trace.step_m,
                max_depth_m: case.environment.trace.max_depth_m,
                max_range_m: case.environment.trace.max_range_m,
                cerveny: case.environment.trace.cerveny.as_ref().map(export_cerveny),
            },
            source_beam_pattern: case.source_beam_pattern.as_ref().map(export_source_pattern),
        })
    }
}

fn import_boundary(
    document: BoundaryDocument,
) -> (
    Boundary,
    Option<BoundaryShape>,
    Option<ReflectionCoefficientTable>,
    Option<InternalReflectionCoefficientTable>,
) {
    let shape = document.shape.map(import_boundary_shape);
    let (condition, reflection, internal) = match document.condition {
        BoundaryConditionDocument::Vacuum => (BoundaryCondition::Vacuum, None, None),
        BoundaryConditionDocument::Rigid => (BoundaryCondition::Rigid, None, None),
        BoundaryConditionDocument::AcoustoElastic { half_space } => (
            BoundaryCondition::AcoustoElastic(import_half_space(half_space)),
            None,
            None,
        ),
        BoundaryConditionDocument::GrainSize { depth_m, phi } => {
            (BoundaryCondition::GrainSize { depth_m, phi }, None, None)
        }
        BoundaryConditionDocument::ReflectionCoefficients { table } => (
            BoundaryCondition::ReflectionCoefficientFile,
            Some(import_reflection_table(table)),
            None,
        ),
        BoundaryConditionDocument::PrecalculatedInternalReflection { table } => (
            BoundaryCondition::PrecalculatedReflectionCoefficient,
            None,
            Some(import_internal_reflection_table(table)),
        ),
    };
    (
        Boundary {
            legacy_options: String::new(),
            condition,
            roughness_m: document.roughness_m,
            has_shape_file: shape.is_some(),
        },
        shape,
        reflection,
        internal,
    )
}

fn export_boundary(
    boundary: &Boundary,
    shape: Option<&BoundaryShape>,
    reflection: Option<&ReflectionCoefficientTable>,
    internal: Option<&InternalReflectionCoefficientTable>,
    top: bool,
    path: &Path,
) -> Result<BoundaryDocument, DiagnosticReport> {
    let missing = |field: &'static str| {
        DiagnosticReport::from_diagnostic(error(
            path,
            "BH0201",
            "loaded boundary data is missing",
            field,
        ))
    };
    let condition = match &boundary.condition {
        BoundaryCondition::Vacuum => BoundaryConditionDocument::Vacuum,
        BoundaryCondition::Rigid => BoundaryConditionDocument::Rigid,
        BoundaryCondition::AcoustoElastic(half_space) => {
            BoundaryConditionDocument::AcoustoElastic {
                half_space: export_half_space(half_space),
            }
        }
        BoundaryCondition::GrainSize { depth_m, phi } => BoundaryConditionDocument::GrainSize {
            depth_m: *depth_m,
            phi: *phi,
        },
        BoundaryCondition::ReflectionCoefficientFile => {
            BoundaryConditionDocument::ReflectionCoefficients {
                table: export_reflection_table(
                    reflection.ok_or_else(|| missing("boundary.reflection_coefficients"))?,
                ),
            }
        }
        BoundaryCondition::PrecalculatedReflectionCoefficient => {
            if top {
                return Err(DiagnosticReport::from_diagnostic(error(
                    path,
                    "BH0202",
                    "top precalculated reflection cannot be exported",
                    "top_boundary.condition",
                )));
            }
            BoundaryConditionDocument::PrecalculatedInternalReflection {
                table: export_internal_reflection_table(
                    internal.ok_or_else(|| missing("bottom_boundary.internal_reflection"))?,
                ),
            }
        }
        BoundaryCondition::WriteReflectionCoefficient => {
            return Err(DiagnosticReport::from_diagnostic(error(
                path,
                "BH0301",
                "W reflection-table generation has no modern JSON representation",
                "boundary.condition",
            )));
        }
    };
    Ok(BoundaryDocument {
        roughness_m: boundary.roughness_m,
        condition,
        shape: shape.map(export_boundary_shape),
    })
}

fn import_sound_speed_point(point: SoundSpeedPointDocument) -> SoundSpeedPoint {
    SoundSpeedPoint {
        depth_m: point.depth_m,
        compressional_speed_mps: point.compressional_speed_mps,
        shear_speed_mps: point.shear_speed_mps,
        density_g_cm3: point.density_kg_m3 / 1000.0,
        compressional_attenuation: point.compressional_attenuation,
        shear_attenuation: point.shear_attenuation,
    }
}

fn export_sound_speed_point(point: &SoundSpeedPoint) -> SoundSpeedPointDocument {
    SoundSpeedPointDocument {
        depth_m: point.depth_m,
        compressional_speed_mps: point.compressional_speed_mps,
        shear_speed_mps: point.shear_speed_mps,
        density_kg_m3: point.density_g_cm3 * 1000.0,
        compressional_attenuation: point.compressional_attenuation,
        shear_attenuation: point.shear_attenuation,
    }
}

fn import_range_dependent_sound_speed(
    field: RangeDependentSoundSpeedDocument,
) -> RangeDependentSoundSpeed {
    RangeDependentSoundSpeed {
        ranges_m: field.ranges_m,
        depths_m: field.depths_m,
        speeds_mps: field.speeds_mps,
    }
}

fn export_range_dependent_sound_speed(
    field: &RangeDependentSoundSpeed,
) -> RangeDependentSoundSpeedDocument {
    RangeDependentSoundSpeedDocument {
        ranges_m: field.ranges_m.clone(),
        depths_m: field.depths_m.clone(),
        speeds_mps: field.speeds_mps.clone(),
    }
}

fn import_boundary_shape(shape: BoundaryShapeDocument) -> BoundaryShape {
    BoundaryShape {
        interpolation: import_boundary_interpolation(shape.interpolation),
        points: shape
            .points
            .into_iter()
            .map(|point| BoundaryShapePoint {
                range_m: point.range_m,
                depth_m: point.depth_m,
                material: point.material.map(import_boundary_material),
            })
            .collect(),
    }
}

fn export_boundary_shape(shape: &BoundaryShape) -> BoundaryShapeDocument {
    BoundaryShapeDocument {
        interpolation: export_boundary_interpolation(shape.interpolation),
        points: shape
            .points
            .iter()
            .map(|point| BoundaryShapePointDocument {
                range_m: point.range_m,
                depth_m: point.depth_m,
                material: point.material.as_ref().map(export_boundary_material),
            })
            .collect(),
    }
}

fn import_boundary_material(material: BoundaryMaterialDocument) -> BoundaryMaterial {
    BoundaryMaterial {
        compressional_speed_mps: material.compressional_speed_mps,
        shear_speed_mps: material.shear_speed_mps,
        density_g_cm3: material.density_kg_m3 / 1000.0,
        compressional_attenuation: material.compressional_attenuation,
        shear_attenuation: material.shear_attenuation,
    }
}

fn export_boundary_material(material: &BoundaryMaterial) -> BoundaryMaterialDocument {
    BoundaryMaterialDocument {
        compressional_speed_mps: material.compressional_speed_mps,
        shear_speed_mps: material.shear_speed_mps,
        density_kg_m3: material.density_g_cm3 * 1000.0,
        compressional_attenuation: material.compressional_attenuation,
        shear_attenuation: material.shear_attenuation,
    }
}

fn import_half_space(half_space: HalfSpaceDocument) -> HalfSpace {
    HalfSpace {
        depth_m: half_space.depth_m,
        compressional_speed_mps: half_space.compressional_speed_mps,
        shear_speed_mps: half_space.shear_speed_mps,
        density_g_cm3: half_space.density_kg_m3 / 1000.0,
        compressional_attenuation: half_space.compressional_attenuation,
        shear_attenuation: half_space.shear_attenuation,
    }
}

fn export_half_space(half_space: &HalfSpace) -> HalfSpaceDocument {
    HalfSpaceDocument {
        depth_m: half_space.depth_m,
        compressional_speed_mps: half_space.compressional_speed_mps,
        shear_speed_mps: half_space.shear_speed_mps,
        density_kg_m3: half_space.density_g_cm3 * 1000.0,
        compressional_attenuation: half_space.compressional_attenuation,
        shear_attenuation: half_space.shear_attenuation,
    }
}

fn import_reflection_table(
    table: ReflectionCoefficientTableDocument,
) -> ReflectionCoefficientTable {
    ReflectionCoefficientTable {
        points: table
            .points
            .into_iter()
            .map(|point| ReflectionCoefficientPoint {
                angle_degrees: point.angle_degrees,
                magnitude: point.magnitude,
                phase_radians: point.phase_degrees.to_radians(),
            })
            .collect(),
    }
}

fn export_reflection_table(
    table: &ReflectionCoefficientTable,
) -> ReflectionCoefficientTableDocument {
    ReflectionCoefficientTableDocument {
        points: table
            .points
            .iter()
            .map(|point| ReflectionCoefficientPointDocument {
                angle_degrees: point.angle_degrees,
                magnitude: point.magnitude,
                phase_degrees: point.phase_radians.to_degrees(),
            })
            .collect(),
    }
}

fn import_internal_reflection_table(
    table: InternalReflectionCoefficientTableDocument,
) -> InternalReflectionCoefficientTable {
    InternalReflectionCoefficientTable {
        title: table.title,
        frequency_hz: table.frequency_hz,
        points: table
            .points
            .into_iter()
            .map(|point| InternalReflectionCoefficientPoint {
                horizontal_wavenumber_squared: point.horizontal_wavenumber_squared,
                f: Complex64::new(point.f.real, point.f.imaginary),
                g: Complex64::new(point.g.real, point.g.imaginary),
                decimal_power: point.decimal_power,
            })
            .collect(),
    }
}

fn export_internal_reflection_table(
    table: &InternalReflectionCoefficientTable,
) -> InternalReflectionCoefficientTableDocument {
    InternalReflectionCoefficientTableDocument {
        title: table.title.clone(),
        frequency_hz: table.frequency_hz,
        points: table
            .points
            .iter()
            .map(|point| InternalReflectionCoefficientPointDocument {
                horizontal_wavenumber_squared: point.horizontal_wavenumber_squared,
                f: ComplexValue {
                    real: point.f.re,
                    imaginary: point.f.im,
                },
                g: ComplexValue {
                    real: point.g.re,
                    imaginary: point.g.im,
                },
                decimal_power: point.decimal_power,
            })
            .collect(),
    }
}

fn import_source_pattern(pattern: SourceBeamPatternDocument) -> SourceBeamPattern {
    SourceBeamPattern {
        points: pattern
            .points
            .into_iter()
            .map(|point| SourceBeamPatternPoint {
                angle_degrees: point.angle_degrees,
                level_db: point.level_db,
                amplitude: 10.0_f64.powf(point.level_db / 20.0),
            })
            .collect(),
    }
}

fn export_source_pattern(pattern: &SourceBeamPattern) -> SourceBeamPatternDocument {
    SourceBeamPatternDocument {
        points: pattern
            .points
            .iter()
            .map(|point| SourceBeamPatternPointDocument {
                angle_degrees: point.angle_degrees,
                level_db: point.level_db,
            })
            .collect(),
    }
}

fn import_cerveny(options: CervenyDocument) -> CervenyOptions {
    CervenyOptions {
        width: import_beam_width(options.width),
        curvature: import_curvature(options.curvature),
        epsilon_multiplier: options.epsilon_multiplier,
        loop_range: options.loop_range,
        image_count: options.image_count,
        beam_window: options.beam_window,
        component: import_beam_component(options.component),
    }
}

fn export_cerveny(options: &CervenyOptions) -> CervenyDocument {
    CervenyDocument {
        width: export_beam_width(options.width),
        curvature: export_curvature(options.curvature),
        epsilon_multiplier: options.epsilon_multiplier,
        loop_range: options.loop_range,
        image_count: options.image_count,
        beam_window: options.beam_window,
        component: export_beam_component(options.component),
    }
}

fn import_volume_attenuation(value: VolumeAttenuationDocument) -> VolumeAttenuation {
    match value {
        VolumeAttenuationDocument::None => VolumeAttenuation::None,
        VolumeAttenuationDocument::Thorp => VolumeAttenuation::Thorp,
        VolumeAttenuationDocument::FrancoisGarrison {
            temperature_c,
            salinity_psu,
            ph,
            mean_depth_m,
        } => VolumeAttenuation::FrancoisGarrison {
            temperature_c,
            salinity_psu,
            ph,
            mean_depth_m,
        },
        VolumeAttenuationDocument::Biological { layers } => VolumeAttenuation::Biological {
            layers: layers
                .into_iter()
                .map(|layer| BiologicalLayer {
                    top_depth_m: layer.top_depth_m,
                    bottom_depth_m: layer.bottom_depth_m,
                    resonance_frequency_hz: layer.resonance_frequency_hz,
                    quality_factor: layer.quality_factor,
                    attenuation: layer.attenuation,
                })
                .collect(),
        },
    }
}

fn export_volume_attenuation(value: &VolumeAttenuation) -> VolumeAttenuationDocument {
    match value {
        VolumeAttenuation::None => VolumeAttenuationDocument::None,
        VolumeAttenuation::Thorp => VolumeAttenuationDocument::Thorp,
        VolumeAttenuation::FrancoisGarrison {
            temperature_c,
            salinity_psu,
            ph,
            mean_depth_m,
        } => VolumeAttenuationDocument::FrancoisGarrison {
            temperature_c: *temperature_c,
            salinity_psu: *salinity_psu,
            ph: *ph,
            mean_depth_m: *mean_depth_m,
        },
        VolumeAttenuation::Biological { layers } => VolumeAttenuationDocument::Biological {
            layers: layers
                .iter()
                .map(|layer| BiologicalLayerDocument {
                    top_depth_m: layer.top_depth_m,
                    bottom_depth_m: layer.bottom_depth_m,
                    resonance_frequency_hz: layer.resonance_frequency_hz,
                    quality_factor: layer.quality_factor,
                    attenuation: layer.attenuation,
                })
                .collect(),
        },
    }
}

macro_rules! enum_map {
    ($import:ident, $export:ident, $doc:ty, $model:ty, { $($variant:ident),+ $(,)? }) => {
        fn $import(value: $doc) -> $model {
            match value { $(<$doc>::$variant => <$model>::$variant),+ }
        }
        fn $export(value: $model) -> $doc {
            match value { $(<$model>::$variant => <$doc>::$variant),+ }
        }
    };
}

enum_map!(import_boundary_interpolation, export_boundary_interpolation, BoundaryInterpolationDocument, BoundaryInterpolation, {
    PiecewiseLinear, Curvilinear
});
enum_map!(import_beam_width, export_beam_width, BeamWidthDocument, BeamWidth, {
    SpaceFilling, Minimum, Wkb
});
enum_map!(import_curvature, export_curvature, CurvatureConditionDocument, CurvatureCondition, {
    Double, Standard, Zero
});
enum_map!(import_beam_component, export_beam_component, BeamComponentDocument, BeamComponent, {
    Pressure, Vertical, Horizontal, Displacement
});
enum_map!(import_ssp_interpolation, export_ssp_interpolation, SspInterpolationDocument, SspInterpolation, {
    N2Linear, CLinear, Pchip, CubicSpline, Quadrilateral, AnalyticMunk
});
enum_map!(import_attenuation_unit, export_attenuation_unit, AttenuationUnitDocument, AttenuationUnit, {
    NepersPerMeter, DbPerMeterKhz, DbPerMeter, DbPerWavelength, QualityFactor, LossParameter
});
enum_map!(import_run_kind, export_run_kind, RunKindDocument, RunKind, {
    Rays, Eigenrays, Coherent, SemiCoherent, Incoherent, Arrivals
});
enum_map!(import_beam_family, export_beam_family, BeamFamilyDocument, BeamFamily, {
    GeometricHatCartesian, GeometricHatRayCentered, GeometricGaussianCartesian,
    GeometricGaussianRayCentered, SimpleGaussian, CervenyCartesian, CervenyRayCentered
});
enum_map!(import_source_geometry, export_source_geometry, SourceGeometryDocument, SourceGeometry, {
    Point, Line
});
enum_map!(import_receiver_grid, export_receiver_grid, ReceiverGridDocument, ReceiverGrid, {
    Rectilinear, Irregular
});

fn error(
    path: &Path,
    code: &'static str,
    message: impl Into<String>,
    field: impl Into<String>,
) -> Diagnostic {
    Diagnostic::error(code, message, field, SourceLocation::file(path))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{CaseDocument, DocumentErrorKind, export_case_document, load_case_document};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/golden")
            .join(name)
    }

    #[test]
    fn all_golden_cases_round_trip_through_the_modern_document() {
        let directory = fixture("");
        let mut count = 0;
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("env") {
                continue;
            }
            let case = crate::legacy::load_case(&path).unwrap().value;
            let document = export_case_document(&case).unwrap();
            let encoded = serde_json::to_vec(&document).unwrap();
            let loaded = load_case_document(&encoded).unwrap().value;
            let round_trip = export_case_document(&loaded).unwrap();
            assert_eq!(document, round_trip, "{}", path.display());
            count += 1;
        }
        assert!(count >= 20, "expected the curated golden matrix");
    }

    #[test]
    fn rejects_unknown_fields_and_unknown_enums_as_malformed_json() {
        let case = crate::legacy::load_case(&fixture("Field_G.env"))
            .unwrap()
            .value;
        let document = export_case_document(&case).unwrap();
        let mut value = serde_json::to_value(document).unwrap();
        value["unexpected"] = serde_json::json!(true);
        let error = load_case_document(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(error.kind(), DocumentErrorKind::Malformed);

        value.as_object_mut().unwrap().remove("unexpected");
        value["run"]["kind"] = serde_json::json!("unknown");
        let error = load_case_document(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(error.kind(), DocumentErrorKind::Malformed);
    }

    #[test]
    fn rejects_unsupported_schema_versions() {
        let case = crate::legacy::load_case(&fixture("Field_G.env"))
            .unwrap()
            .value;
        let mut document = export_case_document(&case).unwrap();
        document.schema_version += 1;
        let error = load_case_document(&serde_json::to_vec(&document).unwrap()).unwrap_err();
        assert_eq!(error.kind(), DocumentErrorKind::Semantic);
        assert!(
            error
                .report()
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "BH0202")
        );
    }

    #[test]
    fn rejects_semantically_incomplete_documents() {
        let case = crate::legacy::load_case(&fixture("Field_G.env"))
            .unwrap()
            .value;
        let mut document: CaseDocument = export_case_document(&case).unwrap();
        document.trace.launch_angles_degrees.clear();
        let error = load_case_document(&serde_json::to_vec(&document).unwrap()).unwrap_err();
        assert_eq!(error.kind(), DocumentErrorKind::Semantic);
        assert!(
            error.report().diagnostics().iter().any(
                |diagnostic| diagnostic.field.as_deref() == Some("trace.launch_angles_degrees")
            )
        );
    }
}

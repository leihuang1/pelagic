//! The hand-writable JSON case document, schema version 2.
//!
//! Units are fixed: metres, m/s, Hz, g/cm³ and degrees. Attenuation uses
//! `attenuation.unit`. Anything the `.env` defaults may be omitted, and
//! [`export`](super::export_case_document) omits it again.

use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::path::Path;

use num_complex::Complex64;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ObjectBuilder, OneOfBuilder, Ref, Schema, Type};

use super::grid::{Grid, same_bits};
use super::{DocumentError, error};
use crate::diagnostic::{DiagnosticReport, LoadOutcome};
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

/// Defaults for sound-speed row columns 3–6: shear speed, density, P and S loss.
const ROW_DEFAULTS: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// A complete BELLHOP case.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseDocument {
    /// The schema version, 2.
    #[schema(minimum = 2, maximum = 2, example = 2)]
    pub pelagic: u32,
    pub model: Model,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Hz.
    pub frequency: f64,
    pub interpolation: Interpolation,
    #[serde(default, skip_serializing_if = "Attenuation::is_default")]
    pub attenuation: Attenuation,
    /// BELLHOP has exactly one medium, the water column.
    pub media: Vec<Medium>,
    #[serde(
        serialize_with = "boundary_shorthand",
        deserialize_with = "name_or_object"
    )]
    #[schema(schema_with = boundary_schema)]
    pub surface: BoundaryDocument,
    #[serde(
        serialize_with = "boundary_shorthand",
        deserialize_with = "name_or_object"
    )]
    #[schema(schema_with = boundary_schema)]
    pub bottom: BoundaryDocument,
    pub source: Source,
    pub receivers: Receivers,
    pub bellhop: Options,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    Bellhop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    N2Linear,
    CLinear,
    Pchip,
    Spline,
    Quadrilateral,
    AnalyticMunk,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Attenuation {
    /// Unit of every `ap`/`as` value; default `db_per_wavelength`.
    #[serde(default, skip_serializing_if = "is_default")]
    pub unit: AttenuationUnitDocument,
    #[serde(
        default,
        skip_serializing_if = "is_default",
        serialize_with = "volume_shorthand",
        deserialize_with = "name_or_object"
    )]
    #[schema(schema_with = volume_schema)]
    pub volume: VolumeAttenuationDocument,
}

impl Attenuation {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttenuationUnitDocument {
    NepersPerMeter,
    DbPerMeterKhz,
    DbPerMeter,
    #[default]
    DbPerWavelength,
    QualityFactor,
    LossParameter,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VolumeAttenuationDocument {
    #[default]
    None,
    Thorp,
    FrancoisGarrison {
        /// °C.
        temperature: f64,
        /// psu.
        salinity: f64,
        ph: f64,
        /// Mean depth, m.
        depth: f64,
    },
    Biological {
        /// Rows of `[top, bottom, resonance frequency, Q, a0]`.
        layers: Vec<[f64; 5]>,
    },
}

/// One medium: the depth of its lower interface and its profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Medium {
    /// Depth of the medium's lower interface, m.
    pub to: f64,
    /// Rows of `[depth, cp, cs, rho, ap, as]`. Trailing values may be left
    /// out: the first row defaults to `cs = 0, rho = 1, ap = as = 0` and later
    /// rows repeat the row above, as a `/` does in the `.env`. Omitted for
    /// `analytic_munk`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ssp: Vec<Vec<f64>>,
    /// The `.env` mesh count NMESH; BELLHOP does not use it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub mesh: i32,
    /// Sound speed versus range (the `.ssp` file), for `quadrilateral`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_dependent: Option<RangeDependentDocument>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeDependentDocument {
    pub ranges: Grid,
    /// `c[depth][range]`, on the depths of the `ssp` rows.
    pub c: Vec<Vec<f64>>,
}

/// A surface or bottom. `"vacuum"` and `"rigid"` may be written as names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoundaryDocument {
    Vacuum {
        #[serde(default, skip_serializing_if = "is_zero32")]
        roughness: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shape: Option<ShapeDocument>,
    },
    Rigid {
        #[serde(default, skip_serializing_if = "is_zero32")]
        roughness: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shape: Option<ShapeDocument>,
    },
    Halfspace {
        #[serde(default, skip_serializing_if = "is_zero32")]
        roughness: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shape: Option<ShapeDocument>,
        /// Defaults to the boundary depth.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth: Option<f64>,
        cp: f64,
        #[serde(default, skip_serializing_if = "is_zero")]
        cs: f64,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        rho: f64,
        #[serde(default, skip_serializing_if = "is_zero")]
        ap: f64,
        #[serde(default, rename = "as", skip_serializing_if = "is_zero")]
        as_: f64,
    },
    GrainSize {
        #[serde(default, skip_serializing_if = "is_zero32")]
        roughness: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shape: Option<ShapeDocument>,
        /// Defaults to the boundary depth.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth: Option<f64>,
        phi: f64,
    },
    /// A reflection-coefficient table (`.trc` / `.brc`).
    Reflection {
        #[serde(default, skip_serializing_if = "is_zero32")]
        roughness: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shape: Option<ShapeDocument>,
        /// Rows of `[angle (deg), magnitude, phase (deg)]`.
        table: Vec<[f64; 3]>,
    },
    /// A precalculated internal reflection table (`.irc`), bottom only.
    Precalculated {
        #[serde(default, skip_serializing_if = "is_zero32")]
        roughness: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shape: Option<ShapeDocument>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        frequency: f64,
        /// Rows of `[k², Re f, Im f, Re g, Im g, power]`.
        table: Vec<[f64; 6]>,
    },
}

/// Boundary range dependence (the `.ati` / `.bty` file).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeDocument {
    #[serde(default, skip_serializing_if = "is_default")]
    pub interpolation: ShapeInterpolation,
    /// Rows of `[range, depth]`, or `[range, depth, cp, cs, rho, ap, as]`
    /// for a bottom whose material changes with range.
    pub points: Vec<Vec<f64>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShapeInterpolation {
    #[default]
    Linear,
    Curvilinear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub depth: Grid,
    #[serde(default, skip_serializing_if = "is_default")]
    pub geometry: SourceGeometryDocument,
    /// Rows of `[angle (deg), level (dB)]` (the `.sbp` file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<Vec<[f64; 2]>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceGeometryDocument {
    #[default]
    Point,
    Line,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Receivers {
    pub depth: Grid,
    pub range: Grid,
    /// `irregular` pairs each depth with one range.
    #[serde(default, skip_serializing_if = "is_default")]
    pub grid: ReceiverGridDocument,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiverGridDocument {
    #[default]
    Rectilinear,
    Irregular,
}

/// Run and beam-tracing options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub run: RunKindDocument,
    /// Required for every run except `rays`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beam: Option<BeamFamilyDocument>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub beam_shift: bool,
    /// Launch angles, degrees. Without `count`, BELLHOP's automatic count.
    pub angles: Grid,
    /// One-based index of a single angle to trace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_angle: Option<usize>,
    /// Step size, m; 0 chooses automatically.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub step: f64,
    #[serde(rename = "box")]
    pub limits: BoxDocument,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cerveny: Option<CervenyDocument>,
}

/// Rays stop when they leave this box, m.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BoxDocument {
    pub depth: f64,
    pub range: f64,
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CervenyDocument {
    pub width: BeamWidthDocument,
    pub curvature: CurvatureDocument,
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
pub enum CurvatureDocument {
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

/// Parses and validates a version 2 document.
pub(super) fn load(bytes: &[u8], source_path: &Path) -> Result<LoadOutcome<Case>, DocumentError> {
    let document: CaseDocument = super::decode(bytes, source_path)?;
    document.into_case(source_path)
}

/// Collects conversion problems as diagnostics at JSON paths.
struct Problems<'a> {
    path: &'a Path,
    report: DiagnosticReport,
}

impl Problems<'_> {
    fn push(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.report.push(error(self.path, "BH0201", message, field));
    }

    fn grid(&mut self, grid: &Grid, automatic: Option<usize>, field: &str) -> Vec<f64> {
        grid.expand(automatic).unwrap_or_else(|message| {
            self.push(field, message);
            Vec::new()
        })
    }
}

impl CaseDocument {
    #[allow(clippy::too_many_lines)]
    fn into_case(self, source_path: &Path) -> Result<LoadOutcome<Case>, DocumentError> {
        let mut problems = Problems {
            path: source_path,
            report: DiagnosticReport::default(),
        };
        if self.media.len() != 1 {
            problems.push(
                "media",
                "BELLHOP needs exactly one medium, the water column",
            );
        }
        let medium = self.media.into_iter().next().unwrap_or(Medium {
            to: f64::NAN,
            ssp: Vec::new(),
            mesh: 0,
            range_dependent: None,
        });
        let points = sound_speed_points(&medium.ssp, &mut problems);
        let top_depth_m = points.first().map_or(0.0, |point| point.depth_m);
        let bottom_depth_m = medium.to;

        let (top_boundary, altimetry, top_reflection, top_internal) =
            import_boundary(self.surface, top_depth_m, "surface", &mut problems);
        if top_internal.is_some() {
            problems.push("surface", "`precalculated` is a bottom-only condition");
        }
        let (bottom_boundary, bathymetry, bottom_reflection, internal_reflection) =
            import_boundary(self.bottom, bottom_depth_m, "bottom", &mut problems);

        let source_depths_m = problems.grid(&self.source.depth, None, "source.depth");
        let receiver_depths_m = problems.grid(&self.receivers.depth, None, "receivers.depth");
        let receiver_ranges_m = problems.grid(&self.receivers.range, None, "receivers.range");
        let run_kind = import_run_kind(self.bellhop.run);
        let automatic = automatic_launch_count(
            run_kind,
            self.frequency,
            bottom_depth_m - top_depth_m,
            receiver_ranges_m.last().copied().unwrap_or(0.0),
        );
        let mut launch_angles_degrees =
            problems.grid(&self.bellhop.angles, Some(automatic), "bellhop.angles");
        if matches!(self.bellhop.angles, Grid::Generated { .. }) {
            drop_repeated_full_circle(&mut launch_angles_degrees);
        }
        let range_dependent_sound_speed =
            medium
                .range_dependent
                .map(|field| RangeDependentSoundSpeed {
                    ranges_m: problems.grid(&field.ranges, None, "media[0].range_dependent.ranges"),
                    depths_m: points.iter().map(|point| point.depth_m).collect(),
                    speeds_mps: field.c,
                });
        let source_beam_pattern = self.source.pattern.map(|rows| SourceBeamPattern {
            points: rows
                .into_iter()
                .map(|[angle_degrees, level_db]| SourceBeamPatternPoint {
                    angle_degrees,
                    level_db,
                    amplitude: 10.0_f64.powf(level_db / 20.0),
                })
                .collect(),
        });
        if !problems.report.diagnostics().is_empty() {
            return Err(DocumentError::semantic(problems.report));
        }

        let definition = CaseDefinition {
            environment: EnvironmentCase {
                source_path: source_path.to_path_buf(),
                title: self.title,
                frequency_hz: self.frequency,
                top_options: TopOptions {
                    legacy: String::new(),
                    interpolation: import_interpolation(self.interpolation),
                    attenuation_unit: import_attenuation_unit(self.attenuation.unit),
                    volume_attenuation: import_volume(self.attenuation.volume),
                    has_altimetry: altimetry.is_some(),
                    development_options: self.bellhop.selected_angle.is_some(),
                },
                sound_speed: SoundSpeedInput {
                    nominal_point_count: medium.mesh,
                    surface_roughness_m: top_boundary.roughness_m,
                    top_depth_m,
                    bottom_depth_m,
                    points,
                },
                top_boundary,
                bottom_boundary,
                positions: Positions {
                    source_depths_m,
                    receiver_depths_m,
                    receiver_ranges_m,
                },
                run: RunOptions {
                    legacy: String::new(),
                    kind: run_kind,
                    arrival_encoding: (run_kind == RunKind::Arrivals)
                        .then_some(LegacyArrivalEncoding::Ascii),
                    beam_family: self.bellhop.beam.map(import_beam_family),
                    has_source_beam_pattern: source_beam_pattern.is_some(),
                    source_geometry: import_source_geometry(self.source.geometry),
                    receiver_grid: import_receiver_grid(self.receivers.grid),
                    beam_shift: self.bellhop.beam_shift,
                },
                trace: TraceOptions {
                    launch_angles_degrees,
                    selected_launch_angle: self.bellhop.selected_angle,
                    step_m: self.bellhop.step,
                    max_depth_m: self.bellhop.limits.depth,
                    max_range_m: self.bellhop.limits.range,
                    cerveny: self.bellhop.cerveny.map(import_cerveny),
                },
            },
            range_dependent_sound_speed,
            altimetry,
            bathymetry,
            bottom_reflection,
            top_reflection,
            internal_reflection,
            source_beam_pattern,
        };
        Case::from_definition_with(definition, &HashMap::new(), DiagnosticReport::default())
            .map(|mut outcome| {
                for warning in &mut outcome.warnings {
                    relocate(warning);
                }
                outcome
            })
            .map_err(|mut report| {
                for diagnostic in report.diagnostics_mut() {
                    relocate(diagnostic);
                }
                DocumentError::semantic(report)
            })
    }
}

impl TryFrom<&Case> for CaseDocument {
    type Error = DiagnosticReport;

    #[allow(clippy::too_many_lines)]
    fn try_from(case: &Case) -> Result<Self, Self::Error> {
        let environment = &case.environment;
        let path = &environment.source_path;
        let sound_speed = &environment.sound_speed;
        let derived_top = sound_speed
            .points
            .first()
            .map_or(0.0, |point| point.depth_m);
        if derived_top.to_bits() != sound_speed.top_depth_m.to_bits() {
            return Err(DiagnosticReport::from_diagnostic(error(
                path,
                "BH0201",
                "the top depth must be the first sound-speed depth",
                "sound_speed.top_depth_m",
            )));
        }
        let surface = export_boundary(
            &environment.top_boundary,
            case.altimetry.as_ref(),
            case.top_reflection.as_ref(),
            None,
            sound_speed.top_depth_m,
            "surface",
            path,
        )?;
        let bottom = export_boundary(
            &environment.bottom_boundary,
            case.bathymetry.as_ref(),
            case.bottom_reflection.as_ref(),
            case.internal_reflection.as_ref(),
            sound_speed.bottom_depth_m,
            "bottom",
            path,
        )?;
        let range_dependent = match &case.range_dependent_sound_speed {
            None => None,
            Some(field) => {
                let depths: Vec<_> = sound_speed.points.iter().map(|p| p.depth_m).collect();
                if !same_bits(&depths, &field.depths_m) {
                    return Err(DiagnosticReport::from_diagnostic(error(
                        path,
                        "BH0201",
                        "range-dependent sound-speed depths must be the profile depths",
                        "sound_speed.range_dependent.depths_m",
                    )));
                }
                Some(RangeDependentDocument {
                    ranges: Grid::describe(&field.ranges_m),
                    c: field.speeds_mps.clone(),
                })
            }
        };
        let trace = &environment.trace;
        let run = &environment.run;
        Ok(Self {
            pelagic: super::SCHEMA_VERSION,
            model: Model::Bellhop,
            title: environment.title.clone(),
            frequency: environment.frequency_hz,
            interpolation: export_interpolation(environment.top_options.interpolation),
            attenuation: Attenuation {
                unit: export_attenuation_unit(environment.top_options.attenuation_unit),
                volume: export_volume(&environment.top_options.volume_attenuation),
            },
            media: vec![Medium {
                to: sound_speed.bottom_depth_m,
                ssp: sound_speed_rows(&sound_speed.points),
                mesh: sound_speed.nominal_point_count,
                range_dependent,
            }],
            surface,
            bottom,
            source: Source {
                depth: Grid::describe(&environment.positions.source_depths_m),
                geometry: export_source_geometry(run.source_geometry),
                pattern: case.source_beam_pattern.as_ref().map(|pattern| {
                    pattern
                        .points
                        .iter()
                        .map(|point| [point.angle_degrees, point.level_db])
                        .collect()
                }),
            },
            receivers: Receivers {
                depth: Grid::describe(&environment.positions.receiver_depths_m),
                range: Grid::describe(&environment.positions.receiver_ranges_m),
                grid: export_receiver_grid(run.receiver_grid),
            },
            bellhop: Options {
                run: export_run_kind(run.kind),
                beam: run.beam_family.map(export_beam_family),
                beam_shift: run.beam_shift,
                angles: describe_angles(&trace.launch_angles_degrees),
                selected_angle: trace.selected_launch_angle,
                step: trace.step_m,
                limits: BoxDocument {
                    depth: trace.max_depth_m,
                    range: trace.max_range_m,
                },
                cerveny: trace.cerveny.as_ref().map(export_cerveny),
            },
        })
    }
}

/// Expands `.env`-style rows, inheriting omitted trailing values.
fn sound_speed_points(rows: &[Vec<f64>], problems: &mut Problems<'_>) -> Vec<SoundSpeedPoint> {
    let mut current = ROW_DEFAULTS;
    let mut points = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let minimum = if index == 0 { 2 } else { 1 };
        if !(minimum..=6).contains(&row.len()) {
            problems.push(
                format!("media[0].ssp[{index}]"),
                format!("expected {minimum} to 6 values: depth, cp, cs, rho, ap, as"),
            );
            continue;
        }
        current[..row.len()].copy_from_slice(row);
        points.push(SoundSpeedPoint {
            depth_m: current[0],
            compressional_speed_mps: current[1],
            shear_speed_mps: current[2],
            density_g_cm3: current[3],
            compressional_attenuation: current[4],
            shear_attenuation: current[5],
        });
    }
    points
}

/// Equal-width rows, dropping trailing columns that only hold defaults.
fn sound_speed_rows(points: &[SoundSpeedPoint]) -> Vec<Vec<f64>> {
    let rows: Vec<[f64; 6]> = points
        .iter()
        .map(|point| {
            [
                point.depth_m,
                point.compressional_speed_mps,
                point.shear_speed_mps,
                point.density_g_cm3,
                point.compressional_attenuation,
                point.shear_attenuation,
            ]
        })
        .collect();
    let width = trimmed_width(&rows, &ROW_DEFAULTS, 2);
    rows.iter().map(|row| row[..width].to_vec()).collect()
}

/// The narrowest width, at least `minimum`, keeping every non-default value.
fn trimmed_width<const N: usize>(rows: &[[f64; N]], defaults: &[f64; N], minimum: usize) -> usize {
    (minimum..N)
        .rev()
        .find(|&column| {
            rows.iter()
                .any(|row| row[column].to_bits() != defaults[column].to_bits())
        })
        .map_or(minimum, |column| column + 1)
}

type ImportedBoundary = (
    Boundary,
    Option<BoundaryShape>,
    Option<ReflectionCoefficientTable>,
    Option<InternalReflectionCoefficientTable>,
);

#[allow(clippy::too_many_lines)]
fn import_boundary(
    document: BoundaryDocument,
    boundary_depth_m: f64,
    field: &str,
    problems: &mut Problems<'_>,
) -> ImportedBoundary {
    let (roughness_m, shape, condition, reflection, internal) = match document {
        BoundaryDocument::Vacuum { roughness, shape } => {
            (roughness, shape, BoundaryCondition::Vacuum, None, None)
        }
        BoundaryDocument::Rigid { roughness, shape } => {
            (roughness, shape, BoundaryCondition::Rigid, None, None)
        }
        BoundaryDocument::Halfspace {
            roughness,
            shape,
            depth,
            cp,
            cs,
            rho,
            ap,
            as_,
        } => (
            roughness,
            shape,
            BoundaryCondition::AcoustoElastic(HalfSpace {
                depth_m: depth.unwrap_or(boundary_depth_m),
                compressional_speed_mps: cp,
                shear_speed_mps: cs,
                density_g_cm3: rho,
                compressional_attenuation: ap,
                shear_attenuation: as_,
            }),
            None,
            None,
        ),
        BoundaryDocument::GrainSize {
            roughness,
            shape,
            depth,
            phi,
        } => (
            roughness,
            shape,
            BoundaryCondition::GrainSize {
                depth_m: depth.unwrap_or(boundary_depth_m),
                phi,
            },
            None,
            None,
        ),
        BoundaryDocument::Reflection {
            roughness,
            shape,
            table,
        } => (
            roughness,
            shape,
            BoundaryCondition::ReflectionCoefficientFile,
            Some(ReflectionCoefficientTable {
                points: table
                    .into_iter()
                    .map(
                        |[angle_degrees, magnitude, phase_degrees]| ReflectionCoefficientPoint {
                            angle_degrees,
                            magnitude,
                            phase_radians: phase_degrees.to_radians(),
                        },
                    )
                    .collect(),
            }),
            None,
        ),
        BoundaryDocument::Precalculated {
            roughness,
            shape,
            title,
            frequency,
            table,
        } => {
            let points = table
                .into_iter()
                .enumerate()
                .filter_map(|(index, [k2, f_re, f_im, g_re, g_im, power])| {
                    #[allow(clippy::cast_possible_truncation, clippy::float_cmp)]
                    let decimal_power = power as i32;
                    #[allow(clippy::float_cmp)]
                    if f64::from(decimal_power) != power {
                        problems.push(
                            format!("{field}.table[{index}]"),
                            "the power must be an integer",
                        );
                        return None;
                    }
                    Some(InternalReflectionCoefficientPoint {
                        horizontal_wavenumber_squared: k2,
                        f: Complex64::new(f_re, f_im),
                        g: Complex64::new(g_re, g_im),
                        decimal_power,
                    })
                })
                .collect();
            (
                roughness,
                shape,
                BoundaryCondition::PrecalculatedReflectionCoefficient,
                None,
                Some(InternalReflectionCoefficientTable {
                    title,
                    frequency_hz: frequency,
                    points,
                }),
            )
        }
    };
    let shape = shape.map(|shape| import_shape(&shape, field, problems));
    (
        Boundary {
            legacy_options: String::new(),
            condition,
            roughness_m,
            has_shape_file: shape.is_some(),
        },
        shape,
        reflection,
        internal,
    )
}

fn import_shape(shape: &ShapeDocument, field: &str, problems: &mut Problems<'_>) -> BoundaryShape {
    let width = shape.points.first().map_or(2, Vec::len);
    let points = shape
        .points
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            if row.len() != width || !matches!(width, 2 | 7) {
                problems.push(
                    format!("{field}.shape.points[{index}]"),
                    "expected equal rows of [range, depth] or [range, depth, cp, cs, rho, ap, as]",
                );
                return None;
            }
            Some(BoundaryShapePoint {
                range_m: row[0],
                depth_m: row[1],
                material: (width == 7).then(|| BoundaryMaterial {
                    compressional_speed_mps: row[2],
                    shear_speed_mps: row[3],
                    density_g_cm3: row[4],
                    compressional_attenuation: row[5],
                    shear_attenuation: row[6],
                }),
            })
        })
        .collect();
    BoundaryShape {
        interpolation: match shape.interpolation {
            ShapeInterpolation::Linear => BoundaryInterpolation::PiecewiseLinear,
            ShapeInterpolation::Curvilinear => BoundaryInterpolation::Curvilinear,
        },
        points,
    }
}

#[allow(clippy::too_many_lines)]
fn export_boundary(
    boundary: &Boundary,
    shape: Option<&BoundaryShape>,
    reflection: Option<&ReflectionCoefficientTable>,
    internal: Option<&InternalReflectionCoefficientTable>,
    boundary_depth_m: f64,
    field: &str,
    path: &Path,
) -> Result<BoundaryDocument, DiagnosticReport> {
    let fail =
        |code, message: &str| DiagnosticReport::from_diagnostic(error(path, code, message, field));
    let roughness = boundary.roughness_m;
    let shape = shape.map(export_shape);
    let depth = |depth_m: f64| (depth_m.to_bits() != boundary_depth_m.to_bits()).then_some(depth_m);
    Ok(match &boundary.condition {
        BoundaryCondition::Vacuum => BoundaryDocument::Vacuum { roughness, shape },
        BoundaryCondition::Rigid => BoundaryDocument::Rigid { roughness, shape },
        BoundaryCondition::AcoustoElastic(half_space) => BoundaryDocument::Halfspace {
            roughness,
            shape,
            depth: depth(half_space.depth_m),
            cp: half_space.compressional_speed_mps,
            cs: half_space.shear_speed_mps,
            rho: half_space.density_g_cm3,
            ap: half_space.compressional_attenuation,
            as_: half_space.shear_attenuation,
        },
        BoundaryCondition::GrainSize { depth_m, phi } => BoundaryDocument::GrainSize {
            roughness,
            shape,
            depth: depth(*depth_m),
            phi: *phi,
        },
        BoundaryCondition::ReflectionCoefficientFile => BoundaryDocument::Reflection {
            roughness,
            shape,
            table: reflection
                .ok_or_else(|| fail("BH0201", "loaded boundary data is missing"))?
                .points
                .iter()
                .map(|point| {
                    [
                        point.angle_degrees,
                        point.magnitude,
                        point.phase_radians.to_degrees(),
                    ]
                })
                .collect(),
        },
        BoundaryCondition::PrecalculatedReflectionCoefficient => {
            let table = internal.ok_or_else(|| {
                if field == "surface" {
                    fail("BH0202", "top precalculated reflection cannot be exported")
                } else {
                    fail("BH0201", "loaded boundary data is missing")
                }
            })?;
            BoundaryDocument::Precalculated {
                roughness,
                shape,
                title: table.title.clone(),
                frequency: table.frequency_hz,
                table: table
                    .points
                    .iter()
                    .map(|point| {
                        [
                            point.horizontal_wavenumber_squared,
                            point.f.re,
                            point.f.im,
                            point.g.re,
                            point.g.im,
                            f64::from(point.decimal_power),
                        ]
                    })
                    .collect(),
            }
        }
        BoundaryCondition::WriteReflectionCoefficient => {
            return Err(fail(
                "BH0301",
                "W reflection-table generation has no modern JSON representation",
            ));
        }
    })
}

fn export_shape(shape: &BoundaryShape) -> ShapeDocument {
    ShapeDocument {
        interpolation: match shape.interpolation {
            BoundaryInterpolation::PiecewiseLinear => ShapeInterpolation::Linear,
            BoundaryInterpolation::Curvilinear => ShapeInterpolation::Curvilinear,
        },
        points: shape
            .points
            .iter()
            .map(|point| {
                let mut row = vec![point.range_m, point.depth_m];
                if let Some(material) = &point.material {
                    row.extend([
                        material.compressional_speed_mps,
                        material.shear_speed_mps,
                        material.density_g_cm3,
                        material.compressional_attenuation,
                        material.shear_attenuation,
                    ]);
                }
                row
            })
            .collect(),
    }
}

fn import_volume(volume: VolumeAttenuationDocument) -> VolumeAttenuation {
    match volume {
        VolumeAttenuationDocument::None => VolumeAttenuation::None,
        VolumeAttenuationDocument::Thorp => VolumeAttenuation::Thorp,
        VolumeAttenuationDocument::FrancoisGarrison {
            temperature,
            salinity,
            ph,
            depth,
        } => VolumeAttenuation::FrancoisGarrison {
            temperature_c: temperature,
            salinity_psu: salinity,
            ph,
            mean_depth_m: depth,
        },
        VolumeAttenuationDocument::Biological { layers } => VolumeAttenuation::Biological {
            layers: layers
                .into_iter()
                .map(
                    |[top, bottom, frequency, quality, attenuation]| BiologicalLayer {
                        top_depth_m: top,
                        bottom_depth_m: bottom,
                        resonance_frequency_hz: frequency,
                        quality_factor: quality,
                        attenuation,
                    },
                )
                .collect(),
        },
    }
}

fn export_volume(volume: &VolumeAttenuation) -> VolumeAttenuationDocument {
    match volume {
        VolumeAttenuation::None => VolumeAttenuationDocument::None,
        VolumeAttenuation::Thorp => VolumeAttenuationDocument::Thorp,
        VolumeAttenuation::FrancoisGarrison {
            temperature_c,
            salinity_psu,
            ph,
            mean_depth_m,
        } => VolumeAttenuationDocument::FrancoisGarrison {
            temperature: *temperature_c,
            salinity: *salinity_psu,
            ph: *ph,
            depth: *mean_depth_m,
        },
        VolumeAttenuation::Biological { layers } => VolumeAttenuationDocument::Biological {
            layers: layers
                .iter()
                .map(|layer| {
                    [
                        layer.top_depth_m,
                        layer.bottom_depth_m,
                        layer.resonance_frequency_hz,
                        layer.quality_factor,
                        layer.attenuation,
                    ]
                })
                .collect(),
        },
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

fn import_interpolation(value: Interpolation) -> SspInterpolation {
    match value {
        Interpolation::N2Linear => SspInterpolation::N2Linear,
        Interpolation::CLinear => SspInterpolation::CLinear,
        Interpolation::Pchip => SspInterpolation::Pchip,
        Interpolation::Spline => SspInterpolation::CubicSpline,
        Interpolation::Quadrilateral => SspInterpolation::Quadrilateral,
        Interpolation::AnalyticMunk => SspInterpolation::AnalyticMunk,
    }
}

fn export_interpolation(value: SspInterpolation) -> Interpolation {
    match value {
        SspInterpolation::N2Linear => Interpolation::N2Linear,
        SspInterpolation::CLinear => Interpolation::CLinear,
        SspInterpolation::Pchip => Interpolation::Pchip,
        SspInterpolation::CubicSpline => Interpolation::Spline,
        SspInterpolation::Quadrilateral => Interpolation::Quadrilateral,
        SspInterpolation::AnalyticMunk => Interpolation::AnalyticMunk,
    }
}

enum_map!(import_beam_width, export_beam_width, BeamWidthDocument, BeamWidth, {
    SpaceFilling, Minimum, Wkb
});
enum_map!(import_curvature, export_curvature, CurvatureDocument, CurvatureCondition, {
    Double, Standard, Zero
});
enum_map!(import_beam_component, export_beam_component, BeamComponentDocument, BeamComponent, {
    Pressure, Vertical, Horizontal, Displacement
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

/// The `.env` NBEAMS = 0 count, from the same quantities the reader uses.
fn automatic_launch_count(
    run_kind: RunKind,
    frequency_hz: f64,
    water_depth_m: f64,
    maximum_receiver_range_m: f64,
) -> usize {
    let count = crate::legacy::automatic_launch_count(
        run_kind,
        frequency_hz,
        water_depth_m,
        maximum_receiver_range_m,
    );
    usize::try_from(count).unwrap_or(0)
}

/// Describes launch angles, keeping a full circle explicit: as a generated grid
/// it would lose its repeated last angle when loaded.
fn describe_angles(angles: &[f64]) -> Grid {
    let grid = Grid::describe(angles);
    if let Ok(mut expanded) = grid.expand(None)
        && matches!(grid, Grid::Generated { .. })
    {
        drop_repeated_full_circle(&mut expanded);
        if expanded.len() != angles.len() {
            return Grid::Values(angles.to_vec());
        }
    }
    grid
}

/// A fan that sweeps a full circle repeats its first angle; the reader drops it.
fn drop_repeated_full_circle(angles: &mut Vec<f64>) {
    if let [first, .., last] = angles[..] {
        let sweep = last - first;
        if sweep.rem_euclid(360.0).abs() < 10.0 * f64::EPSILON * 360.0 {
            angles.pop();
        }
    }
}

/// Maps a validated-model field name to its version 2 JSON path.
fn relocate(diagnostic: &mut crate::Diagnostic) {
    const PATHS: &[(&str, &str)] = &[
        ("frequency_hz", "frequency"),
        ("frequency", "frequency"),
        ("sound_speed.bottom_depth_m", "media[0].to"),
        ("sound_speed.points", "media[0].ssp"),
        ("sound_speed.range_dependent.depths_m", "media[0].ssp"),
        (
            "sound_speed.range_dependent.speeds_mps",
            "media[0].range_dependent.c",
        ),
        ("sound_speed.range_dependent", "media[0].range_dependent"),
        ("sound_speed.volume_attenuation", "attenuation.volume"),
        ("positions.source_depths_m", "source.depth"),
        ("positions.receiver_depths_m", "receivers.depth"),
        ("positions.receiver_ranges_m", "receivers.range"),
        ("positions", "receivers"),
        ("run.beam_family", "bellhop.beam"),
        ("run.arrival_encoding", "bellhop.run"),
        ("trace.launch_angles_degrees", "bellhop.angles"),
        ("trace.selected_launch_angle", "bellhop.selected_angle"),
        ("trace.step_m", "bellhop.step"),
        ("trace.max_depth_m", "bellhop.box.depth"),
        ("trace.max_range_m", "bellhop.box.range"),
        ("trace.cerveny", "bellhop.cerveny"),
        ("source_beam_pattern", "source.pattern"),
        ("internal_reflection.frequency", "bottom.frequency"),
        (
            "top_boundary.condition.table.frequency_hz",
            "surface.frequency",
        ),
        (
            "bottom_boundary.condition.table.frequency_hz",
            "bottom.frequency",
        ),
        ("top_boundary.condition.table", "surface.table"),
        ("bottom_boundary.condition.table", "bottom.table"),
        ("top_boundary.condition", "surface"),
        ("bottom_boundary.condition", "bottom"),
        ("top_boundary.roughness_m", "surface.roughness"),
        ("bottom_boundary.roughness_m", "bottom.roughness"),
        ("top_boundary", "surface"),
        ("bottom_boundary", "bottom"),
    ];
    let Some(field) = diagnostic.field.as_deref() else {
        return;
    };
    // Longest matching prefix wins; the table lists specific paths first.
    if let Some((model, json)) = PATHS.iter().find(|(model, _)| {
        field == *model
            || field
                .strip_prefix(model)
                .is_some_and(|rest| rest.starts_with(['.', '[']))
    }) {
        diagnostic.field = Some(format!("{json}{}", &field[model.len()..]));
    }
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

// serde `skip_serializing_if` passes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &f64) -> bool {
    value.to_bits() == 0
}

// serde `skip_serializing_if` passes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero32(value: &f32) -> bool {
    value.to_bits() == 0
}

// serde `skip_serializing_if` passes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_one(value: &f64) -> bool {
    value.to_bits() == 1.0_f64.to_bits()
}

fn one() -> f64 {
    1.0
}

/// A boundary: `"vacuum"`, `"rigid"`, or a tagged object.
fn boundary_schema() -> RefOr<Schema> {
    name_or_object_schema(&["vacuum", "rigid"], "BoundaryDocument")
}

/// Volume attenuation: `"none"`, `"thorp"`, or a tagged object.
fn volume_schema() -> RefOr<Schema> {
    name_or_object_schema(&["none", "thorp"], "VolumeAttenuationDocument")
}

/// The `OpenAPI` form of [`name_or_object`]: a fieldless variant's name, or the object.
fn name_or_object_schema(names: &[&str], object: &str) -> RefOr<Schema> {
    OneOfBuilder::new()
        .item(
            ObjectBuilder::new()
                .schema_type(Type::String)
                .enum_values(Some(names.iter().copied())),
        )
        .item(Ref::from_schema_name(object))
        .into()
}

/// Accepts `"name"` for `{"type": "name"}`, or the full tagged object.
fn name_or_object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    struct NameOrObject<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for NameOrObject<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a name, or an object with a \"type\"")
        }

        fn visit_str<E: de::Error>(self, name: &str) -> Result<T, E> {
            T::deserialize(de::value::MapDeserializer::<_, E>::new(std::iter::once((
                "type", name,
            ))))
        }

        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(de::value::MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_any(NameOrObject(PhantomData))
}

fn boundary_shorthand<S: Serializer>(
    boundary: &BoundaryDocument,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match boundary {
        BoundaryDocument::Vacuum {
            roughness,
            shape: None,
        } if is_zero32(roughness) => serializer.serialize_str("vacuum"),
        BoundaryDocument::Rigid {
            roughness,
            shape: None,
        } if is_zero32(roughness) => serializer.serialize_str("rigid"),
        other => other.serialize(serializer),
    }
}

fn volume_shorthand<S: Serializer>(
    volume: &VolumeAttenuationDocument,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match volume {
        VolumeAttenuationDocument::None => serializer.serialize_str("none"),
        VolumeAttenuationDocument::Thorp => serializer.serialize_str("thorp"),
        other => other.serialize(serializer),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::CaseDocument;
    use crate::json::{DocumentErrorKind, export_case_document, load_case_document};

    fn golden() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden")
    }

    fn legacy_cases() -> Vec<(PathBuf, crate::Case)> {
        let mut cases: Vec<_> = fs::read_dir(golden())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "env"))
            .map(|path| {
                let case = crate::legacy::load_case(&path).unwrap().value;
                (path, case)
            })
            .collect();
        cases.sort_by(|left, right| left.0.cmp(&right.0));
        assert!(cases.len() >= 20, "expected the curated golden matrix");
        cases
    }

    /// The validated case without provenance, which JSON does not carry.
    fn semantics(case: crate::Case) -> crate::CaseDefinition {
        let mut definition = case.into_definition();
        definition.environment.source_path = PathBuf::new();
        definition.environment.top_options.legacy.clear();
        definition.environment.run.legacy.clear();
        definition.environment.top_boundary.legacy_options.clear();
        definition
            .environment
            .bottom_boundary
            .legacy_options
            .clear();
        definition
    }

    #[test]
    fn every_golden_case_survives_json_exactly() {
        for (path, case) in legacy_cases() {
            let document = export_case_document(&case).unwrap();
            let encoded = serde_json::to_vec(&document).unwrap();
            let loaded = load_case_document(&encoded)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
                .value;
            assert_eq!(
                export_case_document(&loaded).unwrap(),
                document,
                "{}",
                path.display()
            );
            let expected = semantics(case);
            let mut actual = semantics(loaded);
            // The `.env` may select binary arrivals; JSON always writes ASCII.
            actual.environment.run.arrival_encoding = expected.environment.run.arrival_encoding;
            assert_eq!(actual, expected, "{}", path.display());
        }
    }

    #[test]
    fn hand_written_case_uses_defaults_grids_and_shorthand() {
        let case = load_case_document(
            br#"{
              "pelagic": 2, "model": "bellhop", "frequency": 50, "interpolation": "c_linear",
              "media": [{"to": 100, "ssp": [[0, 1500], [100, 1520]]}],
              "surface": "vacuum",
              "bottom": {"type": "halfspace", "cp": 1600, "rho": 1.8, "ap": 0.5},
              "source": {"depth": 20},
              "receivers": {"depth": {"from": 0, "to": 100, "count": 11},
                            "range": {"from": 0, "to": 10, "count": 101, "unit": "km"}},
              "bellhop": {"run": "coherent", "beam": "geometric_gaussian_cartesian",
                          "angles": {"from": -30, "to": 30, "count": 61},
                          "box": {"depth": 110, "range": 10100}}
            }"#,
        )
        .unwrap()
        .value;
        let environment = &case.environment;
        assert_eq!(environment.sound_speed.points[1].density_g_cm3, 1.0);
        assert_eq!(environment.positions.receiver_ranges_m[100], 10_000.0);
        assert_eq!(environment.trace.launch_angles_degrees.len(), 61);
        let crate::model::BoundaryCondition::AcoustoElastic(bottom) =
            &environment.bottom_boundary.condition
        else {
            panic!("expected a half-space");
        };
        assert_eq!((bottom.depth_m, bottom.shear_speed_mps), (100.0, 0.0));
        let exported = serde_json::to_value(export_case_document(&case).unwrap()).unwrap();
        assert_eq!(exported["surface"], "vacuum");
        assert_eq!(
            exported["media"][0]["ssp"],
            serde_json::json!([[0.0, 1500.0], [100.0, 1520.0]])
        );
        assert_eq!(
            exported["receivers"]["range"],
            serde_json::json!({"from": 0.0, "to": 10.0, "count": 101, "unit": "km"})
        );
    }

    #[test]
    fn full_circle_angle_lists_survive_export() {
        let case = crate::legacy::load_case(&golden().join("Field_G.env"))
            .unwrap()
            .value;
        let mut document = export_case_document(&case).unwrap();
        // The reader drops a generated full circle's repeated end; a list keeps it.
        document.bellhop.angles = super::Grid::Values(vec![-180.0, -90.0, 0.0, 90.0, 180.0]);
        for selected in [None, Some(5)] {
            document.bellhop.selected_angle = selected;
            let loaded = load_case_document(&serde_json::to_vec(&document).unwrap())
                .unwrap()
                .value;
            assert_eq!(loaded.environment.trace.launch_angles_degrees.len(), 5);
            let exported = export_case_document(&loaded).unwrap();
            assert_eq!(exported, document);
            let reloaded = load_case_document(&serde_json::to_vec(&exported).unwrap())
                .unwrap()
                .value;
            assert_eq!(
                reloaded.environment.trace, loaded.environment.trace,
                "{selected:?}"
            );
        }
    }

    #[test]
    fn diagnostics_name_json_paths() {
        let case = crate::legacy::load_case(&golden().join("Field_G.env"))
            .unwrap()
            .value;
        let mut document: CaseDocument = export_case_document(&case).unwrap();
        document.bellhop.angles = super::Grid::Values(Vec::new());
        let error = load_case_document(&serde_json::to_vec(&document).unwrap()).unwrap_err();
        assert_eq!(error.kind(), DocumentErrorKind::Semantic);
        assert!(
            error
                .report()
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.field.as_deref() == Some("bellhop.angles")),
            "{error}"
        );

        let mut value = serde_json::to_value(&document).unwrap();
        value["bellhop"]["angels"] = serde_json::json!(1);
        let error = load_case_document(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(error.kind(), DocumentErrorKind::Malformed);

        value["pelagic"] = serde_json::json!(3);
        let error = load_case_document(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(error.report().diagnostics()[0].code, "BH0202");

        value["pelagic"] = serde_json::json!(2);
        value["model"] = serde_json::json!("kraken");
        let error = load_case_document(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(
            error.report().diagnostics()[0].field.as_deref(),
            Some("model")
        );
    }
}

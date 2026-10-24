//! Strict, self-contained KRAKEN JSON input, schema version 2.
//!
//! The document is the `.env`/`.flp` pair with names: one environment, an
//! ordered frequency grid, and optional range-dependent profiles that replace
//! whole top-level keys. Losses keep the `.env` recipe (unit, volume model and
//! power law) and are converted for each solve frequency by the same code as
//! legacy input, so both paths build identical cases.
use std::fmt;
use std::marker::PhantomData;
use std::path::Path;

use num_complex::Complex64;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::material::{Elastic, Fluid, HalfSpace, Materials};
use crate::attenuation::VolumeLoss;
use crate::{
    Boundary, Case, CaseDefinition, Diagnostic, DiagnosticReport, FieldCase, FieldPropagation,
    ImpedancePoint, Interpolation, MAX_MODE_LIMIT, MAX_VECTOR_LENGTH, ModeAddition, ModeSolver,
    ReflectionPoint, SoundSpeedPoint, SourceGeometry, SourcePatternPoint,
};

pub mod grid;

pub use grid::{Grid, GridUnit};

/// The schema version, the document's `"pelagic"` field.
pub const SCHEMA_VERSION: u32 = 2;

/// Defaults for sound-speed row columns 3–6: shear speed, density, P and S loss.
const ROW_DEFAULTS: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// A complete KRAKEN/KRAKENC and FIELD case.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseDocument {
    /// The schema version, 2.
    pub pelagic: u32,
    pub model: Model,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Solve frequencies, Hz, in order; repeats are kept.
    pub frequency: Grid,
    pub interpolation: Interpolation,
    #[serde(default, skip_serializing_if = "is_default")]
    pub attenuation: Attenuation,
    /// Finite media from the surface down: top elastic layers (ice), the
    /// water and further fluids, then elastic sediments.
    pub media: Vec<Medium>,
    #[serde(
        serialize_with = "boundary_shorthand",
        deserialize_with = "name_or_object"
    )]
    pub surface: BoundaryDocument,
    #[serde(
        serialize_with = "boundary_shorthand",
        deserialize_with = "name_or_object"
    )]
    pub bottom: BoundaryDocument,
    pub source: Source,
    pub receivers: Receivers,
    pub kraken: Options,
    /// Further environments for range-dependent FIELD, at increasing ranges
    /// beyond the top-level one at 0 m.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Profile>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    Kraken,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attenuation {
    /// Unit of every `ap`/`as` value; default `db_per_wavelength`.
    #[serde(default, skip_serializing_if = "is_default")]
    pub unit: AttenuationUnit,
    #[serde(
        default,
        skip_serializing_if = "is_default",
        serialize_with = "volume_shorthand",
        deserialize_with = "name_or_object"
    )]
    pub volume: VolumeAttenuation,
    /// Reference frequency of `db_per_meter_power_law`, Hz.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_frequency: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttenuationUnit {
    NepersPerMeter,
    DbPerMeterKhz,
    DbPerMeter,
    /// dB/m scaled by `(f / reference)^beta` up to each medium's transition.
    DbPerMeterPowerLaw,
    #[default]
    DbPerWavelength,
    QualityFactor,
    LossParameter,
}

impl AttenuationUnit {
    fn code(self) -> u8 {
        match self {
            Self::NepersPerMeter => b'N',
            Self::DbPerMeterKhz => b'F',
            Self::DbPerMeter => b'M',
            Self::DbPerMeterPowerLaw => b'm',
            Self::DbPerWavelength => b'W',
            Self::QualityFactor => b'Q',
            Self::LossParameter => b'L',
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            b'N' => Self::NepersPerMeter,
            b'F' => Self::DbPerMeterKhz,
            b'M' => Self::DbPerMeter,
            b'm' => Self::DbPerMeterPowerLaw,
            b'Q' => Self::QualityFactor,
            b'L' => Self::LossParameter,
            _ => Self::DbPerWavelength,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VolumeAttenuation {
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

/// One finite medium: the depth of its lower interface and its profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Medium {
    /// Depth of the lower interface, m.
    pub to: f64,
    /// Rows of `[depth, cp, cs, rho, ap, as]`, from this medium's top to `to`.
    /// The first row defaults to `cs = 0, rho = 1, ap = as = 0`; later rows
    /// repeat the row above, as a `/` does. Omitted for `analytic_munk`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ssp: Vec<Vec<f64>>,
    /// Mesh points; 0 chooses automatically.
    #[serde(default, skip_serializing_if = "is_default")]
    pub mesh: usize,
    /// RMS roughness of this medium's upper interface, m. The first medium's
    /// upper interface is the surface: use `surface.roughness`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub roughness: f64,
    /// `[beta, transition frequency]` for `db_per_meter_power_law`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_law: Option<[f64; 2]>,
}

/// A surface or bottom. `"vacuum"` and `"rigid"` may be written as names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoundaryDocument {
    Vacuum {
        #[serde(default, skip_serializing_if = "is_zero")]
        roughness: f64,
    },
    Rigid {
        #[serde(default, skip_serializing_if = "is_zero")]
        roughness: f64,
    },
    /// A fluid half-space, or elastic when `cs > 0`.
    Halfspace {
        #[serde(default, skip_serializing_if = "is_zero")]
        roughness: f64,
        cp: f64,
        #[serde(default, skip_serializing_if = "is_zero")]
        cs: f64,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        rho: f64,
        #[serde(default, skip_serializing_if = "is_zero")]
        ap: f64,
        #[serde(default, rename = "as", skip_serializing_if = "is_zero")]
        as_: f64,
        /// Bottom `[beta, transition frequency]` for `db_per_meter_power_law`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        power_law: Option<[f64; 2]>,
    },
    /// KRAKENC reflection-coefficient table (`.trc` / `.brc`).
    Reflection {
        #[serde(default, skip_serializing_if = "is_zero")]
        roughness: f64,
        /// Rows of `[angle (deg), magnitude, phase (deg)]`.
        table: Vec<[f64; 3]>,
    },
    /// KRAKENC precalculated impedance table (`.irc`), bottom only.
    Precalculated {
        #[serde(default, skip_serializing_if = "is_zero")]
        roughness: f64,
        frequency: f64,
        /// Rows of `[k², Re f, Im f, Re g, Im g, power]`.
        table: Vec<[f64; 6]>,
    },
}

impl BoundaryDocument {
    fn roughness(&self) -> f64 {
        match self {
            Self::Vacuum { roughness }
            | Self::Rigid { roughness }
            | Self::Halfspace { roughness, .. }
            | Self::Reflection { roughness, .. }
            | Self::Precalculated { roughness, .. } => *roughness,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Depths, m, stored in single precision like the Toolbox.
    pub depth: Grid,
    #[serde(default = "point", skip_serializing_if = "is_point")]
    pub geometry: SourceGeometry,
    /// Rows of `[angle (deg), level (dB)]` (the `.sbp` file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<Vec<[f64; 2]>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receivers {
    /// Depths, m, stored in single precision like the Toolbox.
    pub depth: Grid,
    pub range: Grid,
    /// Horizontal offset of each receiver depth, m; default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Grid>,
}

/// Mode-solver and FIELD options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub solver: ModeSolver,
    /// `[low, high]` phase-speed window, m/s.
    pub phase_speed: [f64; 2],
    /// Largest range of interest, m.
    pub max_range: f64,
    /// Frequency the meshes are sized for, Hz; default each solve frequency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_frequency: Option<f64>,
    /// Depths where modes are sampled; default the source and receiver depths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_depths: Option<Grid>,
    /// Most modes FIELD sums; default all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_limit: Option<usize>,
    #[serde(default = "coherent", skip_serializing_if = "is_coherent")]
    pub addition: ModeAddition,
    /// Required with `profiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub propagation: Option<Propagation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Propagation {
    Adiabatic,
    Coupled,
}

/// An environment at `range`; each key given replaces the top-level one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// m.
    pub range: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpolation: Option<Interpolation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attenuation: Option<Attenuation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<Vec<Medium>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "optional_boundary_shorthand",
        deserialize_with = "optional_name_or_object"
    )]
    pub surface: Option<BoundaryDocument>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "optional_boundary_shorthand",
        deserialize_with = "optional_name_or_object"
    )]
    pub bottom: Option<BoundaryDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kraken: Option<ProfileOptions>,
}

/// Mode-solver options that may change between profiles.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_speed: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_range: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_depths: Option<Grid>,
    /// Replaces the top-level `mesh_frequency`. `null` sizes this profile's
    /// meshes for each solve frequency, as a single-frequency `.env` header does.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub mesh_frequency: Option<Option<f64>>,
}

/// Parse a document using `request.json` as its diagnostic source.
/// # Errors
/// Returns bounded-input, schema or shared case/profile validation diagnostics.
pub fn load_case_document(bytes: &[u8]) -> Result<Vec<FieldCase>, DiagnosticReport> {
    load_case_document_named(bytes, Path::new("request.json"))
}

/// Parse exact bytes into one validated field case per frequency, in order.
/// # Errors
/// Rejects oversized/malformed documents, unsupported versions and invalid cases.
pub fn load_case_document_named(
    bytes: &[u8],
    path: &Path,
) -> Result<Vec<FieldCase>, DiagnosticReport> {
    decode_case_document(bytes, path)?.cases(path)
}

/// Compact JSON for a document, within the 1 MiB input limit so it can always
/// be read back. Writing can lengthen numbers (`1500` becomes `1500.0`), so a
/// document that was read within the limit may still not fit.
/// # Errors
/// Returns `KR0101` when the compact document exceeds 1 MiB.
pub fn encode_case_document(
    document: &CaseDocument,
    path: &Path,
) -> Result<String, DiagnosticReport> {
    let text = serde_json::to_string(document)
        .map_err(|cause| error(path, "KR0103", &cause.to_string(), "json"))?;
    if text.len() as u64 > crate::legacy::MAX_INPUT_BYTES {
        return Err(error(
            path,
            "KR0101",
            "the case's JSON form exceeds 1 MiB, the input limit",
            "json",
        ));
    }
    Ok(text)
}

/// Strictly decode a document without building its cases.
/// # Errors
/// Rejects oversized/malformed documents, unsupported versions and other models.
pub fn decode_case_document(bytes: &[u8], path: &Path) -> Result<CaseDocument, DiagnosticReport> {
    #[derive(Deserialize)]
    struct Probe {
        pelagic: Option<serde_json::Value>,
        model: Option<serde_json::Value>,
    }
    if bytes.len() as u64 > crate::legacy::MAX_INPUT_BYTES {
        return Err(error(path, "KR0101", "JSON input exceeds 1 MiB", "json"));
    }
    let probe: Probe = decode(bytes, path)?;
    match probe.pelagic {
        None => {
            return Err(error(
                path,
                "KR0103",
                "invalid JSON case: missing field `pelagic` (the schema version, 2)",
                "pelagic",
            ));
        }
        Some(version) if version.as_u64() != Some(u64::from(SCHEMA_VERSION)) => {
            return Err(error(
                path,
                "KR0202",
                &format!("unsupported JSON schema version {version}; expected {SCHEMA_VERSION}"),
                "pelagic",
            ));
        }
        Some(_) => {}
    }
    if let Some(model) = probe.model.filter(|model| model != "kraken") {
        return Err(error(
            path,
            "KR0202",
            &format!("this is a {model} case, not a KRAKEN case"),
            "model",
        ));
    }
    decode(bytes, path)
}

fn decode<T: de::DeserializeOwned>(bytes: &[u8], path: &Path) -> Result<T, DiagnosticReport> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = serde_path_to_error::deserialize(&mut decoder).map_err(|cause| {
        DiagnosticReport::one(Diagnostic::new(
            "KR0103",
            format!("invalid JSON case: {cause}"),
            malformed_field(&cause.path().to_string()),
            path,
            cause.inner().line(),
            cause.inner().column(),
        ))
    })?;
    decoder
        .end()
        .map_err(|cause| error(path, "KR0103", &cause.to_string(), "json"))?;
    Ok(value)
}

/// One profile's environment: top-level keys, replaced by a profile's own.
struct Environment<'a> {
    profile: Option<usize>,
    overrides: Option<&'a Profile>,
    title: &'a str,
    interpolation: Interpolation,
    attenuation: &'a Attenuation,
    media: &'a [Medium],
    surface: &'a BoundaryDocument,
    bottom: &'a BoundaryDocument,
    phase_speed: [f64; 2],
    max_range: f64,
    mode_depths: Option<&'a Grid>,
    mesh_frequency: Option<f64>,
}

impl Environment<'_> {
    /// The JSON path of `key`, inside the profile when the profile sets it.
    fn path(&self, key: &str) -> String {
        let overridden = self.overrides.is_some_and(|profile| {
            let options = profile.kraken.as_ref();
            match key.split(['.', '[']).next().unwrap_or(key) {
                "title" => profile.title.is_some(),
                "interpolation" => profile.interpolation.is_some(),
                "attenuation" => profile.attenuation.is_some(),
                "media" => profile.media.is_some(),
                "surface" => profile.surface.is_some(),
                "bottom" => profile.bottom.is_some(),
                "kraken" => match key {
                    "kraken.phase_speed" => options.is_some_and(|o| o.phase_speed.is_some()),
                    "kraken.max_range" => options.is_some_and(|o| o.max_range.is_some()),
                    "kraken.mode_depths" => options.is_some_and(|o| o.mode_depths.is_some()),
                    "kraken.mesh_frequency" => options.is_some_and(|o| o.mesh_frequency.is_some()),
                    _ => false,
                },
                _ => false,
            }
        });
        match self.profile {
            Some(index) if overridden => format!("profiles[{index}].{key}"),
            _ => key.to_owned(),
        }
    }
}

/// Where each validated layer list sits in `media`.
struct Layout {
    top: usize,
    additional: usize,
}

struct Problems<'a> {
    path: &'a Path,
    report: DiagnosticReport,
}

impl Problems<'_> {
    fn push(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.report
            .push(Diagnostic::new("KR0201", message, field, self.path, 1, 1));
    }

    fn check(self) -> Result<(), DiagnosticReport> {
        if self.report.diagnostics().is_empty() {
            Ok(())
        } else {
            Err(self.report)
        }
    }
}

/// FIELD geometry shared by every profile, before the interface snap.
struct Field {
    source_depths: Vec<f64>,
    receiver_depths: Vec<f64>,
    receiver_ranges_m: Vec<f64>,
    receiver_offsets_m: Vec<f64>,
    source_pattern: Vec<SourcePatternPoint>,
}

impl CaseDocument {
    /// Build one validated field case per frequency, in document order.
    /// # Errors
    /// Returns validation diagnostics located at this document's JSON paths.
    #[allow(clippy::too_many_lines)]
    pub fn cases(&self, path: &Path) -> Result<Vec<FieldCase>, DiagnosticReport> {
        let mut problems = Problems {
            path,
            report: DiagnosticReport::default(),
        };
        let frequencies = self.frequency.expand().unwrap_or_else(|message| {
            problems.push("frequency", message);
            Vec::new()
        });
        if !(1..=crate::legacy::MAX_FREQUENCIES).contains(&frequencies.len()) {
            problems.push("frequency", "require 1..=1000 frequencies");
        } else if frequencies.iter().any(|&f| !(f > 0.0 && f.is_finite())) {
            problems.push("frequency", "frequencies must be positive");
        }
        let field = self.field(&mut problems);
        let propagation = match (self.profiles.is_empty(), self.kraken.propagation) {
            (true, None) => Some(FieldPropagation::RangeIndependent),
            (false, Some(Propagation::Adiabatic)) => Some(FieldPropagation::Adiabatic),
            (false, Some(Propagation::Coupled)) => Some(FieldPropagation::Coupled),
            (true, Some(_)) => {
                problems.push("kraken.propagation", "propagation requires `profiles`");
                None
            }
            (false, None) => {
                problems.push(
                    "kraken.propagation",
                    "`profiles` require `adiabatic` or `coupled` propagation",
                );
                None
            }
        };
        problems.check()?;
        let propagation = propagation.unwrap_or(FieldPropagation::RangeIndependent);

        let mut ranges = vec![0.0];
        ranges.extend(self.profiles.iter().map(|profile| profile.range));
        let environments: Vec<_> = std::iter::once(None)
            .chain(self.profiles.iter().enumerate().map(Some))
            .map(|profile| self.environment(profile))
            .collect();
        let mut templates = Vec::with_capacity(environments.len());
        let mut total = 0_usize;
        for environment in &environments {
            let mut problems = Problems {
                path,
                report: DiagnosticReport::default(),
            };
            let built = self.template(environment, &field, &frequencies, &mut problems);
            problems.check()?;
            let Some((template, materials, layout, values)) = built else {
                continue;
            };
            total = total.saturating_add(values.saturating_mul(frequencies.len()));
            if total > crate::case::MAX_SEQUENCE_VALUES {
                return Err(error(
                    path,
                    "KR0201",
                    "profile/frequency cases exceed the cumulative input storage limit",
                    "frequency",
                ));
            }
            templates.push((template, materials, layout));
        }

        let mut cases = Vec::with_capacity(frequencies.len());
        for &frequency in &frequencies {
            let mut profiles = Vec::with_capacity(templates.len());
            for ((template, materials, layout), environment) in templates.iter().zip(&environments)
            {
                let definition = materials.case_definition(template, frequency);
                let case = Case::from_definition(definition)
                    .and_then(|case| {
                        if case.mesh_reference_frequency_hz.is_some() {
                            case.mesh_points_at(1)?;
                        }
                        Ok(case)
                    })
                    .map_err(|report| {
                        relocate(report, path, environment, layout, &frequencies, frequency)
                    })?;
                profiles.push(case);
            }
            let case =
                FieldCase::new(profiles, ranges.clone(), propagation).map_err(|mut report| {
                    for diagnostic in &mut report.diagnostics {
                        diagnostic.path = path.to_path_buf();
                        diagnostic.field = "profiles".into();
                    }
                    report
                })?;
            cases.push(case);
        }
        Ok(cases)
    }

    fn environment<'a>(&'a self, profile: Option<(usize, &'a Profile)>) -> Environment<'a> {
        let overrides = profile.map(|(_, profile)| profile);
        let options = overrides.and_then(|profile| profile.kraken.as_ref());
        Environment {
            profile: profile.map(|(index, _)| index),
            overrides,
            title: overrides
                .and_then(|profile| profile.title.as_deref())
                .unwrap_or(&self.title),
            interpolation: overrides
                .and_then(|profile| profile.interpolation)
                .unwrap_or(self.interpolation),
            attenuation: overrides
                .and_then(|profile| profile.attenuation.as_ref())
                .unwrap_or(&self.attenuation),
            media: overrides
                .and_then(|profile| profile.media.as_deref())
                .unwrap_or(&self.media),
            surface: overrides
                .and_then(|profile| profile.surface.as_ref())
                .unwrap_or(&self.surface),
            bottom: overrides
                .and_then(|profile| profile.bottom.as_ref())
                .unwrap_or(&self.bottom),
            phase_speed: options
                .and_then(|options| options.phase_speed)
                .unwrap_or(self.kraken.phase_speed),
            max_range: options
                .and_then(|options| options.max_range)
                .unwrap_or(self.kraken.max_range),
            mode_depths: options
                .and_then(|options| options.mode_depths.as_ref())
                .or(self.kraken.mode_depths.as_ref()),
            mesh_frequency: options
                .and_then(|options| options.mesh_frequency)
                .unwrap_or(self.kraken.mesh_frequency),
        }
    }

    fn field(&self, problems: &mut Problems<'_>) -> Field {
        let mut expand = |grid: &Grid, single: bool, field: &str| {
            let values = if single {
                grid.expand_single()
            } else {
                grid.expand()
            };
            values.unwrap_or_else(|message| {
                problems.push(field, message);
                Vec::new()
            })
        };
        let source_depths = expand(&self.source.depth, true, "source.depth");
        let receiver_depths = expand(&self.receivers.depth, true, "receivers.depth");
        let receiver_ranges_m = expand(&self.receivers.range, false, "receivers.range");
        let receiver_offsets_m = match &self.receivers.offset {
            Some(grid) => expand(grid, false, "receivers.offset"),
            None => vec![0.0; receiver_depths.len()],
        };
        let mut source_pattern = Vec::new();
        if let Some(rows) = &self.source.pattern {
            if rows.len() < 2 {
                problems.push(
                    "source.pattern",
                    "source pattern requires at least two points",
                );
            }
            for (index, &[angle_degrees, level_db]) in rows.iter().enumerate() {
                let amplitude = 10.0_f64.powf(level_db / 20.0);
                if !amplitude.is_finite()
                    || source_pattern
                        .last()
                        .is_some_and(|point: &SourcePatternPoint| {
                            angle_degrees <= point.angle_degrees
                        })
                {
                    problems.push(
                        format!("source.pattern[{index}]"),
                        "source pattern requires increasing finite angles and finite dB levels",
                    );
                }
                source_pattern.push(SourcePatternPoint {
                    angle_degrees,
                    amplitude,
                });
            }
        }
        Field {
            source_depths,
            receiver_depths,
            receiver_ranges_m,
            receiver_offsets_m,
            source_pattern,
        }
    }

    /// The geometry/options template and raw materials of one environment,
    /// with its layer layout and input-value count.
    #[allow(clippy::too_many_lines, clippy::float_cmp)]
    fn template(
        &self,
        environment: &Environment<'_>,
        field: &Field,
        frequencies: &[f64],
        problems: &mut Problems<'_>,
    ) -> Option<(CaseDefinition, Materials, Layout, usize)> {
        let solver = self.kraken.solver;
        let (materials, surface_boundary, bottom_boundary, interface_roughness_m, layout) =
            build_materials(environment, solver, frequencies, problems)?;
        // Depths are single precision; samples at a fluid interface keep the exact interface.
        #[allow(clippy::cast_possible_truncation)]
        let boundaries = [materials.fluid_bottom(), materials.fluid_top()]
            .map(|depth| (f64::from(depth as f32), depth));
        let snap = |depths: &mut [f64]| snap_to_interfaces(depths, &boundaries);
        let mut source_depths = field.source_depths.clone();
        let mut receiver_depths = field.receiver_depths.clone();
        snap(&mut source_depths);
        snap(&mut receiver_depths);
        let mode_sample_depths_m = if let Some(grid) = environment.mode_depths {
            let mut depths = grid.expand_single().unwrap_or_else(|message| {
                problems.push(environment.path("kraken.mode_depths"), message);
                Vec::new()
            });
            snap(&mut depths);
            depths
        } else {
            let mut depths = source_depths.clone();
            depths.extend(&receiver_depths);
            depths.sort_by(f64::total_cmp);
            depths.dedup();
            depths
        };
        let template = CaseDefinition {
            title: environment.title.to_owned(),
            mode_solver: solver,
            frequency_hz: frequencies.first().copied().unwrap_or(0.0),
            mesh_reference_frequency_hz: environment.mesh_frequency,
            water_depth_m: 0.0,
            interpolation: environment.interpolation,
            surface_boundary,
            surface_sound_speed_mps: 0.0,
            surface_density_g_cm3: 0.0,
            surface_attenuation_db_per_wavelength: 0.0,
            sound_speed_profile: Vec::new(),
            water_density_g_cm3: 0.0,
            water_attenuation_db_per_wavelength: Vec::new(),
            additional_fluid_layers: Vec::new(),
            interface_roughness_m,
            top_elastic_layers: Vec::new(),
            bottom_elastic_layers: Vec::new(),
            bottom_boundary,
            bottom_sound_speed_mps: 0.0,
            bottom_density_g_cm3: 0.0,
            bottom_attenuation_db_per_wavelength: 0.0,
            source_geometry: self.source.geometry,
            mode_addition: self.kraken.addition,
            source_pattern: field.source_pattern.clone(),
            mesh_points: 0,
            c_low_mps: environment.phase_speed[0],
            c_high_mps: environment.phase_speed[1],
            max_range_m: environment.max_range,
            mode_sample_depths_m,
            mode_limit: self.kraken.mode_limit.unwrap_or(MAX_MODE_LIMIT),
            source_depths_m: source_depths,
            receiver_depths_m: receiver_depths,
            receiver_ranges_m: field.receiver_ranges_m.clone(),
            receiver_offsets_m: field.receiver_offsets_m.clone(),
        };
        let table_values = match &template.bottom_boundary {
            Boundary::Reflection(points) => points.len(),
            Boundary::Impedance { points, .. } => points.len(),
            _ => 0,
        } + match &template.surface_boundary {
            Boundary::Reflection(points) => points.len(),
            _ => 0,
        };
        let values = materials.input_values()
            + template.interface_roughness_m.len()
            + table_values
            + 2 * template.source_pattern.len()
            + template.mode_sample_depths_m.len()
            + template.source_depths_m.len()
            + template.receiver_depths_m.len()
            + template.receiver_ranges_m.len()
            + template.receiver_offsets_m.len();
        Some((template, materials, layout, values))
    }
}

pub(crate) fn snap_to_interfaces(depths: &mut [f64], boundaries: &[(f64, f64); 2]) {
    #[allow(clippy::float_cmp)]
    for depth in depths {
        if let Some(&(_, exact)) = boundaries.iter().find(|(rounded, _)| *depth == *rounded) {
            *depth = exact;
        }
    }
}

type BuiltMaterials = (Materials, Boundary, Boundary, Vec<f64>, Layout);

#[allow(clippy::too_many_lines, clippy::float_cmp)]
fn build_materials(
    environment: &Environment<'_>,
    solver: ModeSolver,
    frequencies: &[f64],
    problems: &mut Problems<'_>,
) -> Option<BuiltMaterials> {
    let mesh_frequency = environment.mesh_frequency;
    let attenuation_path = environment.path("attenuation");
    let attenuation = environment.attenuation;
    let unit = attenuation.unit.code();
    let analytic = environment.interpolation == Interpolation::AnalyticMunk;
    let volume = volume_loss(&attenuation.volume, &attenuation_path, problems);
    if analytic && !matches!(volume, VolumeLoss::None) {
        problems.push(
            environment.path("attenuation.volume"),
            "analytic Munk water remains lossless; volume attenuation is not supported",
        );
    }
    let reference = match attenuation.reference_frequency {
        Some(reference) if reference.is_finite() && reference > 0.0 => reference,
        Some(_) => {
            problems.push(
                format!("{attenuation_path}.reference_frequency"),
                "the reference frequency must be finite and positive",
            );
            1.0
        }
        None if unit == b'm' => {
            problems.push(
                format!("{attenuation_path}.reference_frequency"),
                "db_per_meter_power_law requires a reference frequency",
            );
            1.0
        }
        None => frequencies.first().copied().unwrap_or(1.0),
    };
    let power_law = |law: Option<[f64; 2]>, field: String, problems: &mut Problems<'_>| {
        power_law(unit, reference, law, field, problems)
    };

    let media_path = environment.path("media");
    let media = environment.media;
    if media.is_empty() || media.len() > crate::solver::layers::MAX_LAYERS {
        problems.push(&media_path, "require 1..=500 finite media");
        return None;
    }
    let surface_path = environment.path("surface");
    let surface = environment.surface;
    if media.len() > 1 && (analytic || matches!(surface, BoundaryDocument::Reflection { .. })) {
        problems.push(
            &media_path,
            "analytic Munk and a surface reflection table remain single-layer",
        );
    }
    let surface_values = match surface {
        BoundaryDocument::Halfspace {
            cp,
            cs,
            rho,
            ap,
            as_,
            power_law,
            ..
        } => {
            if unit == b'm' {
                problems.push(
                    &surface_path,
                    "a surface half-space with power-law loss has no defined parameters",
                );
            } else if power_law.is_some() {
                problems.push(
                    format!("{surface_path}.power_law"),
                    "power_law is bottom-only",
                );
            }
            let values = [0.0, *cp, *cs, *rho, *ap, *as_];
            check_half_space(&values, &surface_path, problems);
            values
        }
        _ => [0.0; 6],
    };

    let mut water: Option<Fluid> = None;
    let (mut additional, mut top_layers, mut bottom_layers) = (Vec::new(), Vec::new(), Vec::new());
    let mut interface_roughness_m = vec![surface.roughness()];
    let mut top = 0.0;
    let mut profile_points = 0_usize;
    for (index, medium) in media.iter().enumerate() {
        let field = format!("{media_path}[{index}]");
        if index == 0 && medium.roughness != 0.0 {
            problems.push(
                format!("{field}.roughness"),
                "the first medium's upper interface is the surface: use surface.roughness",
            );
        } else if index > 0 {
            interface_roughness_m.push(medium.roughness);
        }
        if medium.roughness < 0.0 || surface.roughness() < 0.0 {
            problems.push(
                format!("{field}.roughness"),
                "interface roughness must be nonnegative",
            );
        }
        let bottom = medium.to;
        if bottom.partial_cmp(&top) != Some(std::cmp::Ordering::Greater) {
            problems.push(
                format!("{field}.to"),
                "layer interfaces must strictly increase",
            );
        }
        let power = power_law(medium.power_law, format!("{field}.power_law"), problems);
        let points = if analytic {
            if !medium.ssp.is_empty() {
                problems.push(
                    format!("{field}.ssp"),
                    "analytic_munk takes no sound-speed rows",
                );
            }
            Vec::new()
        } else {
            medium_points(&medium.ssp, top, bottom, &format!("{field}.ssp"), problems)
        };
        profile_points += points.len();
        if profile_points > MAX_VECTOR_LENGTH {
            problems.push(&media_path, "total finite profile points exceed the limit");
            return None;
        }
        top = bottom;
        if points.first().is_some_and(|point| point[2] > 0.0) {
            let layer = Elastic {
                bottom_depth_m: bottom,
                points,
                mesh_points: medium.mesh,
                power_law: power,
            };
            if water.is_none() {
                top_layers.push(layer);
            } else {
                bottom_layers.push(layer);
            }
        } else {
            if !bottom_layers.is_empty() {
                problems.push(
                    &field,
                    "finite fluids must be contiguous; solids between fluids are unsupported",
                );
            }
            let layer = Fluid {
                bottom_depth_m: bottom,
                density_g_cm3: points.first().map_or(1.0, |point| point[3]),
                mesh_points: medium.mesh,
                points: points
                    .iter()
                    .map(|point| SoundSpeedPoint {
                        depth_m: point[0],
                        sound_speed_mps: point[1],
                    })
                    .collect(),
                attenuation: points.iter().map(|point| point[4]).collect(),
                power_law: power,
            };
            if water.is_none() {
                water = Some(layer);
            } else {
                additional.push(layer);
            }
        }
    }
    let Some(water) = water else {
        problems.push(&media_path, "requires at least one finite fluid medium");
        return None;
    };

    let bottom_path = environment.path("bottom");
    let bottom = environment.bottom;
    interface_roughness_m.push(bottom.roughness());
    if bottom.roughness() < 0.0 {
        problems.push(
            format!("{bottom_path}.roughness"),
            "roughness must be nonnegative",
        );
    }
    if interface_roughness_m
        .iter()
        .all(|&roughness| roughness == 0.0)
    {
        interface_roughness_m.clear();
    }
    let tabulated = |problems: &mut Problems<'_>, path: &str| {
        if solver != ModeSolver::Krakenc {
            problems.push(path, "reflection and precalculated tables require KRAKENC");
        }
        if media.len() > 1 {
            problems.push(path, "tabulated boundaries remain single-layer");
        }
    };
    let surface_boundary = match surface {
        BoundaryDocument::Vacuum { .. } => Boundary::Vacuum,
        BoundaryDocument::Rigid { .. } => Boundary::Rigid,
        BoundaryDocument::Halfspace { cs, .. } if *cs > 0.0 => Boundary::ElasticHalfSpace {
            shear_sound_speed_mps: *cs,
            shear_attenuation_db_per_wavelength: 0.0,
        },
        BoundaryDocument::Halfspace { .. } => Boundary::FluidHalfSpace,
        BoundaryDocument::Reflection { table, .. } => {
            tabulated(problems, &surface_path);
            reflection_boundary(table, &surface_path, problems)
        }
        BoundaryDocument::Precalculated { .. } => {
            problems.push(&surface_path, "`precalculated` is a bottom-only condition");
            Boundary::Vacuum
        }
    };
    let mut bottom_values = [0.0; 6];
    let mut bottom_power = (reference, 1.0, reference);
    let bottom_boundary = match bottom {
        BoundaryDocument::Vacuum { .. } => Boundary::Vacuum,
        BoundaryDocument::Rigid { .. } => Boundary::Rigid,
        BoundaryDocument::Halfspace {
            cp,
            cs,
            rho,
            ap,
            as_,
            power_law: law,
            ..
        } => {
            bottom_values = [top, *cp, *cs, *rho, *ap, *as_];
            check_half_space(&bottom_values, &bottom_path, problems);
            bottom_power = power_law(*law, format!("{bottom_path}.power_law"), problems);
            if *cs > 0.0 {
                Boundary::ElasticHalfSpace {
                    shear_sound_speed_mps: *cs,
                    shear_attenuation_db_per_wavelength: 0.0,
                }
            } else {
                Boundary::FluidHalfSpace
            }
        }
        BoundaryDocument::Reflection { table, .. } => {
            tabulated(problems, &bottom_path);
            reflection_boundary(table, &bottom_path, problems)
        }
        BoundaryDocument::Precalculated {
            frequency, table, ..
        } => {
            tabulated(problems, &bottom_path);
            let points = table
                .iter()
                .enumerate()
                .filter_map(|(index, &[k2, f_re, f_im, g_re, g_im, power])| {
                    #[allow(clippy::cast_possible_truncation)]
                    let integer = power as i32;
                    if f64::from(integer) != power {
                        problems.push(
                            format!("{bottom_path}.table[{index}]"),
                            "the power must be an integer",
                        );
                        return None;
                    }
                    Some(ImpedancePoint {
                        wavenumber_squared: k2,
                        f: Complex64::new(f_re, f_im),
                        g: Complex64::new(g_re, g_im),
                        power: integer,
                    })
                })
                .collect();
            let boundary = Boundary::Impedance {
                frequency_hz: *frequency,
                points,
            };
            // The `.env` checks the table against its header frequency.
            let header = mesh_frequency
                .or(frequencies.first().copied())
                .unwrap_or(*frequency);
            if let Err(message) = crate::solver::reflection::validate_table(&boundary, header) {
                problems.push(&bottom_path, message);
            }
            boundary
        }
    };
    let layout = Layout {
        top: top_layers.len(),
        additional: additional.len(),
    };
    let materials = Materials {
        surface_half_space: HalfSpace {
            values: surface_values,
            power_law: water.power_law,
        },
        bottom_half_space: HalfSpace {
            values: bottom_values,
            power_law: bottom_power,
        },
        water,
        additional,
        top: top_layers,
        bottom: bottom_layers,
        unit,
        volume,
    };
    Some((
        materials,
        surface_boundary,
        bottom_boundary,
        interface_roughness_m,
        layout,
    ))
}

/// The `(reference, beta, transition)` power law of one medium or half-space.
fn power_law(
    unit: u8,
    reference: f64,
    law: Option<[f64; 2]>,
    field: String,
    problems: &mut Problems<'_>,
) -> (f64, f64, f64) {
    match (unit == b'm', law) {
        (false, None) => (reference, 1.0, reference),
        (false, Some(_)) => {
            problems.push(field, "power_law applies only to db_per_meter_power_law");
            (reference, 1.0, reference)
        }
        (true, None) => {
            problems.push(
                field,
                "db_per_meter_power_law requires [beta, transition frequency]",
            );
            (reference, 1.0, reference)
        }
        (true, Some([beta, transition])) => {
            if beta < 0.0 || transition <= 0.0 {
                problems.push(field, "require beta >= 0 and transition frequency > 0");
            }
            (reference, beta, transition)
        }
    }
}

/// Expands one medium's rows with `.env` inheritance and checks them as the reader does.
#[allow(clippy::float_cmp)]
fn medium_points(
    rows: &[Vec<f64>],
    top: f64,
    bottom: f64,
    field: &str,
    problems: &mut Problems<'_>,
) -> Vec<[f64; 6]> {
    let mut current = ROW_DEFAULTS;
    let mut points: Vec<[f64; 6]> = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let row_field = format!("{field}[{index}]");
        let minimum = if index == 0 { 2 } else { 1 };
        if !(minimum..=6).contains(&row.len()) {
            problems.push(
                row_field,
                format!("expected {minimum} to 6 values: depth, cp, cs, rho, ap, as"),
            );
            return points;
        }
        current[..row.len()].copy_from_slice(row);
        let point = current;
        if point[0] < top || point[0] > bottom || points.last().is_some_and(|p| point[0] <= p[0]) {
            problems.push(
                row_field,
                "depths must increase within each medium, from its top to `to`",
            );
            return points;
        }
        if (points.is_empty() && point[0] != top)
            || point[2] < 0.0
            || point[4] < 0.0
            || point[5] < 0.0
            || (point[2] == 0.0 && point[5] != 0.0)
            || points.first().is_some_and(|p| {
                (p[2] == 0.0 && (point[3] != p[3] || point[2] != 0.0))
                    || (p[2] > 0.0 && point[2] == 0.0)
            })
        {
            problems.push(row_field, "requires constant-density fluid (no shear loss), or positive elastic shear speed throughout the medium; nonnegative absorption");
            return points;
        }
        points.push(point);
    }
    if points.len() < 2 || points.last().is_some_and(|point| point[0] != bottom) {
        problems.push(field, "a medium needs rows from its top depth to `to`");
    }
    points
}

#[allow(clippy::float_cmp)]
fn check_half_space(values: &[f64; 6], field: &str, problems: &mut Problems<'_>) {
    if values[1] <= 0.0
        || values[2] < 0.0
        || values[3] <= 0.0
        || values[4] < 0.0
        || values[5] < 0.0
        || (values[2] == 0.0 && values[5] != 0.0)
    {
        problems.push(
            field,
            "require positive cp and rho, nonnegative cs and losses, and no shear loss for a fluid",
        );
    }
}

fn reflection_boundary(table: &[[f64; 3]], field: &str, problems: &mut Problems<'_>) -> Boundary {
    let boundary = Boundary::Reflection(
        table
            .iter()
            .map(
                |&[angle_degrees, magnitude, phase_degrees]| ReflectionPoint {
                    angle_degrees,
                    magnitude,
                    phase_radians: phase_degrees.to_radians(),
                },
            )
            .collect(),
    );
    if let Err(message) = crate::solver::reflection::validate_table(&boundary, 0.0) {
        problems.push(field, message);
    }
    boundary
}

#[allow(clippy::float_cmp)]
fn volume_loss(volume: &VolumeAttenuation, field: &str, problems: &mut Problems<'_>) -> VolumeLoss {
    match volume {
        VolumeAttenuation::None => VolumeLoss::None,
        VolumeAttenuation::Thorp => VolumeLoss::Thorp,
        &VolumeAttenuation::FrancoisGarrison {
            temperature,
            salinity,
            ph,
            depth,
        } => {
            if temperature <= -273.0 || salinity < 0.0 || !(0.0..=14.0).contains(&ph) || depth < 0.0
            {
                problems.push(
                    format!("{field}.volume"),
                    "require temperature > -273, salinity >= 0, 0 <= pH <= 14 and depth >= 0",
                );
            }
            VolumeLoss::FrancoisGarrison([temperature, salinity, ph, depth])
        }
        VolumeAttenuation::Biological { layers } => {
            if layers.len() > 200 {
                problems.push(
                    format!("{field}.volume.layers"),
                    "at most 200 biological layers",
                );
            }
            for (index, p) in layers.iter().enumerate() {
                if p[0] < 0.0 || p[1] < p[0] || p[2] <= 0.0 || p[3] <= 0.0 || p[4] < 0.0 {
                    problems.push(
                        format!("{field}.volume.layers[{index}]"),
                        "require 0 <= top <= bottom, positive resonance/Q and nonnegative a0",
                    );
                }
            }
            VolumeLoss::Biological(layers.clone())
        }
    }
}

/// Maps a validated-model field name to this document's JSON path.
fn relocate(
    mut report: DiagnosticReport,
    path: &Path,
    environment: &Environment<'_>,
    layout: &Layout,
    frequencies: &[f64],
    frequency: f64,
) -> DiagnosticReport {
    let water = layout.top;
    let medium = |index: usize, suffix: &str| {
        let suffix = match suffix {
            "" => String::new(),
            "bottom_depth_m" => ".to".into(),
            "mesh_points" => ".mesh".into(),
            _ => ".ssp".into(),
        };
        format!("{}[{index}]{suffix}", environment.path("media"))
    };
    let layer = |field: &str, list: &str, offset: usize| {
        let rest = field.strip_prefix(list)?.strip_prefix('[')?;
        let (index, rest) = rest.split_once(']')?;
        let index: usize = index.parse().ok()?;
        Some(medium(
            offset + index,
            rest.strip_prefix('.').unwrap_or(rest),
        ))
    };
    for diagnostic in &mut report.diagnostics {
        let field = diagnostic.field.as_str();
        let mapped = layer(field, "top_elastic_layers", 0)
            .or_else(|| layer(field, "additional_fluid_layers", water + 1))
            .or_else(|| {
                layer(
                    field,
                    "bottom_elastic_layers",
                    water + 1 + layout.additional,
                )
            })
            .unwrap_or_else(|| match field {
                "water_depth_m" => medium(water, "bottom_depth_m"),
                "mesh_points" => medium(water, "mesh_points"),
                "sound_speed_profile"
                | "water_density_g_cm3"
                | "water_attenuation_db_per_wavelength" => medium(water, "ssp"),
                "additional_fluid_layers"
                | "top_elastic_layers"
                | "bottom_elastic_layers"
                | "interface_roughness_m" => environment.path("media"),
                "surface_boundary"
                | "surface_sound_speed_mps"
                | "surface_density_g_cm3"
                | "surface_attenuation_db_per_wavelength" => environment.path("surface"),
                "bottom_boundary"
                | "bottom_sound_speed_mps"
                | "bottom_density_g_cm3"
                | "bottom_attenuation_db_per_wavelength" => environment.path("bottom"),
                "frequency_hz" => "frequency".into(),
                "mesh_reference_frequency_hz" => environment.path("kraken.mesh_frequency"),
                "phase_speed_limits" | "c_low_mps" | "c_high_mps" => {
                    environment.path("kraken.phase_speed")
                }
                "max_range_m" => environment.path("kraken.max_range"),
                "mode_sample_depths_m" => environment.path("kraken.mode_depths"),
                "mode_limit" => "kraken.mode_limit".into(),
                "mode_addition" => "kraken.addition".into(),
                "source_depths_m" => "source.depth".into(),
                "source_pattern" => "source.pattern".into(),
                "receiver_depths_m" => "receivers.depth".into(),
                "receiver_ranges_m" => "receivers.range".into(),
                "receiver_offsets_m" => "receivers.offset".into(),
                "field_grid" => "receivers".into(),
                "profiles" | "profile_ranges_m" => "profiles".into(),
                other => other.to_owned(),
            });
        diagnostic.field = mapped;
        diagnostic.path = path.to_path_buf();
        diagnostic.line = 1;
        diagnostic.column = 1;
        if frequencies.len() > 1 {
            diagnostic.message = format!("{} (at {frequency} Hz)", diagnostic.message);
        }
    }
    report
}

/// Builds the document parts of one legacy environment, for export.
pub(crate) struct EnvironmentParts {
    pub title: String,
    pub interpolation: Interpolation,
    pub attenuation: Attenuation,
    pub media: Vec<Medium>,
    pub surface: BoundaryDocument,
    pub bottom: BoundaryDocument,
    pub phase_speed: [f64; 2],
    pub max_range: f64,
    /// This environment's own mode depths, always explicit.
    pub mode_depths: Grid,
    /// The broadband header frequency that sizes this environment's meshes.
    pub mesh_frequency: Option<f64>,
}

impl EnvironmentParts {
    /// Exports raw materials, keeping the loss recipe rather than converted values.
    #[allow(clippy::too_many_arguments, clippy::float_cmp, clippy::too_many_lines)]
    pub(crate) fn new(
        title: String,
        interpolation: Interpolation,
        header_frequency: f64,
        materials: &Materials,
        surface: &Boundary,
        bottom: &Boundary,
        interface_roughness_m: &[f64],
        phase_speed: [f64; 2],
        max_range: f64,
        mode_depths: Grid,
    ) -> Self {
        let power_law = materials.unit == b'm';
        let law = |power: (f64, f64, f64)| power_law.then_some([power.1, power.2]);
        let roughness = |index: usize| interface_roughness_m.get(index).copied().unwrap_or(0.0);
        let mut media = Vec::new();
        let elastic = |layer: &Elastic| Medium {
            to: layer.bottom_depth_m,
            ssp: trimmed_rows(&layer.points),
            mesh: layer.mesh_points,
            roughness: 0.0,
            power_law: law(layer.power_law),
        };
        let fluid = |layer: &Fluid| {
            let rows: Vec<[f64; 6]> = layer
                .points
                .iter()
                .zip(&layer.attenuation)
                .map(|(point, &loss)| {
                    [
                        point.depth_m,
                        point.sound_speed_mps,
                        0.0,
                        layer.density_g_cm3,
                        loss,
                        0.0,
                    ]
                })
                .collect();
            Medium {
                to: layer.bottom_depth_m,
                ssp: trimmed_rows(&rows),
                mesh: layer.mesh_points,
                roughness: 0.0,
                power_law: law(layer.power_law),
            }
        };
        media.extend(materials.top.iter().map(elastic));
        media.push(fluid(&materials.water));
        media.extend(materials.additional.iter().map(fluid));
        media.extend(materials.bottom.iter().map(elastic));
        for (index, medium) in media.iter_mut().enumerate().skip(1) {
            medium.roughness = roughness(index);
        }
        let last = media.len();
        let half_space = |values: &[f64; 6], roughness: f64, power: Option<[f64; 2]>| {
            BoundaryDocument::Halfspace {
                roughness,
                cp: values[1],
                cs: values[2],
                rho: values[3],
                ap: values[4],
                as_: values[5],
                power_law: power,
            }
        };
        let boundary =
            |boundary: &Boundary, values: &[f64; 6], roughness: f64, power| match boundary {
                Boundary::Vacuum => BoundaryDocument::Vacuum { roughness },
                Boundary::Rigid => BoundaryDocument::Rigid { roughness },
                Boundary::FluidHalfSpace | Boundary::ElasticHalfSpace { .. } => {
                    half_space(values, roughness, power)
                }
                Boundary::Reflection(points) => BoundaryDocument::Reflection {
                    roughness,
                    table: points
                        .iter()
                        .map(|p| [p.angle_degrees, p.magnitude, p.phase_radians.to_degrees()])
                        .collect(),
                },
                Boundary::Impedance {
                    frequency_hz,
                    points,
                } => BoundaryDocument::Precalculated {
                    roughness,
                    frequency: *frequency_hz,
                    table: points
                        .iter()
                        .map(|p| {
                            [
                                p.wavenumber_squared,
                                p.f.re,
                                p.f.im,
                                p.g.re,
                                p.g.im,
                                f64::from(p.power),
                            ]
                        })
                        .collect(),
                },
            };
        Self {
            title,
            interpolation,
            attenuation: Attenuation {
                unit: AttenuationUnit::from_code(materials.unit),
                volume: match &materials.volume {
                    VolumeLoss::None => VolumeAttenuation::None,
                    VolumeLoss::Thorp => VolumeAttenuation::Thorp,
                    &VolumeLoss::FrancoisGarrison([temperature, salinity, ph, depth]) => {
                        VolumeAttenuation::FrancoisGarrison {
                            temperature,
                            salinity,
                            ph,
                            depth,
                        }
                    }
                    VolumeLoss::Biological(layers) => VolumeAttenuation::Biological {
                        layers: layers.clone(),
                    },
                },
                reference_frequency: power_law.then_some(header_frequency),
            },
            media,
            surface: boundary(
                surface,
                &materials.surface_half_space.values,
                roughness(0),
                None,
            ),
            bottom: boundary(
                bottom,
                &materials.bottom_half_space.values,
                roughness(last),
                law(materials.bottom_half_space.power_law),
            ),
            phase_speed,
            max_range,
            mode_depths,
            mesh_frequency: None,
        }
    }
}

/// Equal-width rows, dropping trailing columns that only hold defaults.
fn trimmed_rows(rows: &[[f64; 6]]) -> Vec<Vec<f64>> {
    let width = (2..6)
        .rev()
        .find(|&column| {
            rows.iter()
                .any(|row| row[column].to_bits() != ROW_DEFAULTS[column].to_bits())
        })
        .map_or(2, |column| column + 1);
    rows.iter().map(|row| row[..width].to_vec()).collect()
}

/// Assembles a document from exported environments, the first at 0 m.
/// `default_mode_depths` is what an omitted `mode_depths` would mean.
#[allow(clippy::too_many_arguments, clippy::float_cmp)]
pub(crate) fn assemble(
    environments: Vec<(f64, EnvironmentParts)>,
    default_mode_depths: &Grid,
    frequencies: &[f64],
    solver: ModeSolver,
    source: Source,
    receivers: Receivers,
    mode_limit: usize,
    addition: ModeAddition,
    propagation: FieldPropagation,
) -> CaseDocument {
    let mut environments = environments.into_iter();
    let (_, base) = environments.next().expect("at least one environment");
    let mut profiles = Vec::new();
    for (range, parts) in environments {
        profiles.push({
            let options = ProfileOptions {
                phase_speed: (parts.phase_speed != base.phase_speed).then_some(parts.phase_speed),
                max_range: (parts.max_range.to_bits() != base.max_range.to_bits())
                    .then_some(parts.max_range),
                mode_depths: (parts.mode_depths != base.mode_depths).then_some(parts.mode_depths),
                mesh_frequency: (parts.mesh_frequency.map(f64::to_bits)
                    != base.mesh_frequency.map(f64::to_bits))
                .then_some(parts.mesh_frequency),
            };
            Profile {
                range,
                title: (parts.title != base.title).then_some(parts.title),
                interpolation: (parts.interpolation != base.interpolation)
                    .then_some(parts.interpolation),
                attenuation: (parts.attenuation != base.attenuation).then_some(parts.attenuation),
                media: (parts.media != base.media).then_some(parts.media),
                surface: (parts.surface != base.surface).then_some(parts.surface),
                bottom: (parts.bottom != base.bottom).then_some(parts.bottom),
                kraken: (options != ProfileOptions::default()).then_some(options),
            }
        });
    }
    CaseDocument {
        pelagic: SCHEMA_VERSION,
        model: Model::Kraken,
        title: base.title,
        frequency: Grid::describe(frequencies),
        interpolation: base.interpolation,
        attenuation: base.attenuation,
        media: base.media,
        surface: base.surface,
        bottom: base.bottom,
        source,
        receivers,
        kraken: Options {
            solver,
            phase_speed: base.phase_speed,
            max_range: base.max_range,
            mesh_frequency: base.mesh_frequency,
            mode_depths: (base.mode_depths != *default_mode_depths).then_some(base.mode_depths),
            mode_limit: (mode_limit != MAX_MODE_LIMIT).then_some(mode_limit),
            addition,
            propagation: match propagation {
                FieldPropagation::RangeIndependent => None,
                FieldPropagation::Adiabatic => Some(Propagation::Adiabatic),
                FieldPropagation::Coupled => Some(Propagation::Coupled),
            },
        },
        profiles,
    }
}

fn malformed_field(path: &str) -> String {
    // Serde buffers a tagged payload; report its object consistently.
    for boundary in ["surface", "bottom"] {
        for (index, _) in path.match_indices(boundary) {
            let before = &path[..index];
            let after = &path[index + boundary.len()..];
            if (before.is_empty() || before.ends_with('.'))
                && (after.is_empty() || after.starts_with('.'))
            {
                return format!("{before}{boundary}");
            }
        }
    }
    path.to_owned()
}

fn error(path: &Path, code: &'static str, message: &str, field: &str) -> DiagnosticReport {
    DiagnosticReport::one(Diagnostic::new(code, message, field, path, 1, 1))
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
fn is_one(value: &f64) -> bool {
    value.to_bits() == 1.0_f64.to_bits()
}

fn one() -> f64 {
    1.0
}

fn point() -> SourceGeometry {
    SourceGeometry::Point
}

// serde `skip_serializing_if` passes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_point(value: &SourceGeometry) -> bool {
    *value == SourceGeometry::Point
}

fn coherent() -> ModeAddition {
    ModeAddition::Coherent
}

// serde `skip_serializing_if` passes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_coherent(value: &ModeAddition) -> bool {
    *value == ModeAddition::Coherent
}

/// Marks a present field, including `null`, as `Some`; a missing one stays `None`.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
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

fn optional_name_or_object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    name_or_object(deserializer).map(Some)
}

fn boundary_shorthand<S: Serializer>(
    boundary: &BoundaryDocument,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match boundary {
        BoundaryDocument::Vacuum { roughness } if is_zero(roughness) => {
            serializer.serialize_str("vacuum")
        }
        BoundaryDocument::Rigid { roughness } if is_zero(roughness) => {
            serializer.serialize_str("rigid")
        }
        other => other.serialize(serializer),
    }
}

#[allow(clippy::ref_option)]
fn optional_boundary_shorthand<S: Serializer>(
    boundary: &Option<BoundaryDocument>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match boundary {
        Some(boundary) => boundary_shorthand(boundary, serializer),
        None => serializer.serialize_none(),
    }
}

fn volume_shorthand<S: Serializer>(
    volume: &VolumeAttenuation,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match volume {
        VolumeAttenuation::None => serializer.serialize_str("none"),
        VolumeAttenuation::Thorp => serializer.serialize_str("thorp"),
        other => other.serialize(serializer),
    }
}

pub(crate) mod complex {
    use num_complex::Complex64;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Value {
        real: f64,
        imaginary: f64,
    }

    pub fn serialize<S: Serializer>(value: &Complex64, serializer: S) -> Result<S::Ok, S::Error> {
        Value {
            real: value.re,
            imaginary: value.im,
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Complex64, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(Complex64::new(value.real, value.imaginary))
    }
}

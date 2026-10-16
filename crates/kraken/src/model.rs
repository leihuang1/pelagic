use num_complex::Complex64;
use serde::{Deserialize, Serialize};

use crate::json;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    N2Linear,
    CLinear,
    Pchip,
    Spline,
    /// The fixed 5000 m Munk profile from the pinned Fortran `misc/munk.f90`.
    AnalyticMunk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeSolver {
    Kraken,
    Krakenc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceGeometry {
    Line,
    Point,
    ScaledCylindrical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeAddition {
    Coherent,
    Incoherent,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePatternPoint {
    pub angle_degrees: f64,
    /// Linear pressure amplitude.
    pub amplitude: f64,
}

/// Smooth boundary; compressional material values live in the corresponding case fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Boundary {
    Vacuum,
    FluidHalfSpace,
    /// An elastic A half-space; compressional material uses the existing case fields.
    /// Top elasticity requires KRAKENC. Real KRAKEN retains the reference's
    /// elastic-loss omission and shear-speed cutoff.
    ElasticHalfSpace {
        shear_sound_speed_mps: f64,
        /// Solve-frequency shear loss, in dB/wavelength.
        shear_attenuation_db_per_wavelength: f64,
    },
    Rigid,
    /// KRAKENC F (top TRC or bottom BRC): magnitude and unwrapped phase versus grazing angle.
    Reflection(Vec<ReflectionPoint>),
    /// KRAKENC bottom P, scaled impedance functions versus squared wavenumber.
    Impedance {
        frequency_hz: f64,
        points: Vec<ImpedancePoint>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflectionPoint {
    pub angle_degrees: f64,
    pub magnitude: f64,
    pub phase_radians: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpedancePoint {
    pub wavenumber_squared: f64,
    #[serde(with = "json::complex")]
    pub f: Complex64,
    #[serde(with = "json::complex")]
    pub g: Complex64,
    pub power: i32,
}

/// Top V/R/A/F boundary. Top P is explicitly unsupported.
pub type SurfaceBoundary = Boundary;
/// Bottom V/R/A/F/P boundary.
pub type BottomBoundary = Boundary;

impl Boundary {
    /// Whether this A boundary carries acoustic or elastic half-space material.
    #[must_use]
    pub fn is_half_space(&self) -> bool {
        matches!(self, Self::FluidHalfSpace | Self::ElasticHalfSpace { .. })
    }

    pub(crate) fn is_tabulated(&self) -> bool {
        matches!(self, Self::Reflection(_) | Self::Impedance { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundSpeedPoint {
    pub depth_m: f64,
    pub sound_speed_mps: f64,
}

/// A finite fluid layer below the first water layer. Depths are absolute metres.
/// Interpolation and attenuation conventions are shared with the enclosing case.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FluidLayer {
    pub bottom_depth_m: f64,
    pub density_g_cm3: f64,
    pub sound_speed_profile: Vec<SoundSpeedPoint>,
    /// Empty for lossless material, otherwise one solve-frequency dB/wavelength per node.
    pub attenuation_db_per_wavelength: Vec<f64>,
    /// Nominal mesh intervals; 0 selects the reference automatic mesh.
    pub mesh_points: usize,
}

/// One finite-elastic material sample at an absolute depth.
/// Losses are canonical dB/wavelength at the enclosing solve frequency.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElasticMaterialPoint {
    pub depth_m: f64,
    pub compressional_sound_speed_mps: f64,
    pub shear_sound_speed_mps: f64,
    pub density_g_cm3: f64,
    pub compressional_attenuation_db_per_wavelength: f64,
    pub shear_attenuation_db_per_wavelength: f64,
}

/// A finite elastic layer. Empty `material_profile` selects homogeneous material.
/// With a profile, scalar material fields must equal its first sample.
/// Depths are absolute metres; losses are solve-frequency dB/wavelength.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElasticLayer {
    pub bottom_depth_m: f64,
    pub compressional_sound_speed_mps: f64,
    pub shear_sound_speed_mps: f64,
    pub density_g_cm3: f64,
    pub compressional_attenuation_db_per_wavelength: f64,
    pub shear_attenuation_db_per_wavelength: f64,
    /// Nominal mesh intervals, 0 for the reference automatic shear mesh.
    pub mesh_points: usize,
    /// Ordered top-to-bottom material samples, interpolated using the case SSP option.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub material_profile: Vec<ElasticMaterialPoint>,
}

/// Unvalidated contiguous fluid stack with optional solid caps. The water fields
/// define its first layer; `additional_fluid_layers` contains only subsequent layers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseDefinition {
    pub title: String,
    pub mode_solver: ModeSolver,
    pub frequency_hz: f64,
    /// Nominal frequency for scaling the mesh; `None` uses `frequency_hz`.
    /// Legacy broadband inputs retain freq0 here, including for automatic meshes.
    pub mesh_reference_frequency_hz: Option<f64>,
    /// Absolute bottom depth of the first fluid, possibly below an elastic cap.
    pub water_depth_m: f64,
    pub interpolation: Interpolation,
    pub surface_boundary: SurfaceBoundary,
    /// Zero unless the surface is an A half-space.
    pub surface_sound_speed_mps: f64,
    /// Zero unless the surface is an A half-space.
    pub surface_density_g_cm3: f64,
    /// Surface half-space loss at the solve frequency, in dB/wavelength.
    pub surface_attenuation_db_per_wavelength: f64,
    /// Empty for `AnalyticMunk`; otherwise absolute depths from the first fluid top to its bottom.
    pub sound_speed_profile: Vec<SoundSpeedPoint>,
    pub water_density_g_cm3: f64,
    /// Empty for lossless water, otherwise one dB/wavelength value per SSP node.
    /// Values are evaluated at `frequency_hz` before complex SSP interpolation.
    pub water_attenuation_db_per_wavelength: Vec<f64>,
    pub additional_fluid_layers: Vec<FluidLayer>,
    /// RMS roughness in metres: fluid top, internal interfaces, fluid bottom.
    /// Empty means smooth; nonempty has fluid-layer count + 1 entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interface_roughness_m: Vec<f64>,
    /// Ordered elastic caps above the first fluid (top starts at 0).
    pub top_elastic_layers: Vec<ElasticLayer>,
    /// Ordered elastic layers below the last fluid.
    pub bottom_elastic_layers: Vec<ElasticLayer>,
    pub bottom_boundary: BottomBoundary,
    /// Zero for a non-A bottom (no half-space material).
    pub bottom_sound_speed_mps: f64,
    /// Zero for a non-A bottom (no half-space material).
    pub bottom_density_g_cm3: f64,
    /// Bottom half-space attenuation in dB per wavelength (0 for lossless, rigid or table).
    pub bottom_attenuation_db_per_wavelength: f64,
    pub source_geometry: SourceGeometry,
    pub mode_addition: ModeAddition,
    /// Empty for an omnidirectional source; otherwise ordered angle/amplitude samples.
    pub source_pattern: Vec<SourcePatternPoint>,
    /// Mesh intervals at the reference frequency, or 0 for 20 per last-SSP wavelength.
    pub mesh_points: usize,
    /// Lower phase-speed bound; zero lets the solver use the physical minimum.
    pub c_low_mps: f64,
    pub c_high_mps: f64,
    /// Eigenvalue extrapolation convergence control; 0 selects the base mesh only.
    pub max_range_m: f64,
    /// Depths at which the legacy mode file samples each eigenfunction.
    pub mode_sample_depths_m: Vec<f64>,
    pub mode_limit: usize,
    pub source_depths_m: Vec<f64>,
    pub receiver_depths_m: Vec<f64>,
    pub receiver_ranges_m: Vec<f64>,
    pub receiver_offsets_m: Vec<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldPropagation {
    RangeIndependent,
    Adiabatic,
    Coupled,
}

//! Input materials, from an ENV or a JSON case, own raw absorption and its power law until a solve
//! frequency is selected. Canonical Case fields never hold legacy loss units.
//!
//! Both loaders also share this module's rules for raw rows, half-spaces, power laws and volume
//! loss, its stacking of media, its depth snap and its per-frequency storage count. Each loader
//! words, locates and reports a rejection in its own style. The loaders keep their own syntax,
//! one-comparison checks and cumulative storage checks, so they do not accept identical inputs.
use crate::attenuation::{VolumeLoss, db_per_wavelength};
use crate::{
    Boundary, Case, CaseDefinition, DiagnosticReport, ElasticLayer, ElasticMaterialPoint,
    FluidLayer, SoundSpeedPoint,
};

type PowerLaw = (f64, f64, f64);

/// One raw medium row or half-space: depth, cp, cs, rho, ap, as.
type Row = [f64; 6];

pub(crate) const MAX_BIOLOGICAL_LAYERS: usize = 200;

/// Why a finite medium cannot take its next row.
pub(crate) enum RowFault {
    Depth,
    Material,
}

/// Judges `row` against the `rows` already accepted between `top` and `bottom`, depth order first.
#[allow(clippy::float_cmp)]
pub(crate) fn row_fault(rows: &[Row], row: &Row, top: f64, bottom: f64) -> Option<RowFault> {
    if row[0] < top || row[0] > bottom || rows.last().is_some_and(|p| row[0] <= p[0]) {
        return Some(RowFault::Depth);
    }
    if (rows.is_empty() && row[0] != top)
        || row[2] < 0.0
        || row[4] < 0.0
        || row[5] < 0.0
        || (row[2] == 0.0 && row[5] != 0.0)
        || rows.first().is_some_and(|p| {
            (p[2] == 0.0 && (row[3] != p[3] || row[2] != 0.0)) || (p[2] > 0.0 && row[2] == 0.0)
        })
    {
        return Some(RowFault::Material);
    }
    None
}

/// The material rule only: an ENV half-space must also sit at its boundary depth.
pub(crate) fn invalid_half_space(values: &Row) -> bool {
    values[1] <= 0.0
        || values[2] < 0.0
        || values[3] <= 0.0
        || values[4] < 0.0
        || values[5] < 0.0
        || (values[2] == 0.0 && values[5] != 0.0)
}

pub(crate) fn invalid_power_law(beta: f64, transition: f64) -> bool {
    beta < 0.0 || transition <= 0.0
}

pub(crate) fn invalid_francois_garrison(&[temperature, salinity, ph, depth]: &[f64; 4]) -> bool {
    temperature <= -273.0 || salinity < 0.0 || !(0.0..=14.0).contains(&ph) || depth < 0.0
}

pub(crate) fn invalid_biological_layer(p: &[f64; 5]) -> bool {
    p[0] < 0.0 || p[1] < p[0] || p[2] <= 0.0 || p[3] <= 0.0 || p[4] < 0.0
}

/// Shear speed makes a half-space elastic. Its shear loss waits for a solve frequency.
pub(crate) fn half_space_boundary(values: &Row) -> Boundary {
    if values[2] > 0.0 {
        Boundary::ElasticHalfSpace {
            shear_sound_speed_mps: values[2],
            shear_attenuation_db_per_wavelength: 0.0,
        }
    } else {
        Boundary::FluidHalfSpace
    }
}

pub(crate) struct Fluid {
    pub bottom_depth_m: f64,
    pub density_g_cm3: f64,
    pub mesh_points: usize,
    pub points: Vec<SoundSpeedPoint>,
    pub attenuation: Vec<f64>,
    pub power_law: PowerLaw,
}

pub(crate) struct Elastic {
    pub bottom_depth_m: f64,
    pub points: Vec<[f64; 6]>,
    pub mesh_points: usize,
    pub power_law: PowerLaw,
}

pub(crate) struct HalfSpace {
    pub values: [f64; 6],
    pub power_law: PowerLaw,
}

pub(crate) enum FiniteMedium {
    Fluid(Fluid),
    Elastic(Elastic),
}

impl FiniteMedium {
    /// Shear speed in the first row makes a medium elastic. Analytic water has no rows.
    pub fn new(
        bottom_depth_m: f64,
        mesh_points: usize,
        points: Vec<Row>,
        power_law: PowerLaw,
    ) -> Self {
        if points.first().is_some_and(|p| p[2] > 0.0) {
            return Self::Elastic(Elastic {
                bottom_depth_m,
                points,
                mesh_points,
                power_law,
            });
        }
        Self::Fluid(Fluid {
            bottom_depth_m,
            density_g_cm3: points.first().map_or(1.0, |p| p[3]),
            mesh_points,
            points: points
                .iter()
                .map(|p| SoundSpeedPoint {
                    depth_m: p[0],
                    sound_speed_mps: p[1],
                })
                .collect(),
            attenuation: points.iter().map(|p| p[4]).collect(),
            power_law,
        })
    }

    pub fn bottom_depth_m(&self) -> f64 {
        match self {
            Self::Fluid(layer) => layer.bottom_depth_m,
            Self::Elastic(layer) => layer.bottom_depth_m,
        }
    }
}

/// Finite media in depth order, sorted by where they fall around the first fluid.
#[derive(Default)]
pub(crate) struct Stack {
    water: Option<Fluid>,
    additional: Vec<Fluid>,
    top: Vec<Elastic>,
    bottom: Vec<Elastic>,
}

/// Where a pushed medium landed, by its index among the media sorted there.
pub(crate) enum Slot {
    Water,
    Additional(usize),
    Top(usize),
    Bottom(usize),
}

impl Stack {
    /// The stack's fluids must be contiguous, so a fluid cannot follow a bottom solid.
    pub fn splits_fluids(&self, medium: &FiniteMedium) -> bool {
        matches!(medium, FiniteMedium::Fluid(_)) && !self.bottom.is_empty()
    }

    pub fn push(&mut self, medium: FiniteMedium) -> Slot {
        match medium {
            FiniteMedium::Fluid(layer) if self.water.is_none() => {
                self.water = Some(layer);
                Slot::Water
            }
            FiniteMedium::Fluid(layer) => {
                self.additional.push(layer);
                Slot::Additional(self.additional.len() - 1)
            }
            FiniteMedium::Elastic(layer) if self.water.is_none() => {
                self.top.push(layer);
                Slot::Top(self.top.len() - 1)
            }
            FiniteMedium::Elastic(layer) => {
                self.bottom.push(layer);
                Slot::Bottom(self.bottom.len() - 1)
            }
        }
    }

    /// `None` when no medium was a fluid.
    pub fn into_media(self) -> Option<Media> {
        Some(Media {
            water: self.water?,
            additional: self.additional,
            top: self.top,
            bottom: self.bottom,
        })
    }
}

/// A finite stack that has its water.
pub(crate) struct Media {
    pub water: Fluid,
    pub additional: Vec<Fluid>,
    pub top: Vec<Elastic>,
    pub bottom: Vec<Elastic>,
}

pub(crate) struct Materials {
    pub media: Media,
    pub surface_half_space: HalfSpace,
    pub bottom_half_space: HalfSpace,
    pub unit: u8,
    pub volume: VolumeLoss,
}

struct Loss<'a> {
    unit: u8,
    volume: &'a VolumeLoss,
    frequency: f64,
}

impl Loss<'_> {
    fn convert(&self, raw: f64, depth: f64, speed: f64, power: PowerLaw) -> f64 {
        db_per_wavelength(
            self.unit,
            self.volume,
            raw,
            depth,
            speed,
            self.frequency,
            power,
        )
    }
}

impl Fluid {
    fn canonical(&self, loss: &Loss<'_>) -> FluidLayer {
        let mut attenuation: Vec<_> = self
            .points
            .iter()
            .zip(&self.attenuation)
            .map(|(p, &a)| loss.convert(a, p.depth_m, p.sound_speed_mps, self.power_law))
            .collect();
        if attenuation.iter().all(|&a| a == 0.0) {
            attenuation.clear();
        }
        FluidLayer {
            bottom_depth_m: self.bottom_depth_m,
            density_g_cm3: self.density_g_cm3,
            mesh_points: self.mesh_points,
            sound_speed_profile: self.points.clone(),
            attenuation_db_per_wavelength: attenuation,
        }
    }
}

impl Elastic {
    fn canonical(&self, loss: &Loss<'_>) -> ElasticLayer {
        let points: Vec<_> = self
            .points
            .iter()
            .map(|p| ElasticMaterialPoint {
                depth_m: p[0],
                compressional_sound_speed_mps: p[1],
                shear_sound_speed_mps: p[2],
                density_g_cm3: p[3],
                compressional_attenuation_db_per_wavelength: loss.convert(
                    p[4],
                    p[0],
                    p[1],
                    self.power_law,
                ),
                shear_attenuation_db_per_wavelength: loss.convert(p[5], p[0], p[2], self.power_law),
            })
            .collect();
        let first = points[0];
        let uniform = points.iter().all(|p| {
            ElasticMaterialPoint {
                depth_m: first.depth_m,
                ..*p
            } == first
        });
        ElasticLayer {
            bottom_depth_m: self.bottom_depth_m,
            compressional_sound_speed_mps: first.compressional_sound_speed_mps,
            shear_sound_speed_mps: first.shear_sound_speed_mps,
            density_g_cm3: first.density_g_cm3,
            compressional_attenuation_db_per_wavelength: first
                .compressional_attenuation_db_per_wavelength,
            shear_attenuation_db_per_wavelength: first.shear_attenuation_db_per_wavelength,
            mesh_points: self.mesh_points,
            material_profile: if uniform { Vec::new() } else { points },
        }
    }
}

impl HalfSpace {
    fn apply(&self, boundary: &mut Boundary, attenuation: &mut f64, loss: &Loss<'_>) {
        if boundary.is_half_space() {
            // UpdateHSLoss excludes depth-local biological layers using HUGE.
            *attenuation = loss.convert(self.values[4], f64::MAX, self.values[1], self.power_law);
        }
        if let Boundary::ElasticHalfSpace {
            shear_attenuation_db_per_wavelength,
            ..
        } = boundary
        {
            *shear_attenuation_db_per_wavelength =
                loss.convert(self.values[5], f64::MAX, self.values[2], self.power_law);
        }
    }
}

impl Materials {
    /// `surface` and `bottom` are half-space rows, all zero for any other boundary. The surface
    /// half-space has no power law of its own: it uses the water's.
    pub fn new(
        media: Media,
        surface: Row,
        bottom: Row,
        bottom_power_law: PowerLaw,
        unit: u8,
        volume: VolumeLoss,
    ) -> Self {
        Self {
            surface_half_space: HalfSpace {
                values: surface,
                power_law: media.water.power_law,
            },
            bottom_half_space: HalfSpace {
                values: bottom,
                power_law: bottom_power_law,
            },
            media,
            unit,
            volume,
        }
    }

    fn fluid_bottom(&self) -> f64 {
        self.media
            .additional
            .last()
            .map_or(self.media.water.bottom_depth_m, |layer| {
                layer.bottom_depth_m
            })
    }

    fn fluid_top(&self) -> f64 {
        self.media
            .top
            .last()
            .map_or(0.0, |layer| layer.bottom_depth_m)
    }

    /// Depths are single precision. Keep samples at either fluid interface on the exact f64
    /// boundary, whether the f32 spelling rounds up or down.
    pub fn snap_to_interfaces(&self, depths: &mut [f64]) {
        #[allow(clippy::cast_possible_truncation)]
        let boundaries =
            [self.fluid_bottom(), self.fluid_top()].map(|depth| (f64::from(depth as f32), depth));
        #[allow(clippy::float_cmp)]
        for depth in depths {
            if let Some(&(_, exact)) = boundaries.iter().find(|(rounded, _)| *depth == *rounded) {
                *depth = exact;
            }
        }
    }

    /// One solve frequency's share of the input storage budget for `template`, known before any
    /// frequency is converted, so it counts raw material where `case::input_values` counts
    /// canonical material.
    pub fn input_values(&self, template: &CaseDefinition) -> usize {
        self.material_values() + crate::case::template_values(template)
    }

    fn material_values(&self) -> usize {
        self.media
            .top
            .iter()
            .chain(&self.media.bottom)
            .map(|layer| {
                // Bound expanded row storage before converting each frequency; uniform caps
                // without depth-local loss retain the existing seven-value accounting.
                let expanded = matches!(self.volume, VolumeLoss::Biological(_))
                    || layer.points.windows(2).any(|p| p[0][1..] != p[1][1..]);
                7 + if expanded { 6 * layer.points.len() } else { 0 }
            })
            .sum::<usize>()
            + self.media.water.points.len()
            + self.media.water.attenuation.len()
            + self
                .media
                .additional
                .iter()
                .map(|layer| 1 + layer.points.len() + layer.attenuation.len())
                .sum::<usize>()
    }

    /// The validated Case at one solve frequency. A case with a mesh reference frequency scales its
    /// mesh for this frequency, so that mesh must be checked here too.
    pub fn case(
        &self,
        template: &CaseDefinition,
        frequency: f64,
    ) -> Result<Case, DiagnosticReport> {
        let case = Case::from_definition(self.case_definition(template, frequency))?;
        if case.mesh_reference_frequency_hz.is_some() {
            case.mesh_points_at(1)?;
        }
        Ok(case)
    }

    fn case_definition(&self, template: &CaseDefinition, frequency: f64) -> CaseDefinition {
        let mut input = template.clone();
        input.frequency_hz = frequency;
        let loss = Loss {
            unit: self.unit,
            volume: &self.volume,
            frequency,
        };
        let water = self.media.water.canonical(&loss);
        input.water_depth_m = water.bottom_depth_m;
        input.water_density_g_cm3 = water.density_g_cm3;
        input.mesh_points = water.mesh_points;
        input.sound_speed_profile = water.sound_speed_profile;
        input.water_attenuation_db_per_wavelength = water.attenuation_db_per_wavelength;
        input.additional_fluid_layers = self
            .media
            .additional
            .iter()
            .map(|layer| layer.canonical(&loss))
            .collect();
        input.top_elastic_layers = self
            .media
            .top
            .iter()
            .map(|layer| layer.canonical(&loss))
            .collect();
        input.bottom_elastic_layers = self
            .media
            .bottom
            .iter()
            .map(|layer| layer.canonical(&loss))
            .collect();
        input.surface_sound_speed_mps = self.surface_half_space.values[1];
        input.surface_density_g_cm3 = self.surface_half_space.values[3];
        input.bottom_sound_speed_mps = self.bottom_half_space.values[1];
        input.bottom_density_g_cm3 = self.bottom_half_space.values[3];
        self.surface_half_space.apply(
            &mut input.surface_boundary,
            &mut input.surface_attenuation_db_per_wavelength,
            &loss,
        );
        self.bottom_half_space.apply(
            &mut input.bottom_boundary,
            &mut input.bottom_attenuation_db_per_wavelength,
            &loss,
        );
        input
    }
}

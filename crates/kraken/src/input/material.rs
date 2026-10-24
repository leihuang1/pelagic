//! Input materials, from an ENV or a JSON case, own raw absorption and its power law until a solve
//! frequency is selected. Canonical Case fields never hold legacy loss units.
use crate::attenuation::{VolumeLoss, db_per_wavelength};
use crate::{
    Boundary, CaseDefinition, ElasticLayer, ElasticMaterialPoint, FluidLayer, SoundSpeedPoint,
};

type PowerLaw = (f64, f64, f64);

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

pub(crate) struct Materials {
    pub water: Fluid,
    pub additional: Vec<Fluid>,
    pub top: Vec<Elastic>,
    pub bottom: Vec<Elastic>,
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
    pub fn fluid_bottom(&self) -> f64 {
        self.additional
            .last()
            .map_or(self.water.bottom_depth_m, |layer| layer.bottom_depth_m)
    }

    pub fn fluid_top(&self) -> f64 {
        self.top.last().map_or(0.0, |layer| layer.bottom_depth_m)
    }

    pub fn input_values(&self) -> usize {
        self.top
            .iter()
            .chain(&self.bottom)
            .map(|layer| {
                // Bound expanded row storage before converting each frequency; uniform caps
                // without depth-local loss retain the existing seven-value accounting.
                let expanded = matches!(self.volume, VolumeLoss::Biological(_))
                    || layer.points.windows(2).any(|p| p[0][1..] != p[1][1..]);
                7 + if expanded { 6 * layer.points.len() } else { 0 }
            })
            .sum::<usize>()
            + self.water.points.len()
            + self.water.attenuation.len()
            + self
                .additional
                .iter()
                .map(|layer| 1 + layer.points.len() + layer.attenuation.len())
                .sum::<usize>()
    }

    pub fn case_definition(&self, template: &CaseDefinition, frequency: f64) -> CaseDefinition {
        let mut input = template.clone();
        input.frequency_hz = frequency;
        let loss = Loss {
            unit: self.unit,
            volume: &self.volume,
            frequency,
        };
        let water = self.water.canonical(&loss);
        input.water_depth_m = water.bottom_depth_m;
        input.water_density_g_cm3 = water.density_g_cm3;
        input.mesh_points = water.mesh_points;
        input.sound_speed_profile = water.sound_speed_profile;
        input.water_attenuation_db_per_wavelength = water.attenuation_db_per_wavelength;
        input.additional_fluid_layers = self
            .additional
            .iter()
            .map(|layer| layer.canonical(&loss))
            .collect();
        input.top_elastic_layers = self
            .top
            .iter()
            .map(|layer| layer.canonical(&loss))
            .collect();
        input.bottom_elastic_layers = self
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

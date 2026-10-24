#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use std::collections::HashMap;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use crate::case::validate_environment;
use crate::diagnostic::{Diagnostic, DiagnosticReport, LoadOutcome, SourceLocation};
use crate::model::{
    AttenuationUnit, BeamComponent, BeamFamily, BeamWidth, BiologicalLayer, Boundary,
    BoundaryCondition, CervenyOptions, CurvatureCondition, EnvironmentCase, HalfSpace,
    LegacyArrivalEncoding, Positions, ReceiverGrid, RunKind, RunOptions, SoundSpeedInput,
    SoundSpeedPoint, SourceGeometry, SspInterpolation, TopOptions, TraceOptions, VolumeAttenuation,
};

use super::records::{Atom, RecordReader, Slot, parse_f32, parse_f64, parse_i32};

const MAX_LEGACY_VECTOR: usize = 1_000_000;
const MAX_SSP_POINTS: usize = 100_001;
const SSP_BOTTOM_TOLERANCE_M: f64 = 100.0 * f32::EPSILON as f64;

pub(crate) struct ParsedEnvironment {
    pub value: EnvironmentCase,
    pub diagnostics: DiagnosticReport,
    pub locations: HashMap<&'static str, SourceLocation>,
}

pub(crate) fn parse_unvalidated(
    source: &str,
    path: &Path,
) -> Result<ParsedEnvironment, DiagnosticReport> {
    let parser = EnvironmentParser::new(source, path);
    let (value, diagnostics, locations) =
        parser.parse().map_err(DiagnosticReport::from_diagnostic)?;
    Ok(ParsedEnvironment {
        value,
        diagnostics,
        locations,
    })
}

pub(crate) fn parse(
    source: &str,
    path: &Path,
) -> Result<LoadOutcome<EnvironmentCase>, DiagnosticReport> {
    let ParsedEnvironment {
        value,
        mut diagnostics,
        locations,
    } = parse_unvalidated(source, path)?;
    validate_environment(&value, &locations, &mut diagnostics);

    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(LoadOutcome {
            value,
            warnings: diagnostics.diagnostics().to_vec(),
        })
    }
}

struct EnvironmentParser<'a> {
    path: PathBuf,
    reader: RecordReader<'a>,
    diagnostics: DiagnosticReport,
    locations: HashMap<&'static str, SourceLocation>,
    medium_defaults: MediumDefaults,
}

impl<'a> EnvironmentParser<'a> {
    fn new(source: &'a str, path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            reader: RecordReader::new(source, path),
            diagnostics: DiagnosticReport::default(),
            locations: HashMap::new(),
            medium_defaults: MediumDefaults::default(),
        }
    }

    fn parse(
        mut self,
    ) -> Result<
        (
            EnvironmentCase,
            DiagnosticReport,
            HashMap<&'static str, SourceLocation>,
        ),
        Diagnostic,
    > {
        let title = self.reader.read_string("title")?.text;
        let (frequency_hz, frequency_location) = self.reader.read_f64("frequency")?;
        self.locations.insert("frequency", frequency_location);
        let (medium_count, medium_count_location) = self.reader.read_i32("medium_count")?;
        if medium_count != 1 {
            return Err(Diagnostic::error(
                "BH0202",
                "two-dimensional BELLHOP supports exactly one water medium",
                "medium_count",
                medium_count_location,
            ));
        }

        let top_atom = self.reader.read_string("top_options")?;
        let top_legacy = fixed_width(&top_atom.text, 6);
        let top_characters: Vec<char> = top_legacy.chars().collect();
        let interpolation = parse_ssp_interpolation(top_characters[0], &top_atom.location)?;
        let attenuation_unit = parse_attenuation_unit(top_characters[2], &top_atom.location)?;
        let volume_attenuation =
            self.parse_volume_attenuation(top_characters[3], &top_atom.location)?;
        let has_altimetry = parse_shape_marker(
            top_characters[4],
            "top_options.altimetry",
            &top_atom.location,
        )?;
        let development_options = match top_characters[5] {
            'I' => true,
            ' ' => false,
            other => {
                return Err(invalid_option(
                    "top_options.development",
                    other,
                    "blank or I",
                    &top_atom.location,
                ));
            }
        };

        let mut top_boundary = self.parse_boundary(
            &top_legacy,
            top_characters[1],
            0.0,
            has_altimetry,
            "top_boundary",
            &top_atom.location,
        )?;

        let environment_header = required_atoms(
            self.reader.read_fields("sound_speed.header", 3)?,
            3,
            "sound_speed.header",
            &self.path,
        )?;
        let nominal_point_count =
            parse_i32(&environment_header[0], "sound_speed.nominal_point_count")?;
        let surface_roughness_m =
            parse_f32(&environment_header[1], "sound_speed.surface_roughness")?;
        let bottom_depth_m = parse_f64(&environment_header[2], "sound_speed.bottom_depth")?;
        self.locations.insert(
            "sound_speed.bottom_depth_m",
            environment_header[2].location.clone(),
        );
        top_boundary.roughness_m = surface_roughness_m;

        let points = if interpolation == SspInterpolation::AnalyticMunk {
            Vec::new()
        } else {
            self.parse_sound_speed_points(bottom_depth_m)?
        };
        let top_depth_m = points.first().map_or(0.0, |point| point.depth_m);

        let bottom_fields = required_atoms(
            self.reader.read_fields("bottom_options", 2)?,
            2,
            "bottom_options",
            &self.path,
        )?;
        let bottom_legacy = fixed_width(&bottom_fields[0].text, 2);
        let bottom_characters: Vec<char> = bottom_legacy.chars().collect();
        let bottom_roughness_m = parse_f32(&bottom_fields[1], "bottom_options.roughness")?;
        let has_bathymetry = parse_shape_marker(
            bottom_characters[1],
            "bottom_options.bathymetry",
            &bottom_fields[0].location,
        )?;
        let bottom_boundary = self.parse_boundary(
            &bottom_legacy,
            bottom_characters[0],
            bottom_roughness_m,
            has_bathymetry,
            "bottom_boundary",
            &bottom_fields[0].location,
        )?;

        let source_depths_m = self.read_f64_vector("positions.source_depths_m", 1.0)?;
        let receiver_depths_m = self.read_f64_vector("positions.receiver_depths_m", 1.0)?;
        let receiver_ranges_m = self.read_f64_vector("positions.receiver_ranges_m", 1000.0)?;

        let run_atom = self.reader.read_string("run_options")?;
        let run =
            self.parse_run_options(&run_atom, receiver_depths_m.len(), receiver_ranges_m.len())?;
        let trace = self.parse_trace_options(
            &run,
            development_options,
            frequency_hz,
            top_depth_m,
            bottom_depth_m,
            receiver_ranges_m.last().copied().unwrap_or(0.0),
        )?;

        let case = EnvironmentCase {
            source_path: self.path.clone(),
            title,
            frequency_hz,
            top_options: TopOptions {
                legacy: top_legacy,
                interpolation,
                attenuation_unit,
                volume_attenuation,
                has_altimetry,
                development_options,
            },
            top_boundary,
            sound_speed: SoundSpeedInput {
                nominal_point_count,
                surface_roughness_m,
                top_depth_m,
                bottom_depth_m,
                points,
            },
            bottom_boundary,
            positions: Positions {
                source_depths_m,
                receiver_depths_m,
                receiver_ranges_m,
            },
            run,
            trace,
        };

        Ok((case, self.diagnostics, self.locations))
    }

    fn parse_volume_attenuation(
        &mut self,
        option: char,
        location: &SourceLocation,
    ) -> Result<VolumeAttenuation, Diagnostic> {
        match option {
            ' ' => Ok(VolumeAttenuation::None),
            'T' => Ok(VolumeAttenuation::Thorp),
            'F' => {
                let values = required_atoms(
                    self.reader
                        .read_fields("volume_attenuation.francois_garrison", 4)?,
                    4,
                    "volume_attenuation.francois_garrison",
                    &self.path,
                )?;
                Ok(VolumeAttenuation::FrancoisGarrison {
                    temperature_c: parse_f64(&values[0], "volume_attenuation.temperature")?,
                    salinity_psu: parse_f64(&values[1], "volume_attenuation.salinity")?,
                    ph: parse_f64(&values[2], "volume_attenuation.ph")?,
                    mean_depth_m: parse_f64(&values[3], "volume_attenuation.mean_depth")?,
                })
            }
            'B' => {
                let (count, count_location) =
                    self.reader.read_i32("volume_attenuation.layer_count")?;
                let count =
                    checked_count(count, "volume_attenuation.layer_count", &count_location)?;
                let mut layers = Vec::with_capacity(count);
                for _ in 0..count {
                    let values = required_atoms(
                        self.reader.read_fields("volume_attenuation.layer", 5)?,
                        5,
                        "volume_attenuation.layer",
                        &self.path,
                    )?;
                    layers.push(BiologicalLayer {
                        top_depth_m: parse_f64(&values[0], "volume_attenuation.layer.top_depth")?,
                        bottom_depth_m: parse_f64(
                            &values[1],
                            "volume_attenuation.layer.bottom_depth",
                        )?,
                        resonance_frequency_hz: parse_f64(
                            &values[2],
                            "volume_attenuation.layer.resonance_frequency",
                        )?,
                        quality_factor: parse_f64(
                            &values[3],
                            "volume_attenuation.layer.quality_factor",
                        )?,
                        attenuation: parse_f64(&values[4], "volume_attenuation.layer.attenuation")?,
                    });
                }
                Ok(VolumeAttenuation::Biological { layers })
            }
            other => Err(invalid_option(
                "top_options.volume_attenuation",
                other,
                "blank, T, F, or B",
                location,
            )),
        }
    }

    fn parse_boundary(
        &mut self,
        legacy_options: &str,
        condition: char,
        roughness_m: f32,
        has_shape_file: bool,
        field: &'static str,
        location: &SourceLocation,
    ) -> Result<Boundary, Diagnostic> {
        let condition = match condition {
            'V' => BoundaryCondition::Vacuum,
            'R' => BoundaryCondition::Rigid,
            'A' => BoundaryCondition::AcoustoElastic(self.read_half_space(field)?),
            'G' => {
                let values = required_atoms(
                    self.reader.read_fields("boundary.grain_size", 2)?,
                    2,
                    "boundary.grain_size",
                    &self.path,
                )?;
                BoundaryCondition::GrainSize {
                    depth_m: parse_f64(&values[0], "boundary.grain_size.depth")?,
                    phi: parse_f64(&values[1], "boundary.grain_size.phi")?,
                }
            }
            'F' => BoundaryCondition::ReflectionCoefficientFile,
            'W' => BoundaryCondition::WriteReflectionCoefficient,
            'P' => BoundaryCondition::PrecalculatedReflectionCoefficient,
            other => {
                return Err(invalid_option(
                    field,
                    other,
                    "V, R, A, G, F, W, or P",
                    location,
                ));
            }
        };

        Ok(Boundary {
            legacy_options: legacy_options.to_owned(),
            condition,
            roughness_m,
            has_shape_file,
        })
    }

    fn read_half_space(&mut self, field: &'static str) -> Result<HalfSpace, Diagnostic> {
        let slots = self.reader.read_fields("boundary.half_space", 6)?;
        self.medium_defaults.apply(&slots, field, false)?;
        Ok(self.medium_defaults.as_half_space())
    }

    fn parse_sound_speed_points(
        &mut self,
        bottom_depth_m: f64,
    ) -> Result<Vec<SoundSpeedPoint>, Diagnostic> {
        let mut points: Vec<SoundSpeedPoint> = Vec::new();
        for _ in 0..MAX_SSP_POINTS {
            let slots = self.reader.read_fields("sound_speed.point", 6)?;
            self.locations
                .entry("sound_speed.points")
                .or_insert_with(|| first_atom_location(&slots, &self.path));
            self.medium_defaults
                .apply(&slots, "sound_speed.point", true)?;
            let point = self.medium_defaults.as_sound_speed_point();

            if let Some(previous) = points.last()
                && point.depth_m <= previous.depth_m
            {
                let location = first_atom_location(&slots, &self.path);
                return Err(Diagnostic::error(
                    "BH0201",
                    "sound-speed depths must be strictly increasing",
                    "sound_speed.point.depth",
                    location,
                ));
            }
            let at_bottom = (point.depth_m - bottom_depth_m).abs() < SSP_BOTTOM_TOLERANCE_M;
            points.push(point);
            if at_bottom {
                if points.len() < 2 {
                    return Err(Diagnostic::error(
                        "BH0201",
                        "the sound-speed profile must contain at least two points",
                        "sound_speed.points",
                        first_atom_location(&slots, &self.path),
                    ));
                }
                return Ok(points);
            }
        }

        Err(Diagnostic::error(
            "BH0201",
            format!("sound-speed profile exceeds {MAX_SSP_POINTS} points"),
            "sound_speed.points",
            SourceLocation::file(&self.path),
        ))
    }

    fn read_f64_vector(&mut self, field: &'static str, scale: f64) -> Result<Vec<f64>, Diagnostic> {
        let (count, location) = self.reader.read_i32(field)?;
        let count = checked_count(count, field, &location)?;
        let slots = self.reader.read_fields(field, count)?;
        self.locations
            .insert(field, first_atom_location(&slots, &self.path));
        let mut values = expand_f64_vector(&slots, count, field, &self.path)?;
        let was_sorted = values.windows(2).all(|pair| pair[0] <= pair[1]);
        values.sort_by(f64::total_cmp);
        if !was_sorted {
            self.diagnostics.push(Diagnostic::warning(
                "BH1002",
                "legacy BELLHOP sorts this vector before use",
                field,
                location,
            ));
        }
        for value in &mut values {
            *value *= scale;
        }
        Ok(values)
    }

    fn parse_run_options(
        &mut self,
        atom: &Atom,
        receiver_depth_count: usize,
        receiver_range_count: usize,
    ) -> Result<RunOptions, Diagnostic> {
        let legacy = fixed_width(&atom.text, 7);
        let characters: Vec<char> = legacy.chars().collect();
        let (kind, arrival_encoding) = match characters[0] {
            'R' => (RunKind::Rays, None),
            'E' => (RunKind::Eigenrays, None),
            'C' => (RunKind::Coherent, None),
            'S' => (RunKind::SemiCoherent, None),
            'I' => (RunKind::Incoherent, None),
            'A' => (RunKind::Arrivals, Some(LegacyArrivalEncoding::Ascii)),
            'a' => (RunKind::Arrivals, Some(LegacyArrivalEncoding::Binary)),
            other => {
                return Err(invalid_option(
                    "run_options.kind",
                    other,
                    "R, E, C, S, I, A, or a",
                    &atom.location,
                ));
            }
        };

        let beam_family = if kind == RunKind::Rays {
            None
        } else {
            Some(match characters[1] {
                'C' => BeamFamily::CervenyCartesian,
                'R' => BeamFamily::CervenyRayCentered,
                'S' => BeamFamily::SimpleGaussian,
                'b' => BeamFamily::GeometricGaussianRayCentered,
                'B' => BeamFamily::GeometricGaussianCartesian,
                'g' => BeamFamily::GeometricHatRayCentered,
                'G' | '^' | ' ' => BeamFamily::GeometricHatCartesian,
                other => {
                    self.diagnostics.push(Diagnostic::warning(
                        "BH1001",
                        format!(
                            "unknown beam family {other:?}; legacy BELLHOP uses geometric hat beams"
                        ),
                        "run_options.beam_family",
                        atom.location.clone(),
                    ));
                    BeamFamily::GeometricHatCartesian
                }
            })
        };

        let source_geometry = match characters[3] {
            'X' => SourceGeometry::Line,
            'R' | ' ' => SourceGeometry::Point,
            other => {
                self.diagnostics.push(Diagnostic::warning(
                    "BH1001",
                    format!("unknown source geometry {other:?}; using point source"),
                    "run_options.source_geometry",
                    atom.location.clone(),
                ));
                SourceGeometry::Point
            }
        };
        let receiver_grid = match characters[4] {
            'I' => ReceiverGrid::Irregular,
            'R' | ' ' => ReceiverGrid::Rectilinear,
            other => {
                self.diagnostics.push(Diagnostic::warning(
                    "BH1001",
                    format!("unknown receiver grid {other:?}; using rectilinear grid"),
                    "run_options.receiver_grid",
                    atom.location.clone(),
                ));
                ReceiverGrid::Rectilinear
            }
        };
        if receiver_grid == ReceiverGrid::Irregular && receiver_depth_count != receiver_range_count
        {
            return Err(Diagnostic::error(
                "BH0201",
                "irregular receiver grids require equal depth and range counts",
                "run_options.receiver_grid",
                atom.location.clone(),
            ));
        }

        match characters[5] {
            '3' => {
                return Err(Diagnostic::error(
                    "BH0202",
                    "three-dimensional runs are not supported",
                    "run_options.dimensionality",
                    atom.location.clone(),
                ));
            }
            '2' | ' ' => {}
            other => self.diagnostics.push(Diagnostic::warning(
                "BH1001",
                format!("unknown dimensionality {other:?}; using 2D"),
                "run_options.dimensionality",
                atom.location.clone(),
            )),
        }

        Ok(RunOptions {
            legacy,
            kind,
            arrival_encoding,
            beam_family,
            has_source_beam_pattern: characters[2] == '*',
            source_geometry,
            receiver_grid,
            beam_shift: characters[6] == 'S',
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_trace_options(
        &mut self,
        run: &RunOptions,
        development_options: bool,
        frequency_hz: f64,
        top_depth_m: f64,
        bottom_depth_m: f64,
        maximum_receiver_range_m: f64,
    ) -> Result<TraceOptions, Diagnostic> {
        let (mut launch_count, selected_launch_angle, count_location) = if development_options {
            let values = required_atoms(
                self.reader.read_fields("trace.launch_count", 2)?,
                2,
                "trace.launch_count",
                &self.path,
            )?;
            let count = parse_i32(&values[0], "trace.launch_count")?;
            let selected = parse_i32(&values[1], "trace.selected_launch_angle")?;
            let selected = usize::try_from(selected).map_err(|_| {
                Diagnostic::error(
                    "BH0201",
                    "selected launch angle must be positive",
                    "trace.selected_launch_angle",
                    values[1].location.clone(),
                )
            })?;
            (count, Some(selected), values[0].location.clone())
        } else {
            let (count, location) = self.reader.read_i32("trace.launch_count")?;
            (count, None, location)
        };

        self.locations
            .insert("trace.launch_count", count_location.clone());
        if launch_count < 0 {
            return Err(Diagnostic::error(
                "BH0201",
                "launch-angle count cannot be negative",
                "trace.launch_count",
                count_location,
            ));
        }
        if launch_count == 0 {
            launch_count = automatic_launch_count(
                run.kind,
                frequency_hz,
                bottom_depth_m - top_depth_m,
                maximum_receiver_range_m,
            );
        }
        let launch_count = checked_count(launch_count, "trace.launch_count", &count_location)?;
        let slots = self
            .reader
            .read_fields("trace.launch_angles", launch_count)?;
        self.locations.insert(
            "trace.launch_angles_degrees",
            first_atom_location(&slots, &self.path),
        );
        let mut launch_angles_degrees =
            expand_f64_vector(&slots, launch_count, "trace.launch_angles", &self.path)?;
        launch_angles_degrees.sort_by(f64::total_cmp);
        if launch_angles_degrees.len() > 1 {
            let sweep =
                launch_angles_degrees[launch_angles_degrees.len() - 1] - launch_angles_degrees[0];
            if sweep.rem_euclid(360.0).abs() < 10.0 * f64::EPSILON * 360.0 {
                launch_angles_degrees.pop();
            }
        }
        if let Some(selected) = selected_launch_angle
            && (selected == 0 || selected > launch_angles_degrees.len())
        {
            return Err(Diagnostic::error(
                "BH0201",
                format!(
                    "selected launch angle must be in 1..={}",
                    launch_angles_degrees.len()
                ),
                "trace.selected_launch_angle",
                count_location.clone(),
            ));
        }

        let limits = required_atoms(
            self.reader.read_fields("trace.limits", 3)?,
            3,
            "trace.limits",
            &self.path,
        )?;
        let step_m = parse_f64(&limits[0], "trace.step")?;
        let max_depth_m = parse_f64(&limits[1], "trace.max_depth")?;
        let max_range_m = 1000.0 * parse_f64(&limits[2], "trace.max_range")?;
        self.locations
            .insert("trace.step", limits[0].location.clone());
        self.locations
            .insert("trace.max_depth", limits[1].location.clone());
        self.locations
            .insert("trace.max_range", limits[2].location.clone());

        let cerveny = match run.beam_family {
            Some(BeamFamily::CervenyCartesian | BeamFamily::CervenyRayCentered) => {
                let values = required_atoms(
                    self.reader.read_fields("trace.cerveny", 3)?,
                    3,
                    "trace.cerveny",
                    &self.path,
                )?;
                let options = fixed_width(&values[0].text, 2);
                let option_characters: Vec<char> = options.chars().collect();
                let width = match option_characters[0] {
                    'F' => BeamWidth::SpaceFilling,
                    'M' => BeamWidth::Minimum,
                    'W' => BeamWidth::Wkb,
                    other => {
                        return Err(invalid_option(
                            "trace.cerveny.width",
                            other,
                            "F, M, or W",
                            &values[0].location,
                        ));
                    }
                };
                let curvature = match option_characters[1] {
                    'D' => CurvatureCondition::Double,
                    'S' => CurvatureCondition::Standard,
                    'Z' => CurvatureCondition::Zero,
                    other => {
                        return Err(invalid_option(
                            "trace.cerveny.curvature",
                            other,
                            "D, S, or Z",
                            &values[0].location,
                        ));
                    }
                };
                let epsilon_multiplier = parse_f64(&values[1], "trace.cerveny.epsilon_multiplier")?;
                let loop_range = parse_f64(&values[2], "trace.cerveny.loop_range")?;

                let image_values = required_atoms(
                    self.reader.read_fields("trace.cerveny.images", 3)?,
                    3,
                    "trace.cerveny.images",
                    &self.path,
                )?;
                let component = match image_values[2].text.chars().next().unwrap_or('P') {
                    'P' => BeamComponent::Pressure,
                    'V' => BeamComponent::Vertical,
                    'H' => BeamComponent::Horizontal,
                    'D' => BeamComponent::Displacement,
                    other => {
                        return Err(invalid_option(
                            "trace.cerveny.component",
                            other,
                            "P, V, H, or D",
                            &image_values[2].location,
                        ));
                    }
                };
                Some(CervenyOptions {
                    width,
                    curvature,
                    epsilon_multiplier,
                    loop_range,
                    image_count: parse_i32(&image_values[0], "trace.cerveny.image_count")?,
                    beam_window: parse_i32(&image_values[1], "trace.cerveny.beam_window")?,
                    component,
                })
            }
            _ => None,
        };

        Ok(TraceOptions {
            launch_angles_degrees,
            selected_launch_angle,
            step_m,
            max_depth_m,
            max_range_m,
            cerveny,
        })
    }
}

#[derive(Clone, Debug)]
struct MediumDefaults {
    depth_m: f64,
    compressional_speed_mps: f64,
    shear_speed_mps: f64,
    density_g_cm3: f64,
    compressional_attenuation: f64,
    shear_attenuation: f64,
}

impl Default for MediumDefaults {
    fn default() -> Self {
        Self {
            depth_m: 0.0,
            compressional_speed_mps: 1500.0,
            shear_speed_mps: 0.0,
            density_g_cm3: 1.0,
            compressional_attenuation: 0.0,
            shear_attenuation: 0.0,
        }
    }
}

impl MediumDefaults {
    fn apply(
        &mut self,
        slots: &[Slot],
        field: &'static str,
        require_depth: bool,
    ) -> Result<(), Diagnostic> {
        if require_depth && (slots.is_empty() || slots[0].atom.is_none()) {
            return Err(Diagnostic::error(
                "BH0104",
                "depth is required",
                field,
                slots
                    .iter()
                    .find_map(|slot| slot.atom.as_ref().map(|atom| atom.location.clone()))
                    .unwrap_or_else(|| SourceLocation::file("<input>")),
            ));
        }
        for (index, slot) in slots.iter().enumerate() {
            let Some(atom) = &slot.atom else {
                continue;
            };
            let value = parse_f64(atom, field)?;
            match index {
                0 => self.depth_m = value,
                1 => self.compressional_speed_mps = value,
                2 => self.shear_speed_mps = value,
                3 => self.density_g_cm3 = value,
                4 => self.compressional_attenuation = value,
                5 => self.shear_attenuation = value,
                _ => unreachable!(),
            }
        }
        Ok(())
    }

    fn as_half_space(&self) -> HalfSpace {
        HalfSpace {
            depth_m: self.depth_m,
            compressional_speed_mps: self.compressional_speed_mps,
            shear_speed_mps: self.shear_speed_mps,
            density_g_cm3: self.density_g_cm3,
            compressional_attenuation: self.compressional_attenuation,
            shear_attenuation: self.shear_attenuation,
        }
    }

    fn as_sound_speed_point(&self) -> SoundSpeedPoint {
        SoundSpeedPoint {
            depth_m: self.depth_m,
            compressional_speed_mps: self.compressional_speed_mps,
            shear_speed_mps: self.shear_speed_mps,
            density_g_cm3: self.density_g_cm3,
            compressional_attenuation: self.compressional_attenuation,
            shear_attenuation: self.shear_attenuation,
        }
    }
}

fn parse_ssp_interpolation(
    option: char,
    location: &SourceLocation,
) -> Result<SspInterpolation, Diagnostic> {
    match option {
        'N' => Ok(SspInterpolation::N2Linear),
        'C' => Ok(SspInterpolation::CLinear),
        'P' => Ok(SspInterpolation::Pchip),
        'S' => Ok(SspInterpolation::CubicSpline),
        'Q' => Ok(SspInterpolation::Quadrilateral),
        'A' => Ok(SspInterpolation::AnalyticMunk),
        'H' => Err(Diagnostic::error(
            "BH0202",
            "hexahedral sound-speed interpolation is only available in BELLHOP3D",
            "top_options.ssp_interpolation",
            location.clone(),
        )),
        other => Err(invalid_option(
            "top_options.ssp_interpolation",
            other,
            "N, C, P, S, Q, or A",
            location,
        )),
    }
}

fn parse_attenuation_unit(
    option: char,
    location: &SourceLocation,
) -> Result<AttenuationUnit, Diagnostic> {
    match option {
        'N' => Ok(AttenuationUnit::NepersPerMeter),
        'F' => Ok(AttenuationUnit::DbPerMeterKhz),
        'M' => Ok(AttenuationUnit::DbPerMeter),
        'W' => Ok(AttenuationUnit::DbPerWavelength),
        'Q' => Ok(AttenuationUnit::QualityFactor),
        'L' => Ok(AttenuationUnit::LossParameter),
        other => Err(invalid_option(
            "top_options.attenuation_unit",
            other,
            "N, F, M, W, Q, or L",
            location,
        )),
    }
}

fn parse_shape_marker(
    option: char,
    field: &'static str,
    location: &SourceLocation,
) -> Result<bool, Diagnostic> {
    match option {
        '~' | '*' => Ok(true),
        '-' | '_' | ' ' => Ok(false),
        other => Err(invalid_option(
            field,
            other,
            "blank, -, _, ~, or *",
            location,
        )),
    }
}

fn fixed_width(value: &str, width: usize) -> String {
    let mut result: String = value.chars().take(width).collect();
    result.extend(std::iter::repeat_n(
        ' ',
        width.saturating_sub(result.chars().count()),
    ));
    result
}

fn invalid_option(
    field: &'static str,
    option: char,
    expected: &str,
    location: &SourceLocation,
) -> Diagnostic {
    Diagnostic::error(
        "BH0105",
        format!("invalid option {option:?}; expected {expected}"),
        field,
        location.clone(),
    )
}

fn required_atoms(
    slots: Vec<Slot>,
    expected: usize,
    field: &'static str,
    path: &Path,
) -> Result<Vec<Atom>, Diagnostic> {
    if slots.len() < expected {
        return Err(Diagnostic::error(
            "BH0104",
            format!("expected {expected} value(s), found {}", slots.len()),
            field,
            first_atom_location(&slots, path),
        ));
    }
    slots
        .into_iter()
        .enumerate()
        .map(|(index, slot)| {
            slot.atom.ok_or_else(|| {
                Diagnostic::error(
                    "BH0104",
                    format!("value {} cannot be omitted", index + 1),
                    field,
                    SourceLocation::file(path),
                )
            })
        })
        .collect()
}

fn checked_count(
    count: i32,
    field: &'static str,
    location: &SourceLocation,
) -> Result<usize, Diagnostic> {
    if count <= 0 {
        return Err(Diagnostic::error(
            "BH0201",
            "count must be positive",
            field,
            location.clone(),
        ));
    }
    let count = usize::try_from(count).map_err(|_| {
        Diagnostic::error(
            "BH0201",
            "count is outside the supported range",
            field,
            location.clone(),
        )
    })?;
    if count > MAX_LEGACY_VECTOR {
        return Err(Diagnostic::error(
            "BH0201",
            format!("count exceeds the parser limit of {MAX_LEGACY_VECTOR}"),
            field,
            location.clone(),
        ));
    }
    Ok(count)
}

fn expand_f64_vector(
    slots: &[Slot],
    count: usize,
    field: &'static str,
    path: &Path,
) -> Result<Vec<f64>, Diagnostic> {
    let explicit = parse_explicit_f64(slots, field)?;
    if explicit.len() == count {
        return Ok(explicit);
    }
    if count >= 3 && (explicit.len() == 1 || explicit.len() == 2) {
        let first = explicit[0];
        let last = explicit.get(1).copied().unwrap_or(first);
        let denominator = (count - 1) as f64;
        let delta = (last - first) / denominator;
        return Ok((0..count)
            .map(|index| first + index as f64 * delta)
            .collect());
    }
    Err(incomplete_vector(field, count, explicit.len(), slots, path))
}

fn parse_explicit_f64(slots: &[Slot], field: &'static str) -> Result<Vec<f64>, Diagnostic> {
    slots
        .iter()
        .map_while(|slot| slot.atom.as_ref())
        .map(|atom| parse_f64(atom, field))
        .collect()
}

fn incomplete_vector(
    field: &'static str,
    expected: usize,
    actual: usize,
    slots: &[Slot],
    path: &Path,
) -> Diagnostic {
    Diagnostic::error(
        "BH0104",
        format!(
            "expected {expected} explicit values, or one/two endpoints for subtabulation; found {actual}"
        ),
        field,
        first_atom_location(slots, path),
    )
}

fn first_atom_location(slots: &[Slot], path: &Path) -> SourceLocation {
    slots
        .iter()
        .find_map(|slot| slot.atom.as_ref().map(|atom| atom.location.clone()))
        .unwrap_or_else(|| SourceLocation::file(path))
}

pub(crate) fn automatic_launch_count(
    run_kind: RunKind,
    frequency_hz: f64,
    water_depth_m: f64,
    maximum_receiver_range_m: f64,
) -> i32 {
    if run_kind == RunKind::Rays {
        return 50;
    }
    let phase_limit = (0.3 * maximum_receiver_range_m * frequency_hz / 1500.0) as i32;
    let mut count = phase_limit.max(300);
    if maximum_receiver_range_m > 0.0 && water_depth_m > 0.0 {
        let recommended_spacing = (water_depth_m / (10.0 * maximum_receiver_range_m)).atan();
        if recommended_spacing > 0.0 {
            count = count.max((PI / recommended_spacing) as i32);
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::model::{BeamFamily, RunKind, SspInterpolation, VolumeAttenuation};

    use super::parse;

    const BASIC: &str = r"
'Calibration case'
250.0
1
'CVW'
0 0.0 100.0
0.0 1500.0 /
100.0 1500.0 /
'A' 0.0
100.0 1590.0 0.0 1.2 0.5 /
1
50.0 /
3
0.0 100.0 /
3
0.0 5.0 /
'R'
3
-10.0 10.0 /
0.0 101.0 5.1
";

    #[test]
    fn parses_and_subtabulates_basic_ray_case() {
        let outcome = parse(BASIC, Path::new("basic.env")).unwrap();
        let case = outcome.value;
        assert_eq!(case.title, "Calibration case");
        assert_eq!(case.top_options.interpolation, SspInterpolation::CLinear);
        assert_eq!(case.run.kind, RunKind::Rays);
        assert_eq!(case.positions.receiver_depths_m, vec![0.0, 50.0, 100.0]);
        assert_eq!(case.positions.receiver_ranges_m, vec![0.0, 2500.0, 5000.0]);
        assert_eq!(case.trace.launch_angles_degrees, vec![-10.0, 0.0, 10.0]);
    }

    #[test]
    fn parses_francois_garrison_and_default_beam_family() {
        let input = BASIC
            .replace("'CVW'", "'CVWF'")
            .replace("0 0.0 100.0", "19.3 33.5 7.5 50.0\n0 0.0 100.0")
            .replace("'R'", "'C'");
        let outcome = parse(&input, Path::new("volume.env")).unwrap();
        assert!(matches!(
            outcome.value.top_options.volume_attenuation,
            VolumeAttenuation::FrancoisGarrison { .. }
        ));
        assert_eq!(
            outcome.value.run.beam_family,
            Some(BeamFamily::GeometricHatCartesian)
        );
    }

    #[test]
    fn reports_the_actual_line_for_semantic_errors() {
        let input = BASIC.replace("250.0", "-1.0");
        let report = parse(&input, Path::new("bad-frequency.env")).unwrap_err();
        let diagnostic = &report.diagnostics()[0];
        assert_eq!(diagnostic.field.as_deref(), Some("frequency_hz"));
        assert_eq!(diagnostic.location.line, 3);
    }

    #[test]
    fn rejects_3d_input() {
        let input = BASIC.replace("'R'", "'R    3'");
        let report = parse(&input, Path::new("three-d.env")).unwrap_err();
        assert!(report.to_string().contains("three-dimensional"));
    }

    #[test]
    fn rejects_non_monotonic_sound_speed_depths() {
        let input = BASIC.replace("100.0 1500.0 /", "-1.0 1500.0 /\n100.0 1500.0 /");
        let report = parse(&input, Path::new("bad.env")).unwrap_err();
        assert!(report.to_string().contains("strictly increasing"));
    }
}

//! KRAKEN/KRAKENC runs published as protected result directories, with HDF5 v1 as the
//! frequency-block stream's block writer.
//! This schema is independent of BELLHOP schema v3; no solver result types are unified.

use std::fs;
use std::path::{Path, PathBuf};

use hdf5::{File, Group, H5Type};
use kraken::input::{FieldInput, InputSnapshot};
use kraken::{
    Case, FieldCase, FieldPropagation, ModeAddition, ModeSet, ModeSolver, PressureField,
    ProfileSimulationResult, SourceGeometry,
};
use native_output::kraken::stream;
pub use native_output::kraken::stream::{RunError, RunSummary};
use sha2::{Digest, Sha256};

use super::hdf5::{hdf5_error, write_scalar_attribute, write_string_attribute};

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;

fn check_field_payload(cases: &[FieldCase], max_output_bytes: u64) -> Result<(), RunError> {
    if max_output_bytes == 0 {
        return Err(RunError::Output("max_output_bytes must be positive".into()));
    }
    // Admit the known FIELD payload before doing numerical work. Modes are charged
    // as they are written, because their counts are not known before solving.
    let pressure_bytes = cases.iter().try_fold(0_u64, |total, sequence| {
        let case = &sequence.profiles()[0];
        total.checked_add(
            (case.source_depths_m.len() as u64)
                .checked_mul(case.receiver_depths_m.len() as u64)?
                .checked_mul(case.receiver_ranges_m.len() as u64)?
                .checked_mul(8)?,
        )
    });
    if pressure_bytes.is_none_or(|bytes| bytes > max_output_bytes) {
        return Err(RunError::Output(format!(
            "FIELD payload exceeds {max_output_bytes} bytes"
        )));
    }
    Ok(())
}

/// Solve a loaded input once and publish selected CLI formats as a protected file group.
/// # Errors
/// Returns ordered numerical, quota or recoverable publication errors.
pub fn run_directory(
    input: &FieldInput,
    output: &Path,
    format: super::directory::Format,
    maximum: u64,
) -> Result<RunSummary, RunError> {
    let stem = super::directory::stem(input.path())?;
    let cases = input.cases();
    check_field_payload(cases, maximum)?;
    let consumed: Vec<&Path> = input
        .consumed_inputs()
        .iter()
        .map(PathBuf::as_path)
        .collect();
    super::directory::publish(output, &consumed, Some(maximum), |scratch| {
        let inputs = input.snapshots();
        if format.hdf5() {
            run_with_hdf5(cases, inputs, scratch, &stem, format, maximum)
        } else {
            stream::run(cases, inputs, scratch, &stem, format, maximum, &mut ())
        }
    })
}

/// The HDF5 layout around the stream: its header precedes the run report, and its totals follow
/// the last block.
fn run_with_hdf5(
    cases: &[FieldCase],
    inputs: &[InputSnapshot],
    root: &Path,
    stem: &str,
    format: super::directory::Format,
    maximum: u64,
) -> Result<RunSummary, RunError> {
    let path = root.join(format!("{stem}.h5"));
    let file = File::create_excl(&path).map_err(hdf5_error)?;
    let mut budget = Budget {
        payload: 0,
        maximum,
    };
    write_header(&file, cases, inputs, &mut budget)?;
    check_file(&file, &path, maximum)?;
    let mut blocks = Blocks {
        frequencies: file.create_group("frequencies").map_err(hdf5_error)?,
        budget,
        file: &file,
        path: &path,
    };
    let summary = stream::run(cases, inputs, root, stem, format, maximum, &mut blocks)?;
    drop(blocks);
    write_scalar_attribute(&file, "mode_count", &summary.mode_count)?;
    write_scalar_attribute(&file, "pressure_count", &summary.pressure_count)?;
    check_file(&file, &path, maximum)?;
    file.close().map_err(hdf5_error)?;
    Ok(summary)
}

/// One `/frequencies/{index}` group per block.
struct Blocks<'a> {
    frequencies: Group,
    budget: Budget,
    file: &'a File,
    path: &'a Path,
}

impl stream::BlockWriter for Blocks<'_> {
    fn write_block(
        &mut self,
        index: usize,
        case: &FieldCase,
        result: &ProfileSimulationResult,
    ) -> Result<(), String> {
        let group = self
            .frequencies
            .create_group(&index.to_string())
            .map_err(hdf5_error)?;
        let maximum = self.budget.maximum;
        write_frequency(&group, case, result, &mut self.budget, self.file, self.path)?;
        check_file(self.file, self.path, maximum)
    }

    fn check_block(&mut self) -> Result<(), String> {
        // Closing the block's group leaves metadata for this flush, which can grow the file. Keep it
        // out of write_block: the native and directory quotas must see the file before that growth.
        check_file(self.file, self.path, self.budget.maximum)
    }

    fn refresh_quota(&self, quota: &mut super::directory::Quota) -> Result<(), String> {
        quota.refresh(self.path)
    }
}

struct Budget {
    payload: u64,
    maximum: u64,
}

impl Budget {
    fn reserve(&mut self, bytes: u64) -> Result<(), String> {
        self.payload = self
            .payload
            .checked_add(bytes)
            .filter(|&n| n <= self.maximum)
            .ok_or_else(|| format!("dataset payload exceeds {} bytes", self.maximum))?;
        Ok(())
    }
}

fn check_file(file: &File, path: &Path, maximum: u64) -> Result<(), String> {
    // ponytail: physical quota checked after header/profile/frequency flushes; a bounded HDF5 VFD
    // is needed for a strict in-write disk quota, not for sequential bounded results.
    file.flush().map_err(hdf5_error)?;
    if fs::metadata(path).map_err(|error| error.to_string())?.len() > maximum {
        return Err(format!("HDF5 file exceeds {maximum} bytes"));
    }
    Ok(())
}

fn write_header(
    file: &File,
    cases: &[FieldCase],
    inputs: &[InputSnapshot],
    budget: &mut Budget,
) -> Result<(), String> {
    write_scalar_attribute(file, "schema_version", &SCHEMA_VERSION)?;
    write_string_attribute(file, "schema_name", "kraken")?;
    write_string_attribute(
        file,
        "implementation",
        concat!("Pelagic kraken ", env!("CARGO_PKG_VERSION")),
    )?;
    write_string_attribute(
        file,
        "compatibility_reference",
        "Acoustics Toolbox v2023.5 (475108519289c6fb488b58980c644ea14eccc604)",
    )?;
    write_string_attribute(file, "title", &cases[0].profiles()[0].title)?;
    write_string_attribute(
        file,
        "solver",
        match cases[0].profiles()[0].mode_solver {
            ModeSolver::Kraken => "kraken",
            ModeSolver::Krakenc => "krakenc",
        },
    )?;
    write_string_attribute(
        file,
        "coordinate_convention",
        "range origin at source; depth positive downward",
    )?;
    write_scalar_attribute(file, "frequency_count", &(cases.len() as u64))?;
    write_scalar_attribute(file, "max_output_bytes", &budget.maximum)?;
    let frequencies: Vec<_> = cases
        .iter()
        .map(|case| case.profiles()[0].frequency_hz)
        .collect();
    dataset(
        file,
        "frequency_hz",
        &frequencies,
        &[frequencies.len()],
        "Hz",
        budget,
    )?;
    let group = file.create_group("inputs").map_err(hdf5_error)?;
    for snapshot in inputs {
        let input = group.create_group(snapshot.role()).map_err(hdf5_error)?;
        write_string_attribute(&input, "filename", &snapshot.path().to_string_lossy())?;
        write_scalar_attribute(&input, "size_bytes", &(snapshot.source().len() as u64))?;
        write_string_attribute(
            &input,
            "sha256",
            &format!("{:x}", Sha256::digest(snapshot.source().as_bytes())),
        )?;
    }
    Ok(())
}

fn write_frequency(
    group: &Group,
    case: &FieldCase,
    result: &ProfileSimulationResult,
    budget: &mut Budget,
    file: &File,
    path: &Path,
) -> Result<(), String> {
    write_string_attribute(group, "title", &case.profiles()[0].title)?;
    write_profile_metadata(group, &case.profiles()[0], budget)?;
    write_modes(group, &result.modes[0], budget)?;
    write_field(group, &result.field, budget)?;
    write_scalar_attribute(group, "profile_count", &(case.profiles().len() as u64))?;
    write_string_attribute(
        group,
        "field_propagation",
        match case.propagation() {
            FieldPropagation::RangeIndependent => "range_independent",
            FieldPropagation::Adiabatic => "adiabatic",
            FieldPropagation::Coupled => "coupled",
        },
    )?;
    if case.profiles().len() > 1 {
        dataset(
            group,
            "profile_range_m",
            case.ranges_m(),
            &[case.profiles().len()],
            "m",
            budget,
        )?;
        let profiles = group.create_group("profiles").map_err(hdf5_error)?;
        for (index, (profile, modes)) in case.profiles().iter().zip(&result.modes).enumerate() {
            let child = profiles
                .create_group(&index.to_string())
                .map_err(hdf5_error)?;
            write_string_attribute(&child, "title", &profile.title)?;
            write_scalar_attribute(&child, "range_m", &case.ranges_m()[index])?;
            write_profile_metadata(&child, profile, budget)?;
            if index == 0 {
                group
                    .link_hard("modes", "profiles/0/modes")
                    .map_err(hdf5_error)?;
            } else {
                write_modes(&child, modes, budget)?;
            }
            check_file(file, path, budget.maximum)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // Keep each profile's material and interface metadata together.
fn write_profile_metadata(group: &Group, case: &Case, budget: &mut Budget) -> Result<(), String> {
    write_scalar_attribute(group, "frequency_hz", &case.frequency_hz)?;
    write_string_attribute(
        group,
        "surface_boundary",
        match &case.surface_boundary {
            kraken::SurfaceBoundary::Vacuum => "V",
            kraken::SurfaceBoundary::Rigid => "R",
            kraken::SurfaceBoundary::FluidHalfSpace
            | kraken::SurfaceBoundary::ElasticHalfSpace { .. } => "A",
            kraken::SurfaceBoundary::Reflection(_) => "F",
            kraken::SurfaceBoundary::Impedance { .. } => "P",
        },
    )?;
    write_string_attribute(
        group,
        "bottom_boundary",
        match &case.bottom_boundary {
            kraken::BottomBoundary::Vacuum => "V",
            kraken::BottomBoundary::FluidHalfSpace
            | kraken::BottomBoundary::ElasticHalfSpace { .. } => "A",
            kraken::BottomBoundary::Rigid => "R",
            kraken::BottomBoundary::Reflection(_) => "F",
            kraken::BottomBoundary::Impedance { .. } => "P",
        },
    )?;
    write_elastic_half_spaces(group, case)?;
    write_elastic_layers(group, case, budget)?;
    write_scalar_attribute(
        group,
        "mesh_reference_frequency_hz",
        &case
            .mesh_reference_frequency_hz
            .unwrap_or(case.frequency_hz),
    )?;
    write_scalar_attribute(group, "requested_mesh_points", &(case.mesh_points as u64))?;
    write_scalar_attribute(group, "max_range_m", &case.max_range_m)?;
    write_scalar_attribute(group, "field_mode_limit", &(case.mode_limit as u64))?;
    write_string_attribute(
        group,
        "source_geometry",
        match case.source_geometry {
            SourceGeometry::Line => "line",
            SourceGeometry::Point => "point",
            SourceGeometry::ScaledCylindrical => "scaled_cylindrical",
        },
    )?;
    write_string_attribute(
        group,
        "mode_addition",
        match case.mode_addition {
            ModeAddition::Coherent => "coherent",
            ModeAddition::Incoherent => "incoherent",
        },
    )?;
    write_string_attribute(
        group,
        "source_pattern",
        if case.source_pattern.is_empty() {
            "omnidirectional"
        } else {
            "tabulated"
        },
    )?;
    write_scalar_attribute(
        group,
        "source_pattern_point_count",
        &(case.source_pattern.len() as u64),
    )?;

    write_scalar_attribute(
        group,
        "finite_fluid_layer_count",
        &((1 + case.additional_fluid_layers.len()) as u64),
    )?;
    let media = group.create_group("media").map_err(hdf5_error)?;
    let mut top_depth = case.fluid_top_depth_m();
    for index in 0..=case.additional_fluid_layers.len() {
        let (bottom_depth, density, mesh_points) = if index == 0 {
            (
                case.water_depth_m,
                case.water_density_g_cm3,
                case.mesh_points,
            )
        } else {
            let layer = &case.additional_fluid_layers[index - 1];
            (layer.bottom_depth_m, layer.density_g_cm3, layer.mesh_points)
        };
        let layer = media.create_group(&index.to_string()).map_err(hdf5_error)?;
        write_scalar_attribute(&layer, "top_depth_m", &top_depth)?;
        write_scalar_attribute(&layer, "bottom_depth_m", &bottom_depth)?;
        write_scalar_attribute(&layer, "density_g_cm3", &density)?;
        write_scalar_attribute(&layer, "requested_mesh_points", &(mesh_points as u64))?;
        write_scalar_attribute(
            &layer,
            "top_roughness_m",
            &case
                .interface_roughness_m
                .get(index)
                .copied()
                .unwrap_or(0.0),
        )?;
        write_scalar_attribute(
            &layer,
            "bottom_roughness_m",
            &case
                .interface_roughness_m
                .get(index + 1)
                .copied()
                .unwrap_or(0.0),
        )?;
        top_depth = bottom_depth;
    }

    Ok(())
}

#[allow(clippy::too_many_lines)] // Keep finite-elastic metadata and its sampled profile together.
fn write_elastic_layers(group: &Group, case: &Case, budget: &mut Budget) -> Result<(), String> {
    write_scalar_attribute(
        group,
        "finite_elastic_layer_count",
        &((case.top_elastic_layers.len() + case.bottom_elastic_layers.len()) as u64),
    )?;
    if case.top_elastic_layers.is_empty() && case.bottom_elastic_layers.is_empty() {
        return Ok(());
    }
    let media = group.create_group("elastic_media").map_err(hdf5_error)?;
    for (name, layers, mut top) in [
        ("top", &case.top_elastic_layers, 0.0),
        (
            "bottom",
            &case.bottom_elastic_layers,
            case.fluid_bottom_depth_m(),
        ),
    ] {
        let side = media.create_group(name).map_err(hdf5_error)?;
        for (index, material) in layers.iter().enumerate() {
            let layer = side.create_group(&index.to_string()).map_err(hdf5_error)?;
            write_string_attribute(&layer, "material", "elastic")?;
            write_string_attribute(
                &layer,
                "attenuation_model",
                if case.mode_solver == ModeSolver::Kraken {
                    "reference_real_stiffness"
                } else {
                    "complex"
                },
            )?;
            for (attribute, value) in [
                ("top_depth_m", top),
                ("bottom_depth_m", material.bottom_depth_m),
                (
                    "compressional_sound_speed_mps",
                    material.compressional_sound_speed_mps,
                ),
                ("shear_sound_speed_mps", material.shear_sound_speed_mps),
                ("density_g_cm3", material.density_g_cm3),
                (
                    "compressional_attenuation_db_per_wavelength",
                    material.compressional_attenuation_db_per_wavelength,
                ),
                (
                    "shear_attenuation_db_per_wavelength",
                    material.shear_attenuation_db_per_wavelength,
                ),
            ] {
                write_scalar_attribute(&layer, attribute, &value)?;
            }
            write_scalar_attribute(
                &layer,
                "requested_mesh_points",
                &(material.mesh_points as u64),
            )?;
            let points = &material.material_profile;
            write_scalar_attribute(
                &layer,
                "material_profile_point_count",
                &(points.len() as u64),
            )?;
            if !points.is_empty() {
                let profile = layer.create_group("material_profile").map_err(hdf5_error)?;
                for (name, unit, values) in [
                    (
                        "depth_m",
                        "m",
                        points.iter().map(|p| p.depth_m).collect::<Vec<_>>(),
                    ),
                    (
                        "compressional_sound_speed_mps",
                        "m/s",
                        points
                            .iter()
                            .map(|p| p.compressional_sound_speed_mps)
                            .collect(),
                    ),
                    (
                        "shear_sound_speed_mps",
                        "m/s",
                        points.iter().map(|p| p.shear_sound_speed_mps).collect(),
                    ),
                    (
                        "density_g_cm3",
                        "g/cm^3",
                        points.iter().map(|p| p.density_g_cm3).collect(),
                    ),
                    (
                        "compressional_attenuation_db_per_wavelength",
                        "dB/wavelength",
                        points
                            .iter()
                            .map(|p| p.compressional_attenuation_db_per_wavelength)
                            .collect(),
                    ),
                    (
                        "shear_attenuation_db_per_wavelength",
                        "dB/wavelength",
                        points
                            .iter()
                            .map(|p| p.shear_attenuation_db_per_wavelength)
                            .collect(),
                    ),
                ] {
                    dataset(&profile, name, &values, &[points.len()], unit, budget)?;
                }
            }
            top = material.bottom_depth_m;
        }
    }
    Ok(())
}

fn write_elastic_half_spaces(group: &Group, case: &Case) -> Result<(), String> {
    for (name, boundary, cp, density, loss) in [
        (
            "surface",
            &case.surface_boundary,
            case.surface_sound_speed_mps,
            case.surface_density_g_cm3,
            case.surface_attenuation_db_per_wavelength,
        ),
        (
            "bottom",
            &case.bottom_boundary,
            case.bottom_sound_speed_mps,
            case.bottom_density_g_cm3,
            case.bottom_attenuation_db_per_wavelength,
        ),
    ] {
        if let kraken::Boundary::ElasticHalfSpace {
            shear_sound_speed_mps,
            shear_attenuation_db_per_wavelength,
        } = boundary
        {
            write_string_attribute(group, &format!("{name}_half_space_material"), "elastic")?;
            write_string_attribute(
                group,
                &format!("{name}_elastic_attenuation_model"),
                if case.mode_solver == ModeSolver::Kraken {
                    "reference_real"
                } else {
                    "complex"
                },
            )?;
            for (attribute, value) in [
                ("sound_speed_mps", cp),
                ("density_g_cm3", density),
                ("attenuation_db_per_wavelength", loss),
                ("shear_sound_speed_mps", *shear_sound_speed_mps),
                (
                    "shear_attenuation_db_per_wavelength",
                    *shear_attenuation_db_per_wavelength,
                ),
            ] {
                write_scalar_attribute(group, &format!("{name}_{attribute}"), &value)?;
            }
        }
    }
    Ok(())
}

fn write_modes(group: &Group, result: &ModeSet, budget: &mut Budget) -> Result<(), String> {
    let modes = group.create_group("modes").map_err(hdf5_error)?;
    let count = result.modes.len();
    let depths = &result.sampled_depths_m;
    write_string_attribute(&modes, "eigenfunction_axis_order", "mode,sample_depth")?;
    write_string_attribute(
        &modes,
        "normalization",
        "pinned AT density-weighted normalization; fluid/elastic boundary-admittance derivative when present; arbitrary unit phase",
    )?;
    dataset(
        &modes,
        "sample_depth_m",
        depths,
        &[depths.len()],
        "m",
        budget,
    )?;
    for (name, unit, values) in [
        (
            "horizontal_wavenumber_real",
            "rad/m",
            result
                .modes
                .iter()
                .map(|m| m.horizontal_wavenumber_rad_per_m.re)
                .collect::<Vec<_>>(),
        ),
        (
            "horizontal_wavenumber_imaginary",
            "rad/m",
            result
                .modes
                .iter()
                .map(|m| m.horizontal_wavenumber_rad_per_m.im)
                .collect(),
        ),
        (
            "phase_speed_mps",
            "m/s",
            result.modes.iter().map(|m| m.phase_speed_mps).collect(),
        ),
        (
            "group_speed_mps",
            "m/s",
            result.modes.iter().map(|m| m.group_speed_mps).collect(),
        ),
        (
            "attenuation_nepers_per_m",
            "neper/m",
            result
                .modes
                .iter()
                .map(|m| m.attenuation_nepers_per_m)
                .collect(),
        ),
    ] {
        dataset(&modes, name, &values, &[count], unit, budget)?;
    }
    for imaginary in [false, true] {
        let values: Vec<_> = result
            .modes
            .iter()
            .flat_map(|m| &m.eigenfunction)
            .map(|value| if imaginary { value.im } else { value.re })
            .collect();
        dataset(
            &modes,
            if imaginary {
                "eigenfunction_imaginary"
            } else {
                "eigenfunction_real"
            },
            &values,
            &[count, depths.len()],
            "reference_normalized",
            budget,
        )?;
    }
    Ok(())
}

#[allow(clippy::cast_possible_truncation)]
fn write_field(group: &Group, result: &PressureField, budget: &mut Budget) -> Result<(), String> {
    let field = group.create_group("field").map_err(hdf5_error)?;
    write_string_attribute(
        &field,
        "pressure_axis_order",
        "source_depth,receiver_depth,receiver_range",
    )?;
    for (name, values) in [
        ("source_depth_m", &result.source_depths_m),
        ("receiver_depth_m", &result.receiver_depths_m),
        ("receiver_range_m", &result.receiver_ranges_m),
        ("receiver_offset_m", &result.receiver_offsets_m),
    ] {
        dataset(&field, name, values, &[values.len()], "m", budget)?;
    }
    let shape = [
        result.source_depths_m.len(),
        result.receiver_depths_m.len(),
        result.receiver_ranges_m.len(),
    ];
    for imaginary in [false, true] {
        let values: Vec<_> = result
            .pressure
            .iter()
            .map(|value| (if imaginary { value.im } else { value.re }) as f32)
            .collect();
        dataset(
            &field,
            if imaginary {
                "pressure_imaginary"
            } else {
                "pressure_real"
            },
            &values,
            &shape,
            "1",
            budget,
        )?;
    }
    Ok(())
}

fn dataset<T: H5Type + Copy + Into<f64>>(
    group: &Group,
    name: &str,
    values: &[T],
    shape: &[usize],
    unit: &str,
    budget: &mut Budget,
) -> Result<(), String> {
    if values.iter().any(|&value| !value.into().is_finite()) {
        return Err(format!("non-finite output in {name}"));
    }
    budget.reserve(
        (values.len() as u64)
            .checked_mul(std::mem::size_of::<T>() as u64)
            .ok_or("dataset byte count overflow")?,
    )?;
    let data = group
        .new_dataset::<T>()
        .shape(shape)
        .create(name)
        .map_err(hdf5_error)?;
    data.write_raw(values).map_err(hdf5_error)?;
    write_string_attribute(&data, "unit", unit)
}

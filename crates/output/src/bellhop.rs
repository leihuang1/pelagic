use std::io::Write;
use std::path::Path;

use bellhop::diagnostic::Diagnostic;
use bellhop::solver::{
    RayTermination, ReceiverArrivals, SimulationResult, SourceArrivals, SourceEigenrays,
};
use hdf5::{File, Group, H5Type};
use sha2::{Digest, Sha256};

use super::hdf5::{
    hdf5_error, write_scalar_attribute, write_string_array_attribute, write_string_attribute,
};
use super::publication;

pub const SCHEMA_VERSION: u32 = 3;

/// Writes one simulation result using schema v3.
///
/// `input_bytes` must be the exact bytes supplied by the caller. They are not
/// embedded, but their byte length and SHA-256 digest are recorded at the root.
///
/// # Errors
///
/// Returns a descriptive error if metadata conversion or HDF5 I/O fails.
pub fn write_hdf5(
    path: &Path,
    input_filename: &str,
    input_bytes: &[u8],
    result: &SimulationResult,
    warnings: &[Diagnostic],
) -> Result<(), String> {
    let file = File::create(path).map_err(hdf5_error)?;
    write_hdf5_contents(&file, input_filename, input_bytes, result, warnings)
}

/// Write BELLHOP schema v3 and publish it with the same local file-safety policy
/// as KRAKEN: protect consumed input paths/aliases, exclusively reserve scratch,
/// close and sync before installing, and never clobber without `overwrite`.
/// No KRAKEN byte quota is imposed on BELLHOP.
/// # Errors
/// Returns metadata, HDF5 or filesystem errors; preserves existing files on failure.
pub fn write_hdf5_atomic(
    path: &Path,
    input_filename: &str,
    input_bytes: &[u8],
    result: &SimulationResult,
    warnings: &[Diagnostic],
    input_paths: &[&Path],
    overwrite: bool,
) -> Result<(), String> {
    publication::publish(path, input_paths, overwrite, None, "BH0402", |file, _| {
        write_hdf5_contents(file, input_filename, input_bytes, result, warnings)
    })
}

/// Publish CLI artifacts in a result directory. Does not affect HTTP single-file publication.
/// # Errors
/// Returns serialization, ownership, input protection or recoverable publication errors.
#[allow(clippy::too_many_arguments)]
pub fn write_directory(
    path: &Path,
    stem: &str,
    case: &bellhop::Case,
    input_filename: &str,
    input_bytes: &[u8],
    result: &SimulationResult,
    warnings: &[Diagnostic],
    input_paths: &[&Path],
    format: super::directory::Format,
) -> Result<(), String> {
    super::directory::publish(path, input_paths, None, |scratch| {
        if format.legacy() {
            super::bellhop_native::write(scratch, stem, case, result)?;
        }
        if format.hdf5() {
            let file = File::create_excl(scratch.join(format!("{stem}.h5"))).map_err(hdf5_error)?;
            write_hdf5_contents(&file, input_filename, input_bytes, result, warnings)?;
            file.close().map_err(hdf5_error)?;
        }
        let report = format!(
            "Pelagic {} BELLHOP run report (not the Fortran PRT format)\nreference: Acoustics Toolbox v2023.5\ntitle: {:?}\nfrequency_hz: {}\nformat: {format:?}\ninput: {input_filename:?}\ninput_sha256: {:x}\nconsumed_inputs: {input_paths:?}\nwarnings: {warnings:?}\n",
            env!("CARGO_PKG_VERSION"),
            result.title,
            result.frequency_hz,
            Sha256::digest(input_bytes)
        );
        let mut file = native_output::create(&scratch.join(format!("{stem}.prt")))
            .map_err(|e| e.to_string())?;
        file.write_all(report.as_bytes())
            .and_then(|()| file.flush())
            .map_err(|e| e.to_string())
    })
}

fn write_hdf5_contents(
    file: &File,
    input_filename: &str,
    input_bytes: &[u8],
    result: &SimulationResult,
    warnings: &[Diagnostic],
) -> Result<(), String> {
    let input_size = u64::try_from(input_bytes.len())
        .map_err(|_| "input size does not fit in u64".to_owned())?;
    let input_sha256 = format!("{:x}", Sha256::digest(input_bytes));

    write_scalar_attribute(file, "schema_version", &SCHEMA_VERSION)?;
    write_string_attribute(
        file,
        "implementation",
        concat!("Pelagic ", env!("CARGO_PKG_VERSION")),
    )?;
    write_string_attribute(
        file,
        "compatibility_reference",
        "Acoustics Toolbox v2023.5 (475108519289c6fb488b58980c644ea14eccc604)",
    )?;
    write_string_attribute(file, "input_filename", input_filename)?;
    write_scalar_attribute(file, "input_size_bytes", &input_size)?;
    write_string_attribute(file, "input_sha256", &input_sha256)?;
    write_string_attribute(file, "title", &result.title)?;
    write_scalar_attribute(file, "frequency_hz", &result.frequency_hz)?;
    write_string_attribute(
        file,
        "legacy_run_options",
        result.legacy_run_options.trim_end(),
    )?;
    write_string_attribute(
        file,
        "coordinate_convention",
        "range origin at source; depth positive downward; launch angle positive downward",
    )?;
    let warnings: Vec<String> = warnings.iter().map(ToString::to_string).collect();
    write_string_array_attribute(file, "warnings", &warnings)?;

    let rays = file.create_group("rays").map_err(hdf5_error)?;
    write_rays(&rays, result)?;
    if !result.arrival_sources.is_empty() {
        let arrivals = file.create_group("arrivals").map_err(hdf5_error)?;
        write_arrivals(&arrivals, &result.arrival_sources)?;
    }
    if !result.eigenray_sources.is_empty() {
        let eigenrays = file.create_group("eigenrays").map_err(hdf5_error)?;
        write_eigenrays(&eigenrays, &result.eigenray_sources)?;
    }
    if !result.field_sources.is_empty() {
        let field = file.create_group("field").map_err(hdf5_error)?;
        write_field(&field, result)?;
    }
    file.flush().map_err(hdf5_error)
}

#[allow(clippy::too_many_lines)]
fn write_rays(group: &Group, result: &SimulationResult) -> Result<(), String> {
    let source_depth_m: Vec<f64> = result
        .sources
        .iter()
        .map(|source| source.source_depth_m)
        .collect();
    let ray_count: usize = result.sources.iter().map(|source| source.rays.len()).sum();
    let point_count: usize = result
        .sources
        .iter()
        .flat_map(|source| &source.rays)
        .map(|ray| ray.points.len())
        .sum();
    let mut ray_source_index = Vec::with_capacity(ray_count);
    let mut launch_angle_degrees = Vec::with_capacity(ray_count);
    let mut point_offset = Vec::with_capacity(ray_count + 1);
    let mut top_bounces = Vec::with_capacity(ray_count);
    let mut bottom_bounces = Vec::with_capacity(ray_count);
    let mut termination = Vec::with_capacity(ray_count);
    let mut range_m = Vec::with_capacity(point_count);
    let mut depth_m = Vec::with_capacity(point_count);
    let mut travel_time_s = Vec::with_capacity(point_count);
    let mut attenuation_time_s = Vec::with_capacity(point_count);
    let mut amplitude = Vec::with_capacity(point_count);
    let mut phase_radians = Vec::with_capacity(point_count);
    point_offset.push(0_u64);

    for (source_index, source) in result.sources.iter().enumerate() {
        let source_index = u32::try_from(source_index)
            .map_err(|_| "source index does not fit in u32".to_owned())?;
        for ray in &source.rays {
            ray_source_index.push(source_index);
            launch_angle_degrees.push(ray.launch_angle_degrees);
            top_bounces.push(ray.top_bounces);
            bottom_bounces.push(ray.bottom_bounces);
            termination.push(termination_code(ray.termination));
            for point in &ray.points {
                range_m.push(point.range_m);
                depth_m.push(point.depth_m);
                travel_time_s.push(point.travel_time_s);
                attenuation_time_s.push(point.attenuation_time_s);
                amplitude.push(point.amplitude);
                phase_radians.push(point.phase_radians);
            }
            point_offset.push(
                u64::try_from(range_m.len())
                    .map_err(|_| "ray point offset does not fit in u64".to_owned())?,
            );
        }
    }

    write_dataset(group, "source_depth_m", &source_depth_m, "m")?;
    write_dataset(group, "ray_source_index", &ray_source_index, "index")?;
    write_dataset(
        group,
        "launch_angle_degrees",
        &launch_angle_degrees,
        "degree",
    )?;
    write_dataset(group, "point_offset", &point_offset, "index")?;
    write_dataset(group, "top_bounces", &top_bounces, "count")?;
    write_dataset(group, "bottom_bounces", &bottom_bounces, "count")?;
    write_dataset(group, "termination", &termination, "enum")?;
    write_dataset(group, "range_m", &range_m, "m")?;
    write_dataset(group, "depth_m", &depth_m, "m")?;
    write_dataset(group, "travel_time_s", &travel_time_s, "s")?;
    write_dataset(group, "attenuation_time_s", &attenuation_time_s, "s")?;
    write_dataset(group, "amplitude", &amplitude, "1")?;
    write_dataset(group, "phase_radians", &phase_radians, "rad")?;
    write_string_attribute(
        group,
        "termination_codes",
        "0=exited_trace_box,1=lost_energy,2=escaped_boundary,3=source_outside_boundaries,4=step_limit,5=receiver_hit",
    )?;
    Ok(())
}

fn write_arrivals(group: &Group, sources: &[SourceArrivals]) -> Result<(), String> {
    let source_depth_m: Vec<f64> = sources.iter().map(|source| source.source_depth_m).collect();
    let receiver_count: usize = sources.iter().map(|source| source.receivers.len()).sum();
    let arrival_count: usize = sources
        .iter()
        .flat_map(|source| &source.receivers)
        .map(|receiver| receiver.arrivals.len())
        .sum();
    let mut receiver_offset = Vec::with_capacity(sources.len() + 1);
    let mut receiver_range_m = Vec::with_capacity(receiver_count);
    let mut receiver_depth_m = Vec::with_capacity(receiver_count);
    let mut arrival_offset = Vec::with_capacity(receiver_count + 1);
    let mut amplitude = Vec::with_capacity(arrival_count);
    let mut phase_radians = Vec::with_capacity(arrival_count);
    let mut travel_time_s = Vec::with_capacity(arrival_count);
    let mut attenuation_time_s = Vec::with_capacity(arrival_count);
    let mut source_angle_degrees = Vec::with_capacity(arrival_count);
    let mut receiver_angle_degrees = Vec::with_capacity(arrival_count);
    let mut top_bounces = Vec::with_capacity(arrival_count);
    let mut bottom_bounces = Vec::with_capacity(arrival_count);
    receiver_offset.push(0_u64);
    arrival_offset.push(0_u64);

    for source in sources {
        for receiver in &source.receivers {
            receiver_range_m.push(receiver.range_m);
            receiver_depth_m.push(receiver.depth_m);
            append_arrivals(
                receiver,
                &mut amplitude,
                &mut phase_radians,
                &mut travel_time_s,
                &mut attenuation_time_s,
                &mut source_angle_degrees,
                &mut receiver_angle_degrees,
                &mut top_bounces,
                &mut bottom_bounces,
            );
            arrival_offset.push(as_u64(amplitude.len(), "arrival offset")?);
        }
        receiver_offset.push(as_u64(receiver_range_m.len(), "receiver offset")?);
    }

    write_dataset(group, "source_depth_m", &source_depth_m, "m")?;
    write_dataset(group, "receiver_offset", &receiver_offset, "index")?;
    write_dataset(group, "receiver_range_m", &receiver_range_m, "m")?;
    write_dataset(group, "receiver_depth_m", &receiver_depth_m, "m")?;
    write_dataset(group, "arrival_offset", &arrival_offset, "index")?;
    write_dataset(group, "amplitude", &amplitude, "1")?;
    write_dataset(group, "phase_radians", &phase_radians, "rad")?;
    write_dataset(group, "travel_time_s", &travel_time_s, "s")?;
    write_dataset(group, "attenuation_time_s", &attenuation_time_s, "s")?;
    write_dataset(
        group,
        "source_angle_degrees",
        &source_angle_degrees,
        "degree",
    )?;
    write_dataset(
        group,
        "receiver_angle_degrees",
        &receiver_angle_degrees,
        "degree",
    )?;
    write_dataset(group, "top_bounces", &top_bounces, "count")?;
    write_dataset(group, "bottom_bounces", &bottom_bounces, "count")
}

#[allow(clippy::too_many_arguments)]
fn append_arrivals(
    receiver: &ReceiverArrivals,
    amplitude: &mut Vec<f32>,
    phase_radians: &mut Vec<f32>,
    travel_time_s: &mut Vec<f32>,
    attenuation_time_s: &mut Vec<f32>,
    source_angle_degrees: &mut Vec<f32>,
    receiver_angle_degrees: &mut Vec<f32>,
    top_bounces: &mut Vec<u32>,
    bottom_bounces: &mut Vec<u32>,
) {
    for arrival in &receiver.arrivals {
        amplitude.push(arrival.amplitude);
        phase_radians.push(arrival.phase_radians);
        travel_time_s.push(arrival.travel_time_s);
        attenuation_time_s.push(arrival.attenuation_time_s);
        source_angle_degrees.push(arrival.source_angle_degrees);
        receiver_angle_degrees.push(arrival.receiver_angle_degrees);
        top_bounces.push(arrival.top_bounces);
        bottom_bounces.push(arrival.bottom_bounces);
    }
}

#[allow(clippy::too_many_lines)]
fn write_eigenrays(group: &Group, sources: &[SourceEigenrays]) -> Result<(), String> {
    let source_depth_m: Vec<f64> = sources.iter().map(|source| source.source_depth_m).collect();
    let receiver_count: usize = sources.iter().map(|source| source.receivers.len()).sum();
    let eigenray_count: usize = sources
        .iter()
        .flat_map(|source| &source.receivers)
        .map(|receiver| receiver.eigenrays.len())
        .sum();
    let point_count: usize = sources
        .iter()
        .flat_map(|source| &source.receivers)
        .flat_map(|receiver| &receiver.eigenrays)
        .map(|ray| ray.points.len())
        .sum();
    let mut receiver_offset = vec![0_u64];
    let mut receiver_range_m = Vec::with_capacity(receiver_count);
    let mut receiver_depth_m = Vec::with_capacity(receiver_count);
    let mut eigenray_offset = vec![0_u64];
    let mut eigenray_receiver_index = Vec::with_capacity(eigenray_count);
    let mut launch_angle_degrees = Vec::with_capacity(eigenray_count);
    let mut point_offset = vec![0_u64];
    let mut top_bounces = Vec::with_capacity(eigenray_count);
    let mut bottom_bounces = Vec::with_capacity(eigenray_count);
    let mut range_m = Vec::with_capacity(point_count);
    let mut depth_m = Vec::with_capacity(point_count);
    let mut travel_time_s = Vec::with_capacity(point_count);
    let mut attenuation_time_s = Vec::with_capacity(point_count);
    let mut amplitude = Vec::with_capacity(point_count);
    let mut phase_radians = Vec::with_capacity(point_count);

    for source in sources {
        for receiver in &source.receivers {
            let receiver_index = u64::try_from(receiver_range_m.len())
                .map_err(|_| "receiver index does not fit in u64".to_owned())?;
            receiver_range_m.push(receiver.range_m);
            receiver_depth_m.push(receiver.depth_m);
            for ray in &receiver.eigenrays {
                eigenray_receiver_index.push(receiver_index);
                launch_angle_degrees.push(ray.launch_angle_degrees);
                top_bounces.push(ray.top_bounces);
                bottom_bounces.push(ray.bottom_bounces);
                for point in &ray.points {
                    range_m.push(point.range_m);
                    depth_m.push(point.depth_m);
                    travel_time_s.push(point.travel_time_s);
                    attenuation_time_s.push(point.attenuation_time_s);
                    amplitude.push(point.amplitude);
                    phase_radians.push(point.phase_radians);
                }
                point_offset.push(as_u64(range_m.len(), "eigenray point offset")?);
            }
            eigenray_offset.push(as_u64(eigenray_receiver_index.len(), "eigenray offset")?);
        }
        receiver_offset.push(as_u64(receiver_range_m.len(), "receiver offset")?);
    }

    write_dataset(group, "source_depth_m", &source_depth_m, "m")?;
    write_dataset(group, "receiver_offset", &receiver_offset, "index")?;
    write_dataset(group, "receiver_range_m", &receiver_range_m, "m")?;
    write_dataset(group, "receiver_depth_m", &receiver_depth_m, "m")?;
    write_dataset(group, "eigenray_offset", &eigenray_offset, "index")?;
    write_dataset(
        group,
        "eigenray_receiver_index",
        &eigenray_receiver_index,
        "index",
    )?;
    write_dataset(
        group,
        "launch_angle_degrees",
        &launch_angle_degrees,
        "degree",
    )?;
    write_dataset(group, "point_offset", &point_offset, "index")?;
    write_dataset(group, "top_bounces", &top_bounces, "count")?;
    write_dataset(group, "bottom_bounces", &bottom_bounces, "count")?;
    write_dataset(group, "range_m", &range_m, "m")?;
    write_dataset(group, "depth_m", &depth_m, "m")?;
    write_dataset(group, "travel_time_s", &travel_time_s, "s")?;
    write_dataset(group, "attenuation_time_s", &attenuation_time_s, "s")?;
    write_dataset(group, "amplitude", &amplitude, "1")?;
    write_dataset(group, "phase_radians", &phase_radians, "rad")
}

fn write_field(group: &Group, result: &SimulationResult) -> Result<(), String> {
    let source_depth_m: Vec<f64> = result
        .field_sources
        .iter()
        .map(|source| source.source_depth_m)
        .collect();
    let sample_count: usize = result
        .field_sources
        .iter()
        .map(|source| source.samples.len())
        .sum();
    let mut receiver_offset = vec![0_u64];
    let mut receiver_range_m = Vec::with_capacity(sample_count);
    let mut receiver_depth_m = Vec::with_capacity(sample_count);
    let mut pressure_real = Vec::with_capacity(sample_count);
    let mut pressure_imaginary = Vec::with_capacity(sample_count);
    for source in &result.field_sources {
        for sample in &source.samples {
            receiver_range_m.push(sample.range_m);
            receiver_depth_m.push(sample.depth_m);
            pressure_real.push(sample.pressure.re);
            pressure_imaginary.push(sample.pressure.im);
        }
        receiver_offset.push(as_u64(receiver_range_m.len(), "field receiver offset")?);
    }

    write_dataset(group, "source_depth_m", &source_depth_m, "m")?;
    write_dataset(group, "receiver_offset", &receiver_offset, "index")?;
    write_dataset(group, "receiver_range_m", &receiver_range_m, "m")?;
    write_dataset(group, "receiver_depth_m", &receiver_depth_m, "m")?;
    write_dataset(group, "pressure_real", &pressure_real, "1")?;
    write_dataset(group, "pressure_imaginary", &pressure_imaginary, "1")
}

fn as_u64(value: usize, description: &str) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| format!("{description} does not fit in u64"))
}

fn termination_code(termination: RayTermination) -> u8 {
    match termination {
        RayTermination::ExitedTraceBox => 0,
        RayTermination::LostEnergy => 1,
        RayTermination::EscapedBoundary => 2,
        RayTermination::SourceOutsideBoundaries => 3,
        RayTermination::StepLimit => 4,
        RayTermination::ReceiverHit => 5,
    }
}

fn write_dataset<T: H5Type>(
    group: &Group,
    name: &str,
    values: &[T],
    unit: &str,
) -> Result<(), String> {
    let dataset = group
        .new_dataset_builder()
        .with_data(values)
        .create(name)
        .map_err(hdf5_error)?;
    write_string_attribute(&dataset, "unit", unit)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use bellhop::diagnostic::{Diagnostic, SourceLocation};
    use bellhop::solver::{
        RayPoint, RayTermination, RayTrajectory, SimulationLimits, SimulationResult, SourceRaySet,
        run,
    };
    use hdf5::File;
    use hdf5::types::VarLenUnicode;

    use super::write_hdf5;

    #[test]
    fn writes_versioned_flattened_ray_schema() {
        let directory =
            std::env::temp_dir().join(format!("bellhop-hdf5-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let output = directory.join("case.h5");
        let result = SimulationResult {
            title: "test case".to_owned(),
            frequency_hz: 100.0,
            legacy_run_options: "R".to_owned(),
            sources: vec![SourceRaySet {
                source_depth_m: 10.0,
                rays: vec![RayTrajectory {
                    launch_angle_degrees: 5.0,
                    points: vec![
                        RayPoint {
                            range_m: 0.0,
                            depth_m: 10.0,
                            travel_time_s: 0.0,
                            attenuation_time_s: 0.0,
                            amplitude: 1.0,
                            phase_radians: 0.0,
                        },
                        RayPoint {
                            range_m: 100.0,
                            depth_m: 20.0,
                            travel_time_s: 0.1,
                            attenuation_time_s: -0.01,
                            amplitude: 0.8,
                            phase_radians: 1.0,
                        },
                    ],
                    top_bounces: 0,
                    bottom_bounces: 1,
                    termination: RayTermination::ExitedTraceBox,
                }],
            }],
            arrival_sources: Vec::new(),
            eigenray_sources: Vec::new(),
            field_sources: Vec::new(),
        };

        let warning = Diagnostic::warning(
            "BH1000",
            "test warning",
            "test",
            SourceLocation::file("case.env"),
        );
        write_hdf5(&output, "case.env", b"test input", &result, &[warning]).unwrap();
        let file = File::open(&output).unwrap();
        assert_eq!(
            file.attr("schema_version")
                .unwrap()
                .read_scalar::<u32>()
                .unwrap(),
            3
        );
        let warnings = file
            .attr("warnings")
            .unwrap()
            .read_raw::<VarLenUnicode>()
            .unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].as_str().contains("warning[BH1000]"));
        assert_eq!(
            file.dataset("rays/point_offset")
                .unwrap()
                .read_raw::<u64>()
                .unwrap(),
            vec![0, 2]
        );
        assert_eq!(
            file.dataset("rays/source_depth_m")
                .unwrap()
                .read_raw::<f64>()
                .unwrap(),
            vec![10.0]
        );
        assert_eq!(
            file.dataset("rays/depth_m")
                .unwrap()
                .read_raw::<f64>()
                .unwrap(),
            vec![10.0, 20.0]
        );
        drop(file);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn writes_flattened_non_ray_schemas() {
        let directory =
            std::env::temp_dir().join(format!("bellhop-hdf5-arrival-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../bellhop/tests/fixtures/golden/GeoHat_arrival.env");
        let output = directory.join("arrival.h5");
        let case = bellhop::legacy::load_case(&input).unwrap().value;
        let result = run(&case, SimulationLimits::default()).unwrap();

        write_hdf5(
            &output,
            "GeoHat_arrival.env",
            &fs::read(&input).unwrap(),
            &result,
            &[],
        )
        .unwrap();
        let file = File::open(&output).unwrap();
        assert_eq!(
            file.dataset("arrivals/arrival_offset")
                .unwrap()
                .read_raw::<u64>()
                .unwrap(),
            vec![0, 3]
        );
        assert_eq!(file.dataset("arrivals/amplitude").unwrap().shape(), vec![3]);
        file.dataset("arrivals/receiver_depth_m")
            .unwrap()
            .read_raw::<f64>()
            .unwrap();
        drop(file);

        let eigen_input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../bellhop/tests/fixtures/golden/GeoHat_eigen.env");
        let eigen_output = directory.join("eigen.h5");
        let case = bellhop::legacy::load_case(&eigen_input).unwrap().value;
        let result = run(&case, SimulationLimits::default()).unwrap();
        write_hdf5(
            &eigen_output,
            "GeoHat_eigen.env",
            &fs::read(&eigen_input).unwrap(),
            &result,
            &[],
        )
        .unwrap();
        let file = File::open(&eigen_output).unwrap();
        assert_eq!(
            file.dataset("eigenrays/point_offset")
                .unwrap()
                .read_raw::<u64>()
                .unwrap(),
            vec![0, 106, 208, 314]
        );
        drop(file);

        let field_input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../bellhop/tests/fixtures/golden/Field_G.env");
        let field_output = directory.join("field.h5");
        let case = bellhop::legacy::load_case(&field_input).unwrap().value;
        let result = run(&case, SimulationLimits::default()).unwrap();
        write_hdf5(
            &field_output,
            "Field_G.env",
            &fs::read(&field_input).unwrap(),
            &result,
            &[],
        )
        .unwrap();
        let file = File::open(&field_output).unwrap();
        assert_eq!(
            file.dataset("field/receiver_offset")
                .unwrap()
                .read_raw::<u64>()
                .unwrap(),
            vec![0, 33]
        );
        assert_eq!(
            file.dataset("field/pressure_real").unwrap().shape(),
            vec![33]
        );
        drop(file);
        fs::remove_dir_all(directory).unwrap();
    }
}

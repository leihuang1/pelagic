//! Result conversion only: no validation, solver arithmetic or interpolation in this adapter.
use numpy::{Complex32, Complex64};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

/// A value written as explicit little-endian bytes; `NumPy` reads it back with `DTYPE`.
trait Raw {
    const DTYPE: &'static str;
    const SIZE: usize;
    fn put(self, out: &mut [u8]);
}
macro_rules! raw {
    ($type:ty, $dtype:literal) => {
        impl Raw for $type {
            const DTYPE: &'static str = $dtype;
            const SIZE: usize = size_of::<$type>();
            fn put(self, out: &mut [u8]) {
                out.copy_from_slice(&self.to_le_bytes());
            }
        }
    };
}
raw!(f64, "<f8");
raw!(f32, "<f4");
raw!(u32, "<u4");
raw!(u64, "<u8");
impl Raw for Complex32 {
    const DTYPE: &'static str = "<c8";
    const SIZE: usize = 8;
    fn put(self, out: &mut [u8]) {
        self.re.put(&mut out[..4]);
        self.im.put(&mut out[4..]);
    }
}
impl Raw for Complex64 {
    const DTYPE: &'static str = "<c16";
    const SIZE: usize = 16;
    fn put(self, out: &mut [u8]) {
        self.re.put(&mut out[..8]);
        self.im.put(&mut out[8..]);
    }
}

/// Write values once, directly into immutable Python bytes, then view them as an array.
/// The bytes object backs every public array, so `setflags(write=True)` must also fail.
fn array<T: Raw>(
    py: Python<'_>,
    values: impl IntoIterator<Item = T>,
    shape: Vec<usize>,
) -> PyResult<Bound<'_, PyAny>> {
    let length = shape
        .iter()
        .try_fold(T::SIZE, |total, &axis| total.checked_mul(axis))
        .ok_or_else(|| PyValueError::new_err("array shape overflow"))?;
    let mut values = values.into_iter();
    let bytes = PyBytes::new_with(py, length, |buffer| {
        for slot in buffer.chunks_exact_mut(T::SIZE) {
            values
                .next()
                .ok_or_else(|| PyValueError::new_err("array has fewer values than its shape"))?
                .put(slot);
        }
        Ok(())
    })?;
    if values.next().is_some() {
        return Err(PyValueError::new_err(
            "array has more values than its shape",
        ));
    }
    py.import("numpy")?
        .getattr("frombuffer")?
        .call1((bytes, T::DTYPE))?
        .call_method1("reshape", (shape,))
}
fn vector<T: Raw>(py: Python<'_>, values: Vec<T>) -> PyResult<Bound<'_, PyAny>> {
    let len = values.len();
    array(py, values, vec![len])
}

pub(super) fn bellhop<'py>(
    py: Python<'py>,
    case: &bellhop::Case,
    result: &bellhop::result::SimulationResult,
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("model", "bellhop")?;
    output.set_item("frequency_hz", result.frequency_hz)?;
    output.set_item("title", &result.title)?;
    if result.field_sources.is_empty() {
        output.set_item("field", py.None())?;
    } else {
        let grid = result
            .field_grid(case)
            .map_err(|e| super::adapter_error("simulation", "BH0302", "field", e))?;
        let field = PyDict::new(py);
        field.set_item(
            "geometry",
            if case.environment.run.receiver_grid == bellhop::model::ReceiverGrid::Irregular {
                "irregular"
            } else {
                "rectilinear"
            },
        )?;
        field.set_item("pressure", array(py, grid.pressures(), grid.shape())?)?;
        field.set_item(
            "ranges_m",
            vector(py, case.environment.positions.receiver_ranges_m.clone())?,
        )?;
        field.set_item(
            "depths_m",
            vector(py, case.environment.positions.receiver_depths_m.clone())?,
        )?;
        field.set_item(
            "source_depths_m",
            vector(py, case.environment.positions.source_depths_m.clone())?,
        )?;
        output.set_item("field", field)?;
    }
    let rays = PyList::empty(py);
    for (source_index, source) in result.sources.iter().enumerate() {
        for ray in &source.rays {
            rays.append(trajectory(py, source_index, source.source_depth_m, ray)?)?;
        }
    }
    output.set_item("rays", rays)?;
    let eigenrays = PyList::empty(py);
    for (source_index, source) in result.eigenray_sources.iter().enumerate() {
        for (receiver_index, receiver) in source.receivers.iter().enumerate() {
            for ray in &receiver.eigenrays {
                let item = trajectory(py, source_index, source.source_depth_m, ray)?;
                item.set_item("receiver_index", receiver_index)?;
                item.set_item("range_m", receiver.range_m)?;
                item.set_item("depth_m", receiver.depth_m)?;
                eigenrays.append(item)?;
            }
        }
    }
    output.set_item("eigenrays", eigenrays)?;
    output.set_item("arrivals", arrivals(py, &result.arrival_sources)?)?;
    Ok(output)
}

fn trajectory<'py>(
    py: Python<'py>,
    source: usize,
    depth: f64,
    ray: &bellhop::result::RayTrajectory,
) -> PyResult<Bound<'py, PyDict>> {
    let item = PyDict::new(py);
    let points = ray.points.iter().flat_map(|p| {
        [
            p.range_m,
            p.depth_m,
            p.travel_time_s,
            p.attenuation_time_s,
            p.amplitude,
            p.phase_radians,
        ]
    });
    item.set_item("points", array(py, points, vec![ray.points.len(), 6])?)?;
    item.set_item("source_index", source)?;
    item.set_item("source_depth_m", depth)?;
    item.set_item("launch_angle_degrees", ray.launch_angle_degrees)?;
    item.set_item("top_bounces", ray.top_bounces)?;
    item.set_item("bottom_bounces", ray.bottom_bounces)?;
    item.set_item("termination", format!("{:?}", ray.termination))?;
    Ok(item)
}

fn arrivals<'py>(
    py: Python<'py>,
    sources: &[bellhop::result::SourceArrivals],
) -> PyResult<Bound<'py, PyDict>> {
    let item = PyDict::new(py);
    let mut receiver_offset = vec![0_u64];
    let mut arrival_offset = vec![0_u64];
    let mut ranges = Vec::new();
    let mut depths = Vec::new();
    let mut values = Vec::new();
    for source in sources {
        for receiver in &source.receivers {
            ranges.push(receiver.range_m);
            depths.push(receiver.depth_m);
            values.extend_from_slice(&receiver.arrivals);
            arrival_offset.push(values.len() as u64);
        }
        receiver_offset.push(ranges.len() as u64);
    }
    item.set_item(
        "source_depths_m",
        vector(py, sources.iter().map(|s| s.source_depth_m).collect())?,
    )?;
    item.set_item("receiver_offset", vector(py, receiver_offset)?)?;
    item.set_item("arrival_offset", vector(py, arrival_offset)?)?;
    item.set_item("ranges_m", vector(py, ranges)?)?;
    item.set_item("depths_m", vector(py, depths)?)?;
    macro_rules! column {
        ($name:ident) => {
            item.set_item(
                stringify!($name),
                vector(py, values.iter().map(|a| a.$name).collect())?,
            )?;
        };
    }
    column!(amplitude);
    column!(phase_radians);
    column!(travel_time_s);
    column!(attenuation_time_s);
    column!(source_angle_degrees);
    column!(receiver_angle_degrees);
    column!(top_bounces);
    column!(bottom_bounces);
    Ok(item)
}

#[allow(clippy::cast_possible_truncation)] // FIELD values already contain the pinned complex32 rounding.
pub(super) fn kraken<'py>(
    py: Python<'py>,
    cases: &[kraken::FieldCase],
    results: &[kraken::ProfileSimulationResult],
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("model", "kraken")?;
    let frequencies = PyList::empty(py);
    for (case, result) in cases.iter().zip(results) {
        let frequency = PyDict::new(py);
        frequency.set_item("frequency_hz", case.profiles()[0].frequency_hz)?;
        frequency.set_item("title", &case.profiles()[0].title)?;
        frequency.set_item(
            "solver",
            match case.profiles()[0].mode_solver {
                kraken::ModeSolver::Kraken => "kraken",
                kraken::ModeSolver::Krakenc => "krakenc",
            },
        )?;
        let profiles = PyList::empty(py);
        for (index, modes) in result.modes.iter().enumerate() {
            let profile = PyDict::new(py);
            profile.set_item("range_m", case.ranges_m()[index])?;
            profile.set_item("title", &case.profiles()[index].title)?;
            let set = PyDict::new(py);
            set.set_item("depths_m", vector(py, modes.sampled_depths_m.clone())?)?;
            set.set_item(
                "wavenumbers",
                vector(
                    py,
                    modes
                        .modes
                        .iter()
                        .map(|m| m.horizontal_wavenumber_rad_per_m)
                        .collect(),
                )?,
            )?;
            let shapes = (0..modes.sampled_depths_m.len())
                .flat_map(|depth| modes.modes.iter().map(move |m| m.eigenfunction[depth]));
            set.set_item(
                "eigenfunctions",
                array(
                    py,
                    shapes,
                    vec![modes.sampled_depths_m.len(), modes.modes.len()],
                )?,
            )?;
            macro_rules! column {
                ($name:ident) => {
                    set.set_item(
                        stringify!($name),
                        vector(py, modes.modes.iter().map(|m| m.$name).collect())?,
                    )?;
                };
            }
            column!(phase_speed_mps);
            column!(group_speed_mps);
            column!(attenuation_nepers_per_m);
            profile.set_item("modes", set)?;
            profiles.append(profile)?;
        }
        frequency.set_item("profiles", profiles)?;
        let field = PyDict::new(py);
        field.set_item("geometry", "rectilinear")?;
        let f = &result.field;
        field.set_item(
            "pressure",
            array(
                py,
                f.pressure
                    .iter()
                    .map(|p| Complex32::new(p.re as f32, p.im as f32)),
                vec![
                    f.source_depths_m.len(),
                    f.receiver_depths_m.len(),
                    f.receiver_ranges_m.len(),
                ],
            )?,
        )?;
        field.set_item("source_depths_m", vector(py, f.source_depths_m.clone())?)?;
        field.set_item("depths_m", vector(py, f.receiver_depths_m.clone())?)?;
        field.set_item("ranges_m", vector(py, f.receiver_ranges_m.clone())?)?;
        field.set_item(
            "receiver_offsets_m",
            vector(py, f.receiver_offsets_m.clone())?,
        )?;
        frequency.set_item("field", field)?;
        frequencies.append(frequency)?;
    }
    output.set_item("frequencies", frequencies)?;
    Ok(output)
}

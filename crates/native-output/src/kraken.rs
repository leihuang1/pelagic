//! A native MOD/SHD pair per ordered frequency block, each MOD retaining profile order.
//! Separate blocks can have different geometry in the accepted JSON contract.
#![allow(clippy::cast_possible_truncation)]
use crate::native::{self, Direct, count, doubles, singles, text};
use kraken::{Boundary, Case, FieldCase, ModeSet, ModeSolver, ProfileSimulationResult};
use std::io;
use std::path::Path;

/// Serialize an existing frequency result into owned scratch; never runs the solver.
/// # Errors
/// Returns native representation, quota or filesystem errors.
pub fn write(
    root: &Path,
    stem: &str,
    sequence: &FieldCase,
    result: &ProfileSimulationResult,
    maximum: u64,
) -> Result<(), String> {
    let remaining = maximum - crate::directory::check_quota(root, Some(maximum))?;
    modes(
        &root.join(format!("{stem}.mod")),
        sequence,
        result,
        remaining,
    )
    .map_err(|e| e.to_string())?;
    let remaining = maximum - crate::directory::check_quota(root, Some(maximum))?;
    let field = &result.field;
    native::shd(
        &root.join(format!("{stem}.shd")),
        &sequence.profiles()[0].title,
        sequence.profiles()[0].frequency_hz,
        &field.source_depths_m,
        &field.receiver_depths_m,
        &field.receiver_ranges_m,
        false,
        field.pressure.iter().map(|p| (p.re as f32, p.im as f32)),
        remaining,
    )
    .map_err(|e| e.to_string())
}

fn modes(
    path: &Path,
    sequence: &FieldCase,
    result: &ProfileSimulationResult,
    maximum: u64,
) -> io::Result<()> {
    let words = result
        .modes
        .iter()
        .map(|m| 2 * m.sampled_depths_m.len())
        .chain(
            sequence
                .profiles()
                .iter()
                .map(|c| 3 * (1 + c.additional_fluid_layers.len())),
        )
        .max()
        .unwrap_or(0)
        .max(32);
    let words = words.div_ceil(2) * 2;
    let mut file = Direct::new(path, words, maximum)?;
    for (case, modes) in sequence.profiles().iter().zip(&result.modes) {
        profile(&mut file, words, case, modes)?;
    }
    file.finish()
}

fn profile(file: &mut Direct, words: usize, case: &Case, modes: &ModeSet) -> io::Result<()> {
    let mut header = count(words)?.to_vec();
    let solver = match case.mode_solver {
        ModeSolver::Kraken => "KRAKEN",
        ModeSolver::Krakenc => "KRAKENC",
    };
    header.extend(text(&format!("{solver} {}", case.title), 80));
    for n in [
        1,
        1 + case.additional_fluid_layers.len(),
        modes.sampled_depths_m.len(),
        modes.sampled_depths_m.len(),
    ] {
        header.extend(count(n)?);
    }
    file.record(header)?;
    let mesh =
        kraken::solver::base_mesh_intervals(case).map_err(|e| io::Error::other(e.to_string()))?;
    let mut media = Vec::new();
    for n in mesh {
        media.extend(count(n)?);
        media.extend(b"ACOUSTIC");
    }
    file.record(media)?;
    let mut layers = Vec::new();
    for (top, density) in std::iter::once((case.fluid_top_depth_m(), case.water_density_g_cm3))
        .chain(
            case.additional_fluid_layers
                .iter()
                .enumerate()
                .map(|(i, layer)| {
                    (
                        if i == 0 {
                            case.water_depth_m
                        } else {
                            case.additional_fluid_layers[i - 1].bottom_depth_m
                        },
                        layer.density_g_cm3,
                    )
                }),
        )
    {
        layers.extend(native::single(top)?);
        layers.extend(native::single(density)?);
    }
    file.record(layers)?;
    file.record(doubles(&[modes.frequency_hz]))?;
    file.record(singles(&modes.sampled_depths_m)?)?;
    file.record(count(modes.modes.len())?.to_vec())?;
    let mut boundaries = half_space(
        &case.surface_boundary,
        case.surface_sound_speed_mps,
        case.surface_density_g_cm3,
        case.surface_attenuation_db_per_wavelength,
        0.0,
    )?;
    boundaries.extend(half_space(
        &case.bottom_boundary,
        case.bottom_sound_speed_mps,
        case.bottom_density_g_cm3,
        case.bottom_attenuation_db_per_wavelength,
        case.total_depth_m(),
    )?);
    file.record(boundaries)?;
    for mode in &modes.modes {
        let mut shape = Vec::with_capacity(8 * mode.eigenfunction.len());
        for phi in &mode.eigenfunction {
            shape.extend(native::single(phi.re)?);
            shape.extend(native::single(phi.im)?);
        }
        file.record(shape)?;
    }
    // Wavenumbers fold at whole complex values, even when the word count is odd.
    for chunk in modes.modes.chunks(words / 2) {
        let mut values = Vec::new();
        for mode in chunk {
            values.extend(native::single(mode.horizontal_wavenumber_rad_per_m.re)?);
            values.extend(native::single(mode.horizontal_wavenumber_rad_per_m.im)?);
        }
        file.record(values)?;
    }
    // Fortran's integer division truncates toward zero: even M=0 has a wavenumber record.
    if modes.modes.is_empty() {
        file.record(Vec::new())?;
    }
    Ok(())
}

fn half_space(
    boundary: &Boundary,
    cp: f64,
    density: f64,
    loss: f64,
    depth: f64,
) -> io::Result<Vec<u8>> {
    let code = match boundary {
        Boundary::Vacuum => b'V',
        Boundary::Rigid => b'R',
        Boundary::FluidHalfSpace | Boundary::ElasticHalfSpace { .. } => b'A',
        Boundary::Reflection(_) => b'F',
        Boundary::Impedance { .. } => b'P',
    };
    let (cs, shear_loss) = if let Boundary::ElasticHalfSpace {
        shear_sound_speed_mps,
        shear_attenuation_db_per_wavelength,
    } = boundary
    {
        (*shear_sound_speed_mps, *shear_attenuation_db_per_wavelength)
    } else {
        (0.0, 0.0)
    };
    let mut bytes = vec![code];
    // Pinned AttenMod::CRCI uses positive imaginary sound speed (not k's attenuation sign).
    for (speed, loss) in [(cp, loss), (cs, shear_loss)] {
        bytes.extend(native::single(speed)?);
        bytes.extend(native::single(
            loss * speed / (8.685_889_6 * 2.0 * std::f64::consts::PI),
        )?);
    }
    bytes.extend(native::single(density)?);
    bytes.extend(native::single(depth)?);
    Ok(bytes)
}

// Adapted from Acoustics Toolbox v2023.5 Kraken/Scattering.f90 and
// kraken{c}.f90::ScatterLoss, Copyright (C) 2009 Michael B. Porter.
// GPL-3.0-or-later; see LICENSE.
//! First-mesh Kuperman–Ingenito perturbation to squared horizontal wavenumber.
use super::{complex_modes::pekeris_root, elastic::complex_speed, layers::MeshLayer};
use crate::{Boundary, Case, ModeSolver};
use num_complex::Complex64;

pub(super) fn perturbation(
    case: &Case,
    layers: &[MeshLayer],
    x: Complex64,
    omega: f64,
    b: impl Fn(usize) -> Complex64,
    phi: impl Fn(usize) -> Complex64,
) -> Complex64 {
    let mut result = Complex64::new(0.0, 0.0);
    for (interface, &sigma) in case.interface_roughness_m.iter().enumerate() {
        if sigma == 0.0 {
            continue;
        }
        let (j, rho1, eta1, velocity) = if interface == 0 {
            let (rho, eta) = exterior(case, true, omega, x);
            let velocity = match case.surface_boundary {
                Boundary::FluidHalfSpace => {
                    let root = if case.mode_solver == ModeSolver::Kraken {
                        eta.sqrt()
                    } else {
                        pekeris_root(eta)
                    };
                    root * phi(0) / rho
                }
                Boundary::Vacuum => phi(1) / layers[0].h / layers[0].density,
                _ => Complex64::new(0.0, 0.0),
            };
            (0, rho, eta, velocity)
        } else {
            let layer = &layers[interface - 1];
            let j = layer.node_start + layer.intervals;
            let coefficient = b(layer.coefficient_start + layer.intervals);
            let h2 = layer.h * layer.h;
            (
                j,
                layer.density,
                (2.0 + coefficient) / h2 - x,
                (-phi(j - 1) - 0.5 * (coefficient - h2 * x) * phi(j)) / (layer.h * layer.density),
            )
        };
        let (rho2, eta2) = if interface == layers.len() {
            exterior(case, false, omega, x)
        } else {
            let layer = &layers[interface];
            (
                layer.density,
                (2.0 + b(layer.coefficient_start)) / layer.h.powi(2) - x,
            )
        };
        result += kup_ing(sigma, eta1, rho1, eta2, rho2, phi(j), velocity);
    }
    result
}

fn exterior(case: &Case, top: bool, omega: f64, x: Complex64) -> (f64, Complex64) {
    let (boundary, speed, density, loss) = if top {
        (
            &case.surface_boundary,
            case.surface_sound_speed_mps,
            case.surface_density_g_cm3,
            case.surface_attenuation_db_per_wavelength,
        )
    } else {
        (
            &case.bottom_boundary,
            case.bottom_sound_speed_mps,
            case.bottom_density_g_cm3,
            case.bottom_attenuation_db_per_wavelength,
        )
    };
    match boundary {
        Boundary::Vacuum => (1e-9, Complex64::new(1.0, 0.0)),
        Boundary::Rigid => (1e9, Complex64::new(1.0, 0.0)),
        Boundary::FluidHalfSpace => {
            let eta = Complex64::new(omega * omega, 0.0) / complex_speed(speed, loss).powi(2) - x;
            // The reference's top eta has the opposite sign, also used to form U.
            (density, if top { -eta } else { eta })
        }
        _ => unreachable!("rough elastic/table interfaces are rejected by Case validation"),
    }
}

fn scatter_root(z: Complex64) -> Complex64 {
    if z.re >= 0.0 {
        z.sqrt()
    } else {
        -Complex64::i() * (-z).sqrt()
    }
}

fn kup_ing(
    sigma: f64,
    eta1_sq: Complex64,
    rho1: f64,
    eta2_sq: Complex64,
    rho2: f64,
    pressure: Complex64,
    velocity: Complex64,
) -> Complex64 {
    let eta1 = scatter_root(eta1_sq);
    let eta2 = scatter_root(eta2_sq);
    let del = rho1 * eta2 + rho2 * eta1;
    if del == Complex64::new(0.0, 0.0) {
        return del;
    }
    let a11 = 0.5 * (eta1_sq - eta2_sq) - (rho2 * eta1_sq - rho1 * eta2_sq) * (eta1 + eta2) / del;
    let a12 = Complex64::i() * (rho2 - rho1).powi(2) * eta1 * eta2 / del;
    let a21 = -Complex64::i() * (rho2 * eta1_sq - rho1 * eta2_sq).powi(2) / (rho1 * rho2 * del);
    let a22 = 0.5 * (eta1_sq - eta2_sq) + (rho2 - rho1) * eta1 * eta2 * (eta1 + eta2) / del;
    -sigma.powi(2)
        * (-a21 * pressure.powi(2) + (a11 - a22) * pressure * velocity + a12 * velocity.powi(2))
}

#[cfg(test)]
#[test]
fn identical_material_has_no_scatter_and_vacuum_limit_is_passive() {
    let eta = Complex64::new(0.01, -0.001);
    assert_eq!(
        kup_ing(2.0, eta, 1.0, eta, 1.0, 0.3.into(), 0.2.into()),
        0.0.into()
    );
    let loss = kup_ing(
        2.0,
        1.0.into(),
        1e-9,
        0.01.into(),
        1.0,
        0.0.into(),
        0.2.into(),
    );
    assert!((loss - Complex64::new(0.0, -4.0 * 0.1 * 0.04)).norm() < 1e-10);
    assert_eq!(scatter_root((-4.0).into()), Complex64::new(0.0, -2.0));
}

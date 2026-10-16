// Adapted from Acoustics Toolbox v2023.5 Kraken/kraken.f90 and krakenc.f90.
// Copyright (C) 2009 Michael B. Porter. GPL-3.0-or-later; see LICENSE.
//! Mesh-history ownership, not a shared root solver. Raw Neville seeds remain
//! separate from the Richardson table; shapes/loss come only from mesh one.
use crate::{Case, DiagnosticReport, ModeSet, ModeSolver, NormalMode, solver::error};
use num_complex::Complex64;
use std::ops::{Div, Sub};

pub(crate) const MULTIPLIERS: [usize; 5] = [1, 2, 4, 8, 16];

#[derive(Default)]
pub(crate) struct Refinement<T> {
    table: Vec<Vec<T>>,
    seeds: Vec<(f64, Vec<Complex64>)>,
    modes: Vec<NormalMode>,
    perturbations: Vec<Complex64>,
}

impl<T> Refinement<T> {
    pub fn seed(&self, index: usize, h: f64) -> Option<Complex64> {
        // Solve2 keeps the original scan on mesh two. Mesh three is the first
        // with two raw-root histories, including cLow-excluded deflation roots.
        if self.seeds.len() < 2 {
            return None;
        }
        let mut values = self
            .seeds
            .iter()
            .map(|(_, roots)| roots.get(index).copied())
            .collect::<Option<Vec<_>>>()?;
        for width in 1..values.len() {
            for j in 0..values.len() - width {
                let a = self.seeds[j].0.powi(2);
                let b = self.seeds[j + width].0.powi(2);
                values[j] = ((h * h - b) * values[j] - (h * h - a) * values[j + 1]) / (a - b);
            }
        }
        values.first().copied()
    }
}

impl Refinement<f64> {
    // Solve selects M from the previous Richardson row (the raw roots on mesh
    // one), before incorporating the current mesh. Equal minima keep the first.
    pub fn selected_count(&self, roots: &[f64], low: f64) -> Result<usize, DiagnosticReport> {
        self.table
            .first()
            .map_or(roots, Vec::as_slice)
            .iter()
            .take(roots.len())
            .enumerate()
            .filter(|(_, x)| **x > low)
            .min_by(|(i, a), (j, b)| a.total_cmp(b).then_with(|| i.cmp(j)))
            .map(|(i, _)| i + 1)
            .ok_or_else(|| {
                error(
                    "KR0301",
                    "no elastic modes inside spectral limits",
                    "phase_speed_limits",
                )
            })
    }

    pub fn accept(
        &mut self,
        roots: Vec<f64>,
        seed_h: Option<f64>,
        case: &Case,
        mode: impl FnMut(f64) -> Result<(NormalMode, Complex64), DiagnosticReport>,
    ) -> Result<Option<ModeSet>, DiagnosticReport> {
        if let Some(h) = seed_h {
            self.seeds
                .push((h, roots.iter().map(|&x| Complex64::new(x, 0.0)).collect()));
        }
        self.advance(roots, case, mode, f64::abs, |x, perturbation| {
            (x + perturbation).sqrt()
        })
    }
}

impl Refinement<Complex64> {
    pub fn accept(
        &mut self,
        roots: Vec<Complex64>,
        seed_h: f64,
        case: &Case,
        mode: impl FnMut(Complex64) -> Result<(NormalMode, Complex64), DiagnosticReport>,
    ) -> Result<Option<ModeSet>, DiagnosticReport> {
        self.seeds.push((seed_h, roots.clone()));
        let high_k2 = (2.0 * std::f64::consts::PI * case.frequency_hz / case.c_low_mps).powi(2);
        let mut selected: Vec<_> = roots.into_iter().filter(|x| x.re <= high_k2).collect();
        if case.bottom_boundary.is_tabulated()
            || case.surface_boundary.is_tabulated()
            || crate::solver::elastic::has_half_space(case)
            || crate::solver::elastic::has_layers(case)
        {
            selected.sort_by(|a, b| b.re.total_cmp(&a.re));
        }
        if selected.is_empty() {
            return Err(error(
                "KR0301",
                "no complex modes inside spectral limits",
                "phase_speed_limits",
            ));
        }
        if self.table.is_empty()
            && selected
                .len()
                .checked_mul(case.mode_sample_depths_m.len())
                .is_none_or(|w| w > 5_000_000)
        {
            return Err(error(
                "KR0302",
                "mode shape sample limit exceeded",
                "mode_sample_depths_m",
            ));
        }
        self.advance(selected, case, mode, Complex64::norm, |x, perturbation| {
            (x + perturbation).sqrt()
        })
    }
}

impl<T: Copy + Sub<Output = T> + Div<f64, Output = T>> Refinement<T> {
    #[allow(clippy::cast_precision_loss)]
    fn advance(
        &mut self,
        roots: Vec<T>,
        case: &Case,
        mode: impl FnMut(T) -> Result<(NormalMode, Complex64), DiagnosticReport>,
        magnitude: impl FnOnce(T) -> f64,
        wavenumber: impl Fn(T, Complex64) -> Complex64,
    ) -> Result<Option<ModeSet>, DiagnosticReport> {
        let set = self.table.len();
        let multiplier = MULTIPLIERS[set];
        // Both reference solvers reduce M after a spectral exit. Keep only the
        // surviving first-mesh shapes/group speeds/loss and Richardson columns.
        if roots.len() < self.modes.len() {
            self.modes.truncate(roots.len());
            self.perturbations.truncate(roots.len());
            for row in &mut self.table {
                row.truncate(roots.len());
            }
        }
        if set == 0 {
            (self.modes, self.perturbations) = roots
                .iter()
                .copied()
                .map(mode)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .unzip();
        } else if roots.len() != self.modes.len() {
            return Err(error(
                "KR0303",
                if case.mode_solver == ModeSolver::Kraken {
                    format!(
                        "mode count changed during mesh refinement ({} -> {} at multiplier {multiplier}); move spectral limits away from roots",
                        self.modes.len(),
                        roots.len()
                    )
                } else {
                    "mode count changed during KRAKENC mesh refinement; move spectral limits away from roots".into()
                },
                "phase_speed_limits",
            ));
        }
        let key = 2 * self.modes.len() / 3;
        let previous = self.table.first().map(|row| row[key]);
        self.table.push(roots);
        for j in (0..set).rev() {
            let denominator = (multiplier as f64 / MULTIPLIERS[j] as f64).powi(2) - 1.0;
            let (earlier, later) = self.table.split_at_mut(j + 1);
            for (value, &next) in earlier[j].iter_mut().zip(&later[0]) {
                *value = next - (*value - next) / denominator;
            }
        }
        let delta = previous.map_or(1e10, |x| magnitude(self.table[0][key] - x));
        let converged = delta * case.max_range_m < 1.0;
        if !converged {
            return Ok(None);
        }
        let omega = 2.0 * std::f64::consts::PI * case.frequency_hz;
        for ((mode, &x), &perturbation) in self
            .modes
            .iter_mut()
            .zip(&self.table[0])
            .zip(&self.perturbations)
        {
            let k = wavenumber(x, perturbation);
            mode.horizontal_wavenumber_rad_per_m = k;
            mode.phase_speed_mps = omega / k.re;
            mode.attenuation_nepers_per_m = -k.im;
        }
        if self.modes.iter().any(|mode| {
            !mode.phase_speed_mps.is_finite()
                || !mode.group_speed_mps.is_finite()
                || !mode.attenuation_nepers_per_m.is_finite()
                || !mode.horizontal_wavenumber_rad_per_m.re.is_finite()
                || !mode.horizontal_wavenumber_rad_per_m.im.is_finite()
                || mode
                    .eigenfunction
                    .iter()
                    .any(|v| !v.re.is_finite() || !v.im.is_finite())
        }) {
            return Err(if case.mode_solver == ModeSolver::Kraken {
                error("KR0302", "non-finite mode result", "modes")
            } else {
                error("KR0303", "invalid complex mode result", "modes")
            });
        }
        Ok(Some(ModeSet {
            frequency_hz: case.frequency_hz,
            sampled_depths_m: case.mode_sample_depths_m.clone(),
            modes: std::mem::take(&mut self.modes),
        }))
    }
}

#[cfg(test)]
#[test]
fn real_selection_uses_previous_row_current_bound_and_first_tie() {
    let mut refinement = Refinement::<f64>::default();
    assert_eq!(refinement.selected_count(&[9.0, 7.0, 7.0], 1.0).unwrap(), 2);
    refinement.table.push(vec![9.0, 7.0, 8.0, 1.5]);
    assert_eq!(refinement.selected_count(&[8.0, 6.0, 5.0], 1.0).unwrap(), 2);
    assert_eq!(refinement.selected_count(&[8.0], 1.0).unwrap(), 1);
    assert_eq!(
        refinement
            .selected_count(&[8.0], 10.0)
            .unwrap_err()
            .diagnostics()[0]
            .code,
        "KR0301"
    );
}

#[cfg(test)]
#[test]
fn neville_seeds_start_on_the_third_mesh_and_use_raw_roots() {
    let mut refinement = Refinement::<Complex64>::default();
    refinement
        .seeds
        .push((1.0, vec![Complex64::new(10.0, -2.0)]));
    assert_eq!(refinement.seed(0, 0.5), None);
    refinement
        .seeds
        .push((0.5, vec![Complex64::new(7.0, -1.25)]));
    let value = refinement.seed(0, 0.25).unwrap();
    assert_eq!(value.re.to_bits(), 6.25_f64.to_bits());
    assert_eq!(value.im.to_bits(), (-1.0625_f64).to_bits());
    assert_eq!(refinement.seed(1, 0.25), None);
}

//! One layout decision for both the streaming CLI and Python's immutable snapshots.
use super::{mode_data, profile_header};
use crate::directory::Quota;
use crate::native::{self, Direct};
use kraken::{Case, FieldCase, ProfileSimulationResult};
use std::io;
use std::path::Path;

fn layers(case: &Case) -> impl Iterator<Item = (f64, f64)> + '_ {
    std::iter::once(case.fluid_top_depth_m())
        .chain(std::iter::once(case.water_depth_m))
        .chain(
            case.additional_fluid_layers
                .iter()
                .map(|l| l.bottom_depth_m),
        )
        .zip(
            std::iter::once(case.water_density_g_cm3)
                .chain(case.additional_fluid_layers.iter().map(|l| l.density_g_cm3)),
        )
}

fn shared_headers(cases: &[FieldCase]) -> bool {
    let first = &cases[0].profiles()[0];
    cases.iter().all(|block| {
        let other = &block.profiles()[0];
        // Pinned FIELD and MATLAB cannot traverse multiple profiles at multiple frequencies.
        block.profiles().len() == 1
            && first.source_depths_m == other.source_depths_m
            && first.receiver_depths_m == other.receiver_depths_m
            && first.receiver_ranges_m == other.receiver_ranges_m
            && first.title == other.title
            && first.mode_solver == other.mode_solver
            && first.mode_sample_depths_m == other.mode_sample_depths_m
            && layers(first).eq(layers(other))
    })
}

/// Writes ordered results without solving or retaining frequency results.
/// Shared MOD headers use the first frequency's mesh, as in Fortran KRAKEN.
pub struct Writer<'a> {
    root: &'a Path,
    stem: &'a str,
    cases: &'a [FieldCase],
    combined: bool,
    modes: Option<Direct>,
    shade: Option<Direct>,
}

impl<'a> Writer<'a> {
    #[must_use]
    pub fn new(root: &'a Path, stem: &'a str, cases: &'a [FieldCase]) -> Self {
        Self {
            root,
            stem,
            cases,
            combined: cases.len() > 1 && shared_headers(cases),
            modes: None,
            shade: None,
        }
    }

    #[must_use]
    pub fn block_name(&self, index: usize) -> String {
        super::stream::block_name(
            self.stem,
            index,
            if self.combined { 1 } else { self.cases.len() },
        )
    }

    /// Call once per block, in input order. Each completed block is flushed and quota-accounted.
    /// # Errors
    /// Returns native representation, quota or filesystem errors.
    pub fn write_block(
        &mut self,
        index: usize,
        result: &ProfileSimulationResult,
        quota: &mut Quota,
    ) -> Result<(), String> {
        if !self.combined {
            return super::write(
                self.root,
                &self.block_name(index),
                &self.cases[index],
                result,
                quota,
            );
        }
        let mod_path = self.root.join(format!("{}.mod", self.stem));
        self.write_modes(index, result, quota.remaining()?)
            .map_err(|e| e.to_string())?;
        quota.refresh(&mod_path)?;
        let shd_path = self.root.join(format!("{}.shd", self.stem));
        self.write_field(result, quota.remaining()?)
            .map_err(|e| e.to_string())?;
        quota.refresh(&shd_path)
    }

    fn frequencies(&self) -> Vec<f64> {
        self.cases
            .iter()
            .map(|c| c.profiles()[0].frequency_hz)
            .collect()
    }

    fn write_modes(
        &mut self,
        index: usize,
        result: &ProfileSimulationResult,
        remaining: u64,
    ) -> io::Result<()> {
        // The shared header and record length come from the first block; the
        // selector guarantees every later block has the same depths and layers.
        let first = &self.cases[0].profiles()[0];
        let words = (2 * first.mode_sample_depths_m.len())
            .max(3 * (1 + first.additional_fluid_layers.len()))
            .max(32)
            .max(2 * self.cases.len())
            .div_ceil(2)
            * 2;
        if let Some(file) = &mut self.modes {
            file.remaining(remaining);
        } else {
            let mut file = Direct::new(
                &self.root.join(format!("{}.mod", self.stem)),
                words,
                remaining,
            )?;
            profile_header(&mut file, words, first, &self.frequencies())?;
            self.modes = Some(file);
        }
        let file = self.modes.as_mut().expect("opened above");
        mode_data(
            file,
            words,
            &self.cases[index].profiles()[0],
            &result.modes[0],
        )?;
        file.flush()
    }

    #[allow(clippy::cast_possible_truncation)]
    fn write_field(&mut self, result: &ProfileSimulationResult, remaining: u64) -> io::Result<()> {
        let field = &result.field;
        if let Some(file) = &mut self.shade {
            file.remaining(remaining);
        } else {
            self.shade = Some(native::shd_header(
                &self.root.join(format!("{}.shd", self.stem)),
                &self.cases[0].profiles()[0].title,
                &self.frequencies(),
                &field.source_depths_m,
                &field.receiver_depths_m,
                &field.receiver_ranges_m,
                false,
                remaining,
            )?);
        }
        let file = self.shade.as_mut().expect("opened above");
        native::shd_pressure(
            file,
            field.source_depths_m.len(),
            field.receiver_depths_m.len(),
            field.receiver_ranges_m.len(),
            false,
            field.pressure.iter().map(|p| (p.re as f32, p.im as f32)),
        )?;
        file.flush()
    }
}

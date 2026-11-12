//! One layout decision for both the streaming CLI and Python's immutable snapshots.
use super::{mode_data, profile_header};
use crate::directory::{OpenPair, Quota};
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
    // Release the budget only after both files close, including when the writer is dropped.
    open_pair: Option<OpenPair>,
}

impl<'a> Writer<'a> {
    #[must_use]
    pub fn new(root: &'a Path, stem: &'a str, cases: &'a [FieldCase]) -> Self {
        let combined = cases.len() > 1 && shared_headers(cases);
        Self {
            root,
            stem,
            cases,
            combined,
            modes: None,
            shade: None,
            open_pair: combined.then(OpenPair::reserve).flatten(),
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
        let size = self
            .write_modes(index, result, quota.remaining()?)
            .map_err(|e| e.to_string())?;
        quota.account(&mod_path, size)?;
        let shd_path = self.root.join(format!("{}.shd", self.stem));
        let size = self
            .write_field(result, quota.remaining()?)
            .map_err(|e| e.to_string())?;
        quota.account(&shd_path, size)
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
    ) -> io::Result<u64> {
        // The shared header and record length come from the first block; the
        // selector guarantees every later block has the same depths and layers.
        let first = &self.cases[0].profiles()[0];
        let words = (2 * first.mode_sample_depths_m.len())
            .max(3 * (1 + first.additional_fluid_layers.len()))
            .max(32)
            .max(2 * self.cases.len())
            .div_ceil(2)
            * 2;
        let path = self.root.join(format!("{}.mod", self.stem));
        let mut file = if let Some(file) = self.modes.take() {
            file.resume(&path, remaining)?
        } else {
            let mut file = Direct::new(&path, words, remaining)?;
            profile_header(&mut file, words, first, &self.frequencies())?;
            file
        };
        mode_data(
            &mut file,
            words,
            &self.cases[index].profiles()[0],
            &result.modes[0],
        )?;
        let file = file.pause(self.open_pair.is_some())?;
        let size = file.written();
        self.modes = Some(file);
        Ok(size)
    }

    #[allow(clippy::cast_possible_truncation)]
    fn write_field(&mut self, result: &ProfileSimulationResult, remaining: u64) -> io::Result<u64> {
        let field = &result.field;
        let path = self.root.join(format!("{}.shd", self.stem));
        let mut file = if let Some(file) = self.shade.take() {
            file.resume(&path, remaining)?
        } else {
            native::shd_header(
                &path,
                &self.cases[0].profiles()[0].title,
                &self.frequencies(),
                &field.source_depths_m,
                &field.receiver_depths_m,
                &field.receiver_ranges_m,
                false,
                remaining,
            )?
        };
        native::shd_pressure(
            &mut file,
            field.source_depths_m.len(),
            field.receiver_depths_m.len(),
            field.receiver_ranges_m.len(),
            false,
            field.pressure.iter().map(|p| (p.re as f32, p.im as f32)),
        )?;
        let file = file.pause(self.open_pair.is_some())?;
        let size = file.written();
        self.shade = Some(file);
        Ok(size)
    }
}

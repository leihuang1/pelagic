//! The KRAKEN frequency-block stream: the one order in which a run solves and writes its blocks.
//! The order is an interface, because on a failure it decides which diagnostic wins and what
//! partial state exists. HDF5 plugs in as a [`BlockWriter`], which keeps this crate free of it.
use crate::directory::{Format, Quota};
use kraken::input::InputSnapshot;
use kraken::{DiagnosticReport, FieldCase, ProfileSimulationResult};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::{self, BufWriter, Write};
use std::path::Path;

/// Count accepted bytes, subtracting the report's unchanged buffer at quota checkpoints.
struct Report {
    file: BufWriter<std::fs::File>,
    written: u64,
}

impl Report {
    fn physical_size(&self) -> u64 {
        self.written - self.file.buffer().len() as u64
    }
}

impl Write for Report {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.file.write(bytes)?;
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunSummary {
    pub frequency_count: usize,
    pub mode_count: u64,
    pub pressure_count: u64,
}

#[derive(Debug)]
pub enum RunError {
    Input(DiagnosticReport),
    Simulation {
        frequency_index: usize,
        frequency_hz: f64,
        report: DiagnosticReport,
    },
    Output(String),
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(report) => report.fmt(formatter),
            Self::Simulation {
                frequency_index,
                frequency_hz,
                report,
            } => {
                write!(
                    formatter,
                    "frequency[{frequency_index}] ({frequency_hz} Hz): {report}"
                )
            }
            Self::Output(message) => write!(formatter, "error[KR0402]: {message}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<String> for RunError {
    fn from(message: String) -> Self {
        Self::Output(message)
    }
}

/// A format written block by block beside the native pair; `()` writes none.
pub trait BlockWriter {
    /// Write block `index` before its native pair, so that this error beats a native quota error.
    /// # Errors
    /// Returns the format's representation, quota or I/O error.
    fn write_block(
        &mut self,
        index: usize,
        case: &FieldCase,
        result: &ProfileSimulationResult,
    ) -> Result<(), String>;

    /// Check the format again once the block's directory quota has passed. HDF5 needs this second
    /// step: closing the block's group leaves metadata that only a later flush writes, so folding it
    /// into `write_block` would move that growth ahead of the native and directory quotas.
    /// # Errors
    /// Returns the format's quota or I/O error.
    fn check_block(&mut self) -> Result<(), String>;

    /// Refresh the physical sizes of this writer's files, without flushing or enforcing a limit.
    /// # Errors
    /// Returns filesystem or size-overflow errors.
    fn refresh_quota(&self, quota: &mut Quota) -> Result<(), String>;
}

impl BlockWriter for () {
    fn write_block(
        &mut self,
        _: usize,
        _: &FieldCase,
        _: &ProfileSimulationResult,
    ) -> Result<(), String> {
        Ok(())
    }

    fn check_block(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn refresh_quota(&self, _: &mut Quota) -> Result<(), String> {
        Ok(())
    }
}

/// The native stem of block `index` of `count`: the run's own stem when it is the only block.
#[must_use]
pub fn block_name(stem: &str, index: usize, count: usize) -> String {
    if count == 1 {
        stem.to_owned()
    } else {
        format!("{stem}.f{index:04}")
    }
}

/// Solve `cases` through one iterator, which carries the cross-frequency seeds and stops at the
/// first error, and write each block into exclusively owned `root`.
///
/// `{stem}.prt` is created on entry, so whatever a caller does before this call precedes it. Its
/// opening lines are buffered until the first block's flush, so while they fit the buffer, that
/// block's native quota does not count them. Each block then runs, in this order:
///
/// 1. [`BlockWriter::write_block`];
/// 2. the native pair, when `format` selects it;
/// 3. the block's report lines, then a flush;
/// 4. the quota of all of `root`, including anything `writer` wrote there;
/// 5. [`BlockWriter::check_block`].
///
/// The caller keeps `format` and `writer` consistent: `format` is only what the report records.
///
/// # Errors
/// Returns the first solver failure as [`RunError::Simulation`], and a `writer`, native,
/// report or quota failure as [`RunError::Output`]. Earlier blocks stay in `root`.
#[allow(clippy::unnecessary_debug_formatting)] // Quote input paths so embedded newlines cannot spoof report fields.
pub fn run(
    cases: &[FieldCase],
    inputs: &[InputSnapshot],
    root: &Path,
    stem: &str,
    format: Format,
    maximum: u64,
    writer: &mut impl BlockWriter,
) -> Result<RunSummary, RunError> {
    let report_path = root.join(format!("{stem}.prt"));
    let mut report = Report {
        file: crate::create(&report_path).map_err(|e| RunError::Output(e.to_string()))?,
        written: 0,
    };
    writeln!(report, "Pelagic {} KRAKEN run report (not the Fortran PRT format)\nreference: Acoustics Toolbox v2023.5\nformat: {format:?}\nmax_output_bytes: {maximum}", env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())?;
    for input in inputs {
        writeln!(
            report,
            "input {}: {:?} sha256={:x}",
            input.role(),
            input.path().as_os_str(),
            Sha256::digest(input.source().as_bytes())
        )
        .map_err(|e| e.to_string())?;
    }
    let mut quota = Quota::new(root, maximum)?;
    let mut summary = RunSummary {
        frequency_count: cases.len(),
        ..RunSummary::default()
    };
    let mut native = format
        .legacy()
        .then(|| super::Writer::new(root, stem, cases));
    for (index, (case, result)) in cases
        .iter()
        .zip(kraken::solve_frequencies(cases))
        .enumerate()
    {
        let result = result.map_err(|report| RunError::Simulation {
            frequency_index: index,
            frequency_hz: case.profiles()[0].frequency_hz,
            report,
        })?;
        writer.write_block(index, case, &result)?;
        writer.refresh_quota(&mut quota)?;
        quota.account(&report_path, report.physical_size())?;
        let name = if let Some(native) = &mut native {
            native.write_block(index, &result, &mut quota)?;
            native.block_name(index)
        } else {
            block_name(stem, index, cases.len())
        };
        writeln!(report, "frequency[{index}] {} Hz; native_stem={name}; profiles={} ranges_m={:?}; modes={:?}; pressures={}", case.profiles()[0].frequency_hz, case.profiles().len(), case.ranges_m(), result.modes.iter().map(|m| m.modes.len()).collect::<Vec<_>>(), result.field.pressure.len()).map_err(|e| e.to_string())?;
        for (profile, environment) in case.profiles().iter().enumerate() {
            writeln!(report, "  profile[{profile}] title={:?}; solver={:?}; mesh_reference_frequency_hz={}; source_geometry={:?}; mode_addition={:?}", environment.title, environment.mode_solver, environment.mesh_reference_frequency_hz.unwrap_or(environment.frequency_hz), environment.source_geometry, environment.mode_addition).map_err(|e| e.to_string())?;
        }
        report.flush().map_err(|e| e.to_string())?;
        quota.account(&report_path, report.physical_size())?;
        quota.remaining()?;
        summary.mode_count += result
            .modes
            .iter()
            .map(|m| m.modes.len() as u64)
            .sum::<u64>();
        summary.pressure_count += result.field.pressure.len() as u64;
        writer.check_block()?;
    }
    // Every block flushed already; this flush writes only when there were none, and reports its error.
    report.flush().map_err(|e| e.to_string())?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_counts_only_physical_bytes_including_automatic_buffer_flushes() {
        let root = crate::directory::tests::root("report-bytes");
        let path = root.join("case.prt");
        let mut report = Report {
            file: crate::create(&path).unwrap(),
            written: 0,
        };
        let mut quota = Quota::new(&root, 999_999).unwrap();
        let capacity = report.file.capacity();
        for count in [1, capacity - 1, 1, 2 * capacity, 7] {
            report.write_all(&vec![b'x'; count]).unwrap();
            let size = std::fs::metadata(&path).unwrap().len();
            assert_eq!(report.physical_size(), size);
            quota.account(&path, report.physical_size()).unwrap();
            assert_eq!(quota.remaining().unwrap(), 999_999 - size);
        }
        report.flush().unwrap();
        assert_eq!(report.physical_size(), report.written);
        assert_eq!(
            report.physical_size(),
            std::fs::metadata(&path).unwrap().len()
        );
        drop(report);
        std::fs::remove_dir_all(root).unwrap();
    }
}

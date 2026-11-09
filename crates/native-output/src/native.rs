//! Pinned v2023.5 little-endian direct/sequential records, not native-endian Rust structs.
#![allow(clippy::cast_possible_truncation)]
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;

/// Exclusively create a numerical artifact in an owned scratch directory.
/// # Errors
/// Returns filesystem errors, including an already existing destination.
pub fn create(path: &Path) -> io::Result<BufWriter<File>> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(BufWriter::new)
}

pub(crate) fn count(n: usize) -> io::Result<[u8; 4]> {
    i32::try_from(n)
        .map(i32::to_le_bytes)
        .map_err(|_| io::Error::other("native count exceeds i32"))
}

pub(crate) fn single(value: f64) -> io::Result<[u8; 4]> {
    let value = value as f32;
    if !value.is_finite() {
        return Err(io::Error::other("non-finite native single-precision value"));
    }
    Ok(value.to_le_bytes())
}

pub(crate) fn singles(values: &[f64]) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(4 * values.len());
    for &value in values {
        bytes.extend(single(value)?);
    }
    Ok(bytes)
}
pub(crate) fn doubles(values: &[f64]) -> Vec<u8> {
    values.iter().flat_map(|x| x.to_le_bytes()).collect()
}
pub(crate) fn text(value: &str, length: usize) -> Vec<u8> {
    let mut result: Vec<_> = value
        .chars()
        .take(length)
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() {
                c as u8
            } else {
                b' '
            }
        })
        .collect();
    result.resize(length, b' ');
    result
}

pub(crate) fn sequential(file: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    let marker = count(bytes.len())?;
    file.write_all(&marker)?;
    file.write_all(bytes)?;
    file.write_all(&marker)
}

pub(crate) struct Direct {
    file: BufWriter<File>,
    bytes: usize,
    written: u64,
    maximum: u64,
}

/// A [`Direct`] file between appends. It holds no descriptor, so a multi-frequency run does not
/// keep its shared files open while it solves the next frequency.
pub(crate) struct Closed {
    bytes: usize,
    written: u64,
}

// Batch binary records without changing the report buffer's quota checkpoints.
fn buffered(file: File) -> BufWriter<File> {
    BufWriter::with_capacity(64 * 1024, file)
}

impl Direct {
    pub(crate) fn new(path: &Path, words: usize, maximum: u64) -> io::Result<Self> {
        let bytes = words
            .checked_mul(4)
            .ok_or_else(|| io::Error::other("record length overflow"))?;
        count(words)?;
        if bytes as u64 > maximum {
            return Err(io::Error::other("native record exceeds output quota"));
        }
        Ok(Self {
            file: buffered(OpenOptions::new().write(true).create_new(true).open(path)?),
            bytes,
            written: 0,
            maximum,
        })
    }
    pub(crate) fn record(&mut self, mut bytes: Vec<u8>) -> io::Result<()> {
        if bytes.len() > self.bytes {
            return Err(io::Error::other(
                "native payload exceeds declared record length",
            ));
        }
        self.written = self
            .written
            .checked_add(self.bytes as u64)
            .filter(|&n| n <= self.maximum)
            .ok_or_else(|| io::Error::other("native file exceeds output quota"))?;
        bytes.resize(self.bytes, 0);
        self.file.write_all(&bytes)
    }
    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.flush()
    }
    /// Flush and close the file, keeping what [`Closed::reopen`] needs to append to it.
    pub(crate) fn close(mut self) -> io::Result<Closed> {
        self.flush()?;
        Ok(Closed {
            bytes: self.bytes,
            written: self.written,
        })
    }
}

impl Closed {
    /// Reopen `path` to append records of the same length, allowing `remaining` more bytes.
    /// Every record went through [`Direct::record`], so the file must be exactly as long as what
    /// was written; anything else means it changed between blocks.
    pub(crate) fn reopen(self, path: &Path, remaining: u64) -> io::Result<Direct> {
        let file = OpenOptions::new().append(true).open(path)?;
        if file.metadata()?.len() != self.written {
            return Err(io::Error::other("native file changed between blocks"));
        }
        Ok(Direct {
            file: buffered(file),
            bytes: self.bytes,
            written: self.written,
            maximum: self.written.saturating_add(remaining),
        })
    }
}

/// The pressure iterator is already in native [source][depth][range] row order.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shd(
    path: &Path,
    title: &str,
    frequency: f64,
    sources: &[f64],
    depths: &[f64],
    ranges: &[f64],
    irregular: bool,
    pressure: impl Iterator<Item = (f32, f32)>,
    maximum: u64,
) -> io::Result<()> {
    let mut file = shd_header(
        path,
        title,
        &[frequency],
        sources,
        depths,
        ranges,
        irregular,
        maximum,
    )?;
    shd_pressure(
        &mut file,
        sources.len(),
        depths.len(),
        ranges.len(),
        irregular,
        pressure,
    )?;
    file.finish()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn shd_header(
    path: &Path,
    title: &str,
    frequencies: &[f64],
    sources: &[f64],
    depths: &[f64],
    ranges: &[f64],
    irregular: bool,
    maximum: u64,
) -> io::Result<Direct> {
    let words = 41
        .max(2 * frequencies.len())
        .max(2 * ranges.len())
        .max(depths.len())
        .max(sources.len());
    let mut file = Direct::new(path, words, maximum)?;
    let mut header = count(words)?.to_vec();
    header.extend(text(title, 80));
    file.record(header)?;
    file.record(text(if irregular { "irregular" } else { "rectilin" }, 10))?;
    let mut dimensions = Vec::new();
    for n in [
        frequencies.len(),
        1,
        1,
        1,
        sources.len(),
        depths.len(),
        ranges.len(),
    ] {
        dimensions.extend(count(n)?);
    }
    dimensions.extend(frequencies[0].to_le_bytes());
    dimensions.extend(0_f64.to_le_bytes());
    file.record(dimensions)?;
    file.record(doubles(frequencies))?;
    for _ in 0..3 {
        file.record(doubles(&[0.0]))?;
    }
    file.record(singles(sources)?)?;
    file.record(singles(depths)?)?;
    file.record(doubles(ranges))?;
    Ok(file)
}

pub(crate) fn shd_pressure(
    file: &mut Direct,
    sources: usize,
    depths: usize,
    ranges: usize,
    irregular: bool,
    mut pressure: impl Iterator<Item = (f32, f32)>,
) -> io::Result<()> {
    for _ in 0..sources * if irregular { 1 } else { depths } {
        let mut row = Vec::with_capacity(8 * ranges);
        for _ in 0..ranges {
            let (real, imaginary) = pressure
                .next()
                .ok_or_else(|| io::Error::other("incomplete native pressure grid"))?;
            if !real.is_finite() || !imaginary.is_finite() {
                return Err(io::Error::other("non-finite native pressure"));
            }
            row.extend(real.to_le_bytes());
            row.extend(imaginary.to_le_bytes());
        }
        file.record(row)?;
    }
    if pressure.next().is_some() {
        return Err(io::Error::other("trailing native pressure values"));
    }
    Ok(())
}

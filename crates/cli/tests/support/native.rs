//! Independent v2023.5 layout readers. No calls to output's serialization helpers.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp
)]
use hdf5::{File, Group, H5Type};
use std::fs;
use std::path::Path;

pub fn i32_at(bytes: &[u8], offset: usize) -> usize {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
        .try_into()
        .unwrap()
}
pub fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
pub fn f64_at(bytes: &[u8], offset: usize) -> f64 {
    f64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn data<T: H5Type>(group: &Group, name: &str) -> Vec<T> {
    group.dataset(name).unwrap().read_raw().unwrap()
}
fn attr(group: &Group, name: &str) -> f64 {
    group.attr(name).unwrap().read_scalar().unwrap()
}
fn text(group: &Group, name: &str) -> String {
    group
        .attr(name)
        .unwrap()
        .read_scalar::<hdf5::types::VarLenUnicode>()
        .unwrap()
        .as_str()
        .into()
}
fn check_singles(bytes: &[u8], values: &[f64]) {
    for (i, &value) in values.iter().enumerate() {
        assert_eq!(f32_at(bytes, 4 * i).to_bits(), (value as f32).to_bits());
    }
}

#[allow(clippy::too_many_arguments)]
pub fn shd(
    path: &Path,
    frequency: f64,
    sources: &[f64],
    depths: &[f64],
    ranges: &[f64],
    irregular: bool,
    real: &[f32],
    imaginary: &[f32],
) {
    shd_block(
        path,
        &[frequency],
        0,
        sources,
        depths,
        ranges,
        irregular,
        real,
        imaginary,
    );
}

#[allow(clippy::too_many_arguments)]
fn shd_block(
    path: &Path,
    frequencies: &[f64],
    index: usize,
    sources: &[f64],
    depths: &[f64],
    ranges: &[f64],
    irregular: bool,
    real: &[f32],
    imaginary: &[f32],
) {
    let bytes = fs::read(path).unwrap();
    let record_bytes = 4 * i32_at(&bytes, 0);
    assert!(record_bytes >= 164 && record_bytes >= 8 * ranges.len());
    assert_eq!(bytes.len() % record_bytes, 0);
    let rec = |i: usize| &bytes[i * record_bytes..(i + 1) * record_bytes];
    assert_eq!(
        std::str::from_utf8(&rec(1)[..10]).unwrap().trim(),
        if irregular { "irregular" } else { "rectilin" }
    );
    for (i, n) in [
        frequencies.len(),
        1,
        1,
        1,
        sources.len(),
        depths.len(),
        ranges.len(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(i32_at(rec(2), 4 * i), n);
    }
    assert_eq!(f64_at(rec(2), 28), frequencies[0]);
    assert_eq!(f64_at(rec(2), 36), 0.0);
    for (i, &frequency) in frequencies.iter().enumerate() {
        assert_eq!(f64_at(rec(3), 8 * i), frequency);
    }
    for i in 4..7 {
        assert_eq!(f64_at(rec(i), 0), 0.0);
    }
    check_singles(rec(7), sources);
    check_singles(rec(8), depths);
    for (i, &r) in ranges.iter().enumerate() {
        assert_eq!(f64_at(rec(9), 8 * i), r);
    }
    let rows = sources.len() * if irregular { 1 } else { depths.len() };
    assert_eq!(bytes.len(), (10 + frequencies.len() * rows) * record_bytes);
    let first_row = 10 + index * rows;
    assert_eq!(real.len(), rows * ranges.len());
    assert_eq!(imaginary.len(), real.len());
    for index in 0..real.len() {
        let row = rec(first_row + index / ranges.len());
        let offset = 8 * (index % ranges.len());
        assert_eq!(f32_at(row, offset).to_bits(), real[index].to_bits());
        assert_eq!(
            f32_at(row, offset + 4).to_bits(),
            imaginary[index].to_bits()
        );
    }
}

#[allow(clippy::too_many_lines)]
pub fn kraken(root: &Path, input: &Path) {
    // Cargo runs integration tests in crates/cli; CLI snapshots retain workspace-relative paths.
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input = workspace.join(input);
    let input = input.as_path();
    let stem = input.file_stem().unwrap().to_string_lossy();
    let file = File::open(root.join(format!("{stem}.h5"))).unwrap();
    let frequencies = data::<f64>(&file, "frequency_hz");
    let cases = if input.extension().is_some_and(|e| e == "json") {
        kraken::json::load_case_document_named(&fs::read(input).unwrap(), input).unwrap()
    } else {
        let engine = if text(&file, "solver") == "krakenc" {
            kraken::ModeSolver::Krakenc
        } else {
            kraken::ModeSolver::Kraken
        };
        let flp = file.group("inputs/flp").unwrap();
        kraken::legacy::load_field_cases(input, workspace.join(text(&flp, "filename")), engine)
            .unwrap()
    };
    let combined = root.join(format!("{stem}.mod")).exists();
    for (index, &frequency) in frequencies.iter().enumerate() {
        let name = if combined {
            stem.to_string()
        } else {
            format!("{stem}.f{index:04}")
        };
        let group = file.group(&format!("frequencies/{index}")).unwrap();
        let field = group.group("field").unwrap();
        let native_frequencies = if combined {
            frequencies.as_slice()
        } else {
            std::slice::from_ref(&frequency)
        };
        let native_index = if combined { index } else { 0 };
        shd_block(
            &root.join(format!("{name}.shd")),
            native_frequencies,
            native_index,
            &data(&field, "source_depth_m"),
            &data(&field, "receiver_depth_m"),
            &data(&field, "receiver_range_m"),
            false,
            &data(&field, "pressure_real"),
            &data(&field, "pressure_imaginary"),
        );
        let bytes = fs::read(root.join(format!("{name}.mod"))).unwrap();
        let record_bytes = 4 * i32_at(&bytes, 0);
        assert!(record_bytes >= 128 && record_bytes.is_multiple_of(8));
        assert_eq!(bytes.len() % record_bytes, 0);
        let rec = |i: usize| &bytes[i * record_bytes..(i + 1) * record_bytes];
        let mut start = 0;
        for profile in 0..attr(&group, "profile_count") as usize {
            let metadata = if attr(&group, "profile_count") == 1.0 {
                group.clone()
            } else {
                group.group(&format!("profiles/{profile}")).unwrap()
            };
            let modes = metadata.group("modes").unwrap();
            let depths = data::<f64>(&modes, "sample_depth_m");
            let media = metadata.group("media").unwrap();
            let media_count = media.member_names().unwrap().len();
            assert_eq!(i32_at(rec(start), 0) * 4, record_bytes);
            assert!(
                std::str::from_utf8(&rec(start)[4..84])
                    .unwrap()
                    .starts_with(if text(&file, "solver") == "krakenc" {
                        "KRAKENC"
                    } else {
                        "KRAKEN"
                    })
            );
            for (i, n) in [
                native_frequencies.len(),
                media_count,
                depths.len(),
                depths.len(),
            ]
            .into_iter()
            .enumerate()
            {
                assert_eq!(i32_at(rec(start), 84 + 4 * i), n);
            }
            let case = &cases[index].profiles()[profile];
            let header_case = &cases[if combined { 0 } else { index }].profiles()[profile];
            let intervals = kraken::solver::base_mesh_intervals(header_case).unwrap();
            assert_eq!(intervals.len(), media_count);
            for (i, &intervals) in intervals.iter().enumerate() {
                assert_eq!(i32_at(rec(start + 1), 12 * i), intervals);
                assert_eq!(&rec(start + 1)[12 * i + 4..12 * i + 12], b"ACOUSTIC");
                let layer = media.group(&i.to_string()).unwrap();
                assert_eq!(
                    f32_at(rec(start + 2), 8 * i),
                    attr(&layer, "top_depth_m") as f32
                );
                assert_eq!(
                    f32_at(rec(start + 2), 8 * i + 4),
                    attr(&layer, "density_g_cm3") as f32
                );
            }
            for (i, &frequency) in native_frequencies.iter().enumerate() {
                assert_eq!(f64_at(rec(start + 3), 8 * i), frequency);
            }
            check_singles(rec(start + 4), &depths);
            let mut block = start + 5;
            let mut selected = block;
            for i in 0..native_frequencies.len() {
                if i == native_index {
                    selected = block;
                }
                let m = i32_at(rec(block), 0);
                block += 2 + m + m.div_ceil(record_bytes / 8).max(1);
            }
            let next_profile = block;
            let block = selected;
            let wave_real = data::<f64>(&modes, "horizontal_wavenumber_real");
            let wave_imag = data::<f64>(&modes, "horizontal_wavenumber_imaginary");
            let count = wave_real.len();
            assert_eq!(i32_at(rec(block), 0), count);
            for (side, offset, cp, density, loss, depth, boundary) in [
                (
                    "surface",
                    0,
                    case.surface_sound_speed_mps,
                    case.surface_density_g_cm3,
                    case.surface_attenuation_db_per_wavelength,
                    0.0,
                    &case.surface_boundary,
                ),
                (
                    "bottom",
                    25,
                    case.bottom_sound_speed_mps,
                    case.bottom_density_g_cm3,
                    case.bottom_attenuation_db_per_wavelength,
                    case.total_depth_m(),
                    &case.bottom_boundary,
                ),
            ] {
                assert_eq!(
                    rec(block + 1)[offset],
                    text(&metadata, &format!("{side}_boundary")).as_bytes()[0]
                );
                for (value, position) in [(cp, 1), (density, 17), (depth, 21)] {
                    assert_eq!(f32_at(rec(block + 1), offset + position), value as f32);
                }
                let (cs, shear_loss) = if let kraken::Boundary::ElasticHalfSpace {
                    shear_sound_speed_mps,
                    shear_attenuation_db_per_wavelength,
                } = boundary
                {
                    (*shear_sound_speed_mps, *shear_attenuation_db_per_wavelength)
                } else {
                    (0.0, 0.0)
                };
                assert_eq!(f32_at(rec(block + 1), offset + 9), cs as f32);
                assert_eq!(
                    f32_at(rec(block + 1), offset + 13),
                    (shear_loss * cs / (8.685_889_6 * 2.0 * std::f64::consts::PI)) as f32
                );
                if rec(block + 1)[offset] == b'A' {
                    let expected = (loss * cp / (8.685_889_6 * 2.0 * std::f64::consts::PI)) as f32;
                    assert_eq!(f32_at(rec(block + 1), offset + 5), expected);
                }
            }
            let real = data::<f64>(&modes, "eigenfunction_real");
            let imaginary = data::<f64>(&modes, "eigenfunction_imaginary");
            assert_eq!(real.len(), count * depths.len());
            for mode in 0..count {
                for z in 0..depths.len() {
                    assert_eq!(
                        f32_at(rec(block + 2 + mode), 8 * z).to_bits(),
                        (real[mode * depths.len() + z] as f32).to_bits()
                    );
                    assert_eq!(
                        f32_at(rec(block + 2 + mode), 8 * z + 4).to_bits(),
                        (imaginary[mode * depths.len() + z] as f32).to_bits()
                    );
                }
            }
            for mode in 0..count {
                let record = block + 2 + count + mode / (record_bytes / 8);
                let offset = 8 * (mode % (record_bytes / 8));
                assert_eq!(
                    f32_at(rec(record), offset).to_bits(),
                    (wave_real[mode] as f32).to_bits()
                );
                assert_eq!(
                    f32_at(rec(record), offset + 4).to_bits(),
                    (wave_imag[mode] as f32).to_bits()
                );
            }
            start = next_profile;
        }
        assert_eq!(
            start * record_bytes,
            bytes.len(),
            "trailing/missing MOD profiles"
        );
    }
}

pub fn sequential(bytes: &[u8]) -> Vec<&[u8]> {
    let mut pos = 0;
    let mut records = Vec::new();
    while pos < bytes.len() {
        let n = i32_at(bytes, pos);
        assert_eq!(i32_at(bytes, pos + 4 + n), n);
        records.push(&bytes[pos + 4..pos + 4 + n]);
        pos += 8 + n;
    }
    assert_eq!(pos, bytes.len());
    records
}

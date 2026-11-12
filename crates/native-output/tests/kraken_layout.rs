//! Read both layouts as Fortran records; compare numerical payload bytes, not floats.
use kraken::{Case, CaseDefinition, FieldCase, FieldPropagation};
use native_output::directory::Quota;
use native_output::kraken::Writer;
use std::fs;
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("kraken-layout-{name}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn cases() -> Vec<FieldCase> {
    kraken::input::load(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kraken/tests/fixtures/PekerisBroadband.env"),
        None,
        None,
    )
    .unwrap()
    .into_cases()
}
fn change(block: &FieldCase, edit: impl Fn(&mut CaseDefinition)) -> FieldCase {
    let profiles = block
        .profiles()
        .iter()
        .map(|case| {
            let mut definition = case.clone().into_definition();
            edit(&mut definition);
            Case::from_definition(definition).unwrap()
        })
        .collect();
    FieldCase::new(profiles, block.ranges_m().to_vec(), block.propagation()).unwrap()
}

#[test]
fn shared_headers_decide_the_layout_not_frequency_order_or_modal_physics() {
    let scratch = Scratch::new("selection");
    let original = cases();
    let selects = |blocks: &[FieldCase], shared| {
        let writer = Writer::new(&scratch.0, "case", blocks);
        assert_eq!(
            writer.block_name(0),
            if shared { "case" } else { "case.f0000" }
        );
    };
    selects(&original, true); // descending
    selects(&[original[0].clone(), original[0].clone()], true); // repeated
    for edit in [
        (|c: &mut CaseDefinition| c.title.push('!')) as fn(&mut CaseDefinition),
        |c| c.mode_sample_depths_m = vec![0.0, 50.0, 100.0],
        |c| c.source_depths_m = vec![40.0, 50.0],
        |c| {
            c.receiver_depths_m = vec![30.0, 70.0];
            c.receiver_offsets_m = vec![0.0; 2];
        },
        |c| c.receiver_ranges_m = vec![1000.0, 2000.0],
        |c| c.water_density_g_cm3 = 1.1,
    ] {
        let mut blocks = original.clone();
        blocks[1] = change(&blocks[1], edit);
        selects(&blocks, false);
    }
    let mut blocks = original.clone();
    blocks[1] = FieldCase::new(
        vec![blocks[1].profiles()[0].clone(); 2],
        vec![0.0, 1000.0],
        FieldPropagation::Adiabatic,
    )
    .unwrap();
    selects(&blocks, false);
    let multi: Vec<_> = original
        .iter()
        .map(|block| {
            FieldCase::new(
                vec![block.profiles()[0].clone(); 2],
                vec![0.0, 1000.0],
                FieldPropagation::Adiabatic,
            )
            .unwrap()
        })
        .collect();
    selects(&multi, false); // even identical multi-profile headers must stay separate
    assert_eq!(
        Writer::new(&scratch.0, "case", &multi[..1]).block_name(0),
        "case"
    );
    blocks = original;
    blocks[1] = change(&blocks[1], |c| {
        c.bottom_attenuation_db_per_wavelength = 0.1;
        c.mode_limit = 1;
        c.receiver_offsets_m.fill(10.0);
    });
    selects(&blocks, true); // per-frequency half-spaces; offsets/limits are not header fields
}

#[test]
fn multi_profile_frequency_blocks_keep_the_existing_pair_bytes() {
    let scratch = Scratch::new("multi-profile");
    let cases: Vec<_> = cases()
        .iter()
        .map(|block| {
            let first = block.profiles()[0].clone();
            let mut second = first.clone().into_definition();
            second.title.push_str(" second");
            second.mode_sample_depths_m = vec![0.0, 25.0, 50.0, 75.0, 100.0];
            FieldCase::new(
                vec![first, Case::from_definition(second).unwrap()],
                vec![0.0, 1000.0],
                FieldPropagation::Adiabatic,
            )
            .unwrap()
        })
        .collect();
    let mut quota = Quota::new(&scratch.0, u64::MAX).unwrap();
    let mut writer = Writer::new(&scratch.0, "case", &cases);
    for (index, (case, result)) in cases
        .iter()
        .zip(kraken::solve_frequencies(&cases))
        .enumerate()
    {
        let result = result.unwrap();
        writer.write_block(index, &result, &mut quota).unwrap();
        native_output::kraken::write(
            &scratch.0,
            &format!("old{index}"),
            case,
            &result,
            &mut quota,
        )
        .unwrap();
        for extension in ["mod", "shd"] {
            assert_eq!(
                fs::read(scratch.0.join(format!("case.f{index:04}.{extension}"))).unwrap(),
                fs::read(scratch.0.join(format!("old{index}.{extension}"))).unwrap()
            );
        }
    }
    assert!(!scratch.0.join("case.mod").exists());
    assert!(!scratch.0.join("case.shd").exists());
}

fn number(bytes: &[u8], offset: usize) -> usize {
    usize::try_from(i32::from_le_bytes(
        bytes[offset..offset + 4].try_into().unwrap(),
    ))
    .unwrap()
}
fn records(bytes: &[u8]) -> Vec<&[u8]> {
    let size = 4 * number(bytes, 0);
    assert_eq!(bytes.len() % size, 0);
    bytes.chunks_exact(size).collect()
}
fn mode_payload(records: &[&[u8]], start: &mut usize, depths: usize) -> Vec<u8> {
    let m = number(records[*start], 0);
    let mut bytes = records[*start][..4].to_vec();
    bytes.extend(&records[*start + 1][..50]);
    for mode in 0..m {
        bytes.extend(&records[*start + 2 + mode][..8 * depths]);
    }
    let wave_records = m.div_ceil(records[0].len() / 8).max(1);
    let waves: Vec<_> = records[*start + 2 + m..*start + 2 + m + wave_records].concat();
    bytes.extend(&waves[..8 * m]);
    *start += 2 + m + wave_records;
    bytes
}

#[test]
fn single_profile_frequency_payloads_equal_separate_files() {
    let scratch = Scratch::new("payloads");
    let originals = cases();
    // Repeated, descending frequencies; the frequency vector forces a larger record.
    let cases: Vec<_> = (0..37).map(|i| originals[i % 3].clone()).collect();
    let solved: Vec<_> = kraken::solve_frequencies(&cases[..3])
        .map(Result::unwrap)
        .collect();
    let mut quota = Quota::new(&scratch.0, u64::MAX).unwrap();
    let mut writer = Writer::new(&scratch.0, "all", &cases);
    for (index, case) in cases.iter().enumerate() {
        let mut result = solved[index % 3].clone();
        if index == 1 {
            result.modes[0].modes.clear();
        } // zero-mode record accounting
        if index == 2 {
            let mode = result.modes[0].modes[0].clone();
            result.modes[0].modes.resize(90, mode); // folded wavenumber records
        }
        writer.write_block(index, &result, &mut quota).unwrap();
        native_output::kraken::write(
            &scratch.0,
            &format!("old{index}"),
            case,
            &result,
            &mut quota,
        )
        .unwrap();
    }
    // The writer flushes both files after every block, so they are complete here.
    let bytes = fs::read(scratch.0.join("all.mod")).unwrap();
    let rec = records(&bytes);
    assert_eq!(number(rec[0], 84), 37);
    let depths = number(rec[0], 92);
    for (i, case) in cases.iter().enumerate() {
        assert_eq!(
            &rec[3][i * 8..(i + 1) * 8],
            &case.profiles()[0].frequency_hz.to_le_bytes()
        );
    }
    let mut start = 5;
    for index in 0..37 {
        let old = fs::read(scratch.0.join(format!("old{index}.mod"))).unwrap();
        let old = records(&old);
        assert_eq!(number(old[0], 92), depths);
        let mut pos = 5;
        assert_eq!(
            mode_payload(&rec, &mut start, depths),
            mode_payload(&old, &mut pos, depths)
        );
        assert_eq!(pos, old.len());
    }
    assert_eq!(start, rec.len());
    let bytes = fs::read(scratch.0.join("all.shd")).unwrap();
    let rec = records(&bytes);
    let rows = number(rec[2], 16) * number(rec[2], 20);
    let ranges = number(rec[2], 24);
    assert_eq!(number(rec[2], 0), 37);
    assert_eq!(rec.len(), 10 + 37 * rows);
    for index in 0..37 {
        let old = fs::read(scratch.0.join(format!("old{index}.shd"))).unwrap();
        let old = records(&old);
        for row in 0..rows {
            assert_eq!(
                &rec[10 + index * rows + row][..8 * ranges],
                &old[10 + row][..8 * ranges]
            );
        }
    }
}

//! The KRAKEN frequency-block stream at its interface: order, naming, quota and report.
//! The `BlockWriter` here records what is on disk at each of its steps; none of this builds HDF5.
use Step::{Checked, Written};
use kraken::input::FieldInput;
use kraken::{FieldCase, ProfileSimulationResult};
use native_output::directory::Format;
use native_output::kraken::stream::{self, BlockWriter, RunError, RunSummary, block_name};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

const NO_LIMIT: u64 = u64::MAX;
/// Every total and limit in a measured run has six digits, so that the `max_output_bytes` line
/// has one length and tightening a limit does not change the report it is measured against.
const PADDING: usize = 100_000;
const MEASURING_LIMIT: u64 = 999_999;
const PEKERIS_FREQUENCIES: [&str; 3] = ["75", "50", "62.5"];

type Files = BTreeMap<String, Vec<u8>>;

struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "pelagic-kraken-stream-{test}-{}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    /// The three-frequency Pekeris fixture, with `edit` applied to its ENV.
    fn input(&self, edit: impl FnOnce(String) -> String) -> FieldInput {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../kraken/tests/fixtures/PekerisBroadband");
        let env = self.0.join("case.env");
        let source = fs::read_to_string(fixture.with_extension("env")).unwrap();
        fs::write(&env, edit(source)).unwrap();
        fs::copy(fixture.with_extension("flp"), self.0.join("case.flp")).unwrap();
        kraken::input::load(&env, None, None).unwrap()
    }

    fn pekeris(&self) -> FieldInput {
        self.input(|source| source)
    }

    fn root(&self, name: &str) -> PathBuf {
        let root = self.0.join(name);
        fs::create_dir(&root).unwrap();
        root
    }

    fn padded_root(&self, name: &str) -> PathBuf {
        let root = self.root(name);
        fs::write(root.join("padding"), vec![0; PADDING]).unwrap();
        root
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn on_disk(root: &Path) -> Files {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let bytes = fs::read(entry.path()).unwrap();
            (entry.file_name().into_string().unwrap(), bytes)
        })
        .filter(|(name, _)| name != "padding")
        .collect()
}

fn total(files: &Files) -> u64 {
    let bytes: usize = files.values().map(Vec::len).sum();
    (PADDING + bytes) as u64
}

fn names(files: &Files) -> Vec<&str> {
    files.keys().map(String::as_str).collect()
}

fn report(files: &Files) -> &str {
    std::str::from_utf8(&files["case.prt"]).unwrap()
}

fn has_block(report: &str, block: usize) -> bool {
    report.contains(&format!("frequency[{block}] "))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Step {
    Written(usize),
    Checked(usize),
}

/// What was on disk when the `BlockWriter` took one step.
struct Seen {
    step: Step,
    files: Files,
}

/// Records the directory at each step, then returns `act`'s outcome for that step.
struct Recorder<'a, F> {
    root: &'a Path,
    seen: Vec<Seen>,
    modes: u64,
    pressures: u64,
    act: F,
}

impl<'a, F: FnMut(Step) -> Result<(), String>> Recorder<'a, F> {
    fn new(root: &'a Path, act: F) -> Self {
        Self {
            root,
            seen: Vec::new(),
            modes: 0,
            pressures: 0,
            act,
        }
    }

    fn record(&mut self, step: Step) -> Result<(), String> {
        let files = on_disk(self.root);
        self.seen.push(Seen { step, files });
        (self.act)(step)
    }
}

impl<F: FnMut(Step) -> Result<(), String>> BlockWriter for Recorder<'_, F> {
    fn write_block(
        &mut self,
        index: usize,
        _: &FieldCase,
        result: &ProfileSimulationResult,
    ) -> Result<(), String> {
        self.modes += result
            .modes
            .iter()
            .map(|m| m.modes.len() as u64)
            .sum::<u64>();
        self.pressures += result.field.pressure.len() as u64;
        self.record(Written(index))
    }

    fn check_block(&mut self) -> Result<(), String> {
        let index = match self.seen.last().map(|seen| seen.step) {
            Some(Written(index)) => index,
            step => panic!("check_block after {step:?}"),
        };
        self.record(Checked(index))
    }

    fn refresh_quota(&self, quota: &mut native_output::directory::Quota) -> Result<(), String> {
        let path = self.root.join("case.h5");
        if path.exists() {
            quota.refresh(&path)?;
        }
        Ok(())
    }
}

fn recorded(
    input: &FieldInput,
    root: &Path,
    format: Format,
    maximum: u64,
    act: impl FnMut(Step) -> Result<(), String>,
) -> (Result<RunSummary, RunError>, Vec<Seen>) {
    let mut recorder = Recorder::new(root, act);
    let result = stream::run(
        input.cases(),
        input.snapshots(),
        root,
        "case",
        format,
        maximum,
        &mut recorder,
    );
    (result, recorder.seen)
}

fn steps(seen: &[Seen]) -> Vec<Step> {
    seen.iter().map(|seen| seen.step).collect()
}

/// Both steps of each of the first `count` blocks.
fn complete(count: usize) -> Vec<Step> {
    (0..count)
        .flat_map(|index| [Written(index), Checked(index)])
        .collect()
}

fn then(mut steps: Vec<Step>, step: Step) -> Vec<Step> {
    steps.push(step);
    steps
}

/// The sizes of a run that stays within its limit, from which each test places a tighter one.
struct Measured {
    /// Directory total when each block's write step ran, then at the end of the run.
    totals: Vec<u64>,
    files: Files,
}

impl Measured {
    fn new(input: &FieldInput, root: &Path, format: Format) -> Self {
        let (result, seen) = recorded(input, root, format, MEASURING_LIMIT, |_| Ok(()));
        result.unwrap();
        let files = on_disk(root);
        let mut totals: Vec<_> = seen
            .iter()
            .filter(|seen| matches!(seen.step, Written(_)))
            .map(|seen| total(&seen.files))
            .collect();
        totals.push(total(&files));
        Self { totals, files }
    }

    fn size(&self, name: &str) -> u64 {
        self.files[name].len() as u64
    }
}

fn output_error(result: Result<RunSummary, RunError>) -> String {
    match result {
        Err(RunError::Output(message)) => message,
        other => panic!("expected an output error, got {other:?}"),
    }
}

#[test]
fn block_name_is_the_stem_for_one_block_and_indexed_for_several() {
    assert_eq!(block_name("case", 0, 1), "case");
    assert_eq!(block_name("case", 0, 2), "case.f0000");
    assert_eq!(block_name("case", 999, 1000), "case.f0999");
    assert_eq!(block_name("case", 12345, 20000), "case.f12345");
}

#[test]
fn native_pairs_and_report_use_the_block_names() {
    let scratch = Scratch::new("naming");
    let input = scratch.pekeris();
    let run = |cases, root: &Path| {
        stream::run(
            cases,
            input.snapshots(),
            root,
            "case",
            Format::Legacy,
            NO_LIMIT,
            &mut (),
        )
        .unwrap();
        on_disk(root)
    };

    let single = run(&input.cases()[..1], &scratch.root("single"));
    assert_eq!(names(&single), ["case.mod", "case.prt", "case.shd"]);
    assert!(report(&single).contains("frequency[0] 75 Hz; native_stem=case; profiles=1 "));

    let several = run(input.cases(), &scratch.root("several"));
    assert_eq!(
        names(&several),
        [
            "case.f0000.mod",
            "case.f0000.shd",
            "case.f0001.mod",
            "case.f0001.shd",
            "case.f0002.mod",
            "case.f0002.shd",
            "case.prt"
        ]
    );
    for (index, frequency) in PEKERIS_FREQUENCIES.iter().enumerate() {
        let line = format!("frequency[{index}] {frequency} Hz; native_stem=case.f000{index}; ");
        assert!(report(&several).contains(&line), "{line}");
    }
}

#[test]
fn write_step_precedes_the_native_pair_and_check_step_follows_the_flushed_report() {
    let scratch = Scratch::new("order");
    let input = scratch.pekeris();
    let root = scratch.root("out");
    let (result, seen) = recorded(&input, &root, Format::Legacy, NO_LIMIT, |_| Ok(()));
    result.unwrap();
    let last = on_disk(&root);
    assert_eq!(steps(&seen), complete(3));
    for seen in &seen {
        // Blocks before `done` are complete on disk; the others have not started.
        let done = match seen.step {
            Written(index) => index,
            Checked(index) => index + 1,
        };
        for block in 0..3 {
            for extension in ["mod", "shd"] {
                let file = format!("{}.{extension}", block_name("case", block, 3));
                if block < done {
                    assert_eq!(seen.files.get(&file), last.get(&file), "{file} incomplete");
                } else {
                    assert!(!seen.files.contains_key(&file), "{file} already written");
                }
            }
        }
        let flushed = report(&seen.files);
        assert!(report(&last).starts_with(flushed));
        for block in 0..3 {
            assert_eq!(has_block(flushed, block), block < done);
        }
        // The first block's native quota must not count the buffered opening lines.
        assert_eq!(flushed.is_empty(), done == 0);
        assert!(flushed.is_empty() || flushed.ends_with('\n'));
    }
}

#[test]
fn summary_counts_every_block_the_writer_saw() {
    let scratch = Scratch::new("summary");
    let input = scratch.pekeris();
    let root = scratch.root("out");
    let mut recorder = Recorder::new(&root, |_| Ok(()));
    let summary = stream::run(
        input.cases(),
        input.snapshots(),
        &root,
        "case",
        Format::Legacy,
        NO_LIMIT,
        &mut recorder,
    )
    .unwrap();
    assert_eq!(
        summary,
        RunSummary {
            frequency_count: 3,
            mode_count: recorder.modes,
            pressure_count: recorder.pressures
        }
    );
    assert!(recorder.modes > 0 && recorder.pressures > 0);
}

#[test]
#[allow(clippy::unnecessary_debug_formatting)] // The report quotes input paths this way.
fn report_opens_with_the_run_and_its_inputs_then_describes_each_block() {
    let scratch = Scratch::new("report");
    let input = scratch.pekeris();
    let root = scratch.root("out");
    let (result, _) = recorded(&input, &root, Format::Both, NO_LIMIT, |_| Ok(()));
    result.unwrap();
    let files = on_disk(&root);
    let mut opening = format!(
        "Pelagic {} KRAKEN run report (not the Fortran PRT format)\n\
         reference: Acoustics Toolbox v2023.5\n\
         format: Both\n\
         max_output_bytes: {NO_LIMIT}\n",
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(input.snapshots().len(), 2);
    for snapshot in input.snapshots() {
        writeln!(
            opening,
            "input {}: {:?} sha256={:x}",
            snapshot.role(),
            snapshot.path().as_os_str(),
            Sha256::digest(snapshot.source().as_bytes())
        )
        .unwrap();
    }
    let blocks = report(&files).strip_prefix(opening.as_str()).unwrap();
    let lines: Vec<_> = blocks.lines().collect();
    assert_eq!(lines.len(), 6);
    assert!(lines[0].starts_with("frequency[0] 75 Hz; native_stem=case.f0000; profiles=1 "));
    assert!(lines[1].starts_with("  profile[0] title=\"Derived broadband Pekeris"));
    assert!(lines[2].starts_with("frequency[1] 50 Hz; native_stem=case.f0001; profiles=1 "));
    assert!(lines[4].starts_with("frequency[2] 62.5 Hz; native_stem=case.f0002; profiles=1 "));
}

#[test]
fn solver_failure_keeps_earlier_blocks_and_starts_no_later_block() {
    let scratch = Scratch::new("solver");
    let input = scratch.input(|source| source.replace("75.0 50.0 62.5 /", "75.0 5.0 62.5 /"));
    let root = scratch.root("out");
    let (result, seen) = recorded(&input, &root, Format::Legacy, NO_LIMIT, |_| Ok(()));

    let error = result.unwrap_err();
    assert!(
        error.to_string().starts_with("frequency[1] (5 Hz): "),
        "{error}"
    );
    let RunError::Simulation {
        frequency_index: 1,
        frequency_hz,
        report: diagnostics,
    } = error
    else {
        panic!("expected block 1's solver failure, got {error:?}");
    };
    assert_eq!(frequency_hz.to_bits(), 5_f64.to_bits());
    assert_ne!(diagnostics.diagnostics(), []);

    assert_eq!(steps(&seen), complete(1));
    let files = on_disk(&root);
    assert_eq!(
        names(&files),
        ["case.f0000.mod", "case.f0000.shd", "case.prt"]
    );
    assert!(has_block(report(&files), 0));
    assert!(!has_block(report(&files), 1));
}

#[test]
fn native_quota_stops_the_block_before_its_report_lines() {
    let scratch = Scratch::new("native-quota");
    let input = scratch.pekeris();
    let measured = Measured::new(&input, &scratch.padded_root("measured"), Format::Legacy);
    let before = measured.totals[1];
    let (modes, field) = (
        measured.size("case.f0001.mod"),
        measured.size("case.f0001.shd"),
    );
    for (name, limit, field_started) in [
        ("modes", before + modes - 1, false),
        ("field", before + modes + field - 1, true),
    ] {
        let root = scratch.padded_root(name);
        let (result, seen) = recorded(&input, &root, Format::Legacy, limit, |_| Ok(()));
        assert_eq!(output_error(result), "native file exceeds output quota");
        assert_eq!(steps(&seen), then(complete(1), Written(1)));
        let files = on_disk(&root);
        assert!(files.contains_key("case.f0000.shd"));
        assert_eq!(files.contains_key("case.f0001.shd"), field_started);
        assert!(!files.contains_key("case.f0002.mod"));
        assert!(has_block(report(&files), 0));
        assert!(!has_block(report(&files), 1));
    }
}

#[test]
fn report_lines_that_cross_the_quota_stop_the_stream_after_flushing_them() {
    let scratch = Scratch::new("report-quota");
    let input = scratch.pekeris();
    let measured = Measured::new(&input, &scratch.padded_root("measured"), Format::Legacy);
    // Each limit fits the block's native pair, which precedes its report lines, by construction.
    for crossing in [1, 2] {
        let limit = measured.totals[crossing + 1] - 1;
        let root = scratch.padded_root(&format!("block-{crossing}"));
        let (result, seen) = recorded(&input, &root, Format::Legacy, limit, |_| Ok(()));
        assert_eq!(
            output_error(result),
            format!("result directory exceeds {limit} bytes")
        );
        assert_eq!(steps(&seen), then(complete(crossing), Written(crossing)));
        let files = on_disk(&root);
        assert_eq!(total(&files), limit + 1);
        assert!(has_block(report(&files), crossing));
        let name = block_name("case", crossing, 3);
        assert_eq!(
            files[&format!("{name}.shd")],
            measured.files[&format!("{name}.shd")]
        );
    }

    let root = scratch.padded_root("exactly");
    let (result, seen) = recorded(
        &input,
        &root,
        Format::Legacy,
        measured.totals[3],
        |_| Ok(()),
    );
    assert_eq!(result.unwrap().frequency_count, 3);
    assert_eq!(steps(&seen), complete(3));
}

#[test]
fn write_step_error_beats_a_native_quota_error_for_the_same_block() {
    let scratch = Scratch::new("write-error");
    let input = scratch.pekeris();
    let measured = Measured::new(&input, &scratch.padded_root("measured"), Format::Legacy);
    // Too small for block 1's MOD, which the write step precedes.
    let limit = measured.totals[1] + measured.size("case.f0001.mod") - 1;
    let root = scratch.padded_root("out");
    let refuse = |step| {
        if step == Written(1) {
            Err("write refused".to_owned())
        } else {
            Ok(())
        }
    };
    let (result, seen) = recorded(&input, &root, Format::Legacy, limit, refuse);
    assert_eq!(output_error(result), "write refused");
    assert_eq!(steps(&seen), then(complete(1), Written(1)));
    let files = on_disk(&root);
    assert!(!files.contains_key("case.f0001.mod"));
    assert!(has_block(report(&files), 0));
    assert!(!has_block(report(&files), 1));
}

#[test]
fn check_step_error_keeps_its_block_and_starts_no_later_block() {
    let scratch = Scratch::new("check-error");
    let input = scratch.pekeris();
    let root = scratch.root("out");
    let refuse = |step| {
        if step == Checked(1) {
            Err("check refused".to_owned())
        } else {
            Ok(())
        }
    };
    let (result, seen) = recorded(&input, &root, Format::Legacy, NO_LIMIT, refuse);
    assert_eq!(output_error(result), "check refused");
    assert_eq!(steps(&seen), complete(2));
    let files = on_disk(&root);
    assert!(files.contains_key("case.f0001.shd"));
    assert!(!files.contains_key("case.f0002.mod"));
    assert!(has_block(report(&files), 1));
    assert!(!has_block(report(&files), 2));
}

#[test]
fn quota_counts_what_the_writer_wrote_and_precedes_its_check_step() {
    let scratch = Scratch::new("writer-file");
    let input = scratch.pekeris();
    let measured = Measured::new(&input, &scratch.padded_root("measured"), Format::Legacy);
    // Fits exactly without the writer's byte, so that byte alone crosses after block 1's report.
    let limit = measured.totals[2];
    let root = scratch.padded_root("out");
    let write = |step| {
        if step == Written(1) {
            fs::write(root.join("case.h5"), [0]).map_err(|e| e.to_string())
        } else {
            Ok(())
        }
    };
    let (result, seen) = recorded(&input, &root, Format::Legacy, limit, write);
    assert_eq!(
        output_error(result),
        format!("result directory exceeds {limit} bytes")
    );
    assert_eq!(steps(&seen), then(complete(1), Written(1)));
    assert!(has_block(report(&on_disk(&root)), 1));
}

#[test]
fn non_legacy_format_writes_only_the_report_and_still_checks_the_quota() {
    let scratch = Scratch::new("non-legacy");
    let input = scratch.pekeris();
    let root = scratch.padded_root("measured");
    let measured = Measured::new(&input, &root, Format::Hdf5);
    assert_eq!(names(&measured.files), ["case.prt"]);
    let text = report(&measured.files);
    assert!(text.contains("format: Hdf5\n"));
    for (index, frequency) in PEKERIS_FREQUENCIES.iter().enumerate() {
        let line = format!("frequency[{index}] {frequency} Hz; native_stem=case.f000{index}; ");
        assert!(text.contains(&line), "{line}");
    }

    let limit = measured.totals[2] - 1;
    let root = scratch.padded_root("limited");
    let (result, seen) = recorded(&input, &root, Format::Hdf5, limit, |_| Ok(()));
    assert_eq!(
        output_error(result),
        format!("result directory exceeds {limit} bytes")
    );
    assert_eq!(steps(&seen), then(complete(1), Written(1)));
    assert_eq!(names(&on_disk(&root)), ["case.prt"]);
}

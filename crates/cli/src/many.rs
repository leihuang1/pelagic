#![forbid(unsafe_code)]
//! `run-many`: independent cases, each published as `run` publishes it, with
//! one status line per case, round-robin shards and `--resume`.

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Diagnostics;

pub(crate) const STATUS_VERSION: u32 = 1;
/// Every status record starts with this, so even a first record that a killed
/// job cut short shows that the file is a status file.
const STATUS_PREFIX: &[u8] = b"{\"status_version\":";

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Case files (.json or .env) and directories of them; directories are not searched recursively.
    inputs: Vec<PathBuf>,
    /// File with one case path per line; - reads the list from stdin.
    #[arg(long)]
    list: Option<PathBuf>,
    /// Directory for the results, one `<stem>/` per case, and the status file.
    #[arg(short, long)]
    output: PathBuf,
    #[arg(long, value_enum, default_value = "legacy")]
    pub format: crate::Format,
    /// Cases solved at once [default: one per available CPU, at most one per case].
    #[arg(long)]
    workers: Option<NonZeroUsize>,
    /// Solve only shard I of N (0-based): the cases whose position in the sorted list is I modulo N.
    #[arg(long, value_name = "I/N", value_parser = parse_shard)]
    shard: Option<Shard>,
    /// Skip cases whose recorded results are current: same input and format, results unmodified.
    #[arg(long)]
    resume: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Shard {
    index: usize,
    count: usize,
}

fn parse_shard(value: &str) -> Result<Shard, String> {
    let (index, count) = value
        .split_once('/')
        .ok_or("expected I/N, for example 0/4")?;
    let index = index
        .parse()
        .map_err(|_| "shard I must be a whole number")?;
    let count = count
        .parse()
        .map_err(|_| "shard N must be a whole number")?;
    if index >= count {
        return Err("shard I must be below N".into());
    }
    Ok(Shard { index, count })
}

/// One model's `run`, split so that each case is read once: the snapshot that
/// is fingerprinted for `--resume` is the one that is solved.
pub(crate) trait Model: Sync {
    /// The model's options that change its results, recorded with each case.
    fn options(&self) -> serde_json::Value;
    /// Read a case as `run` does; a failure is the case's outcome.
    fn load(&self, case: &Path) -> Result<Box<dyn Loaded + '_>, Outcome>;
}

/// A case read by [`Model::load`].
pub(crate) trait Loaded {
    /// The exact bytes of every input file the case read, in reading order.
    fn inputs(&self) -> Vec<&[u8]>;
    /// Solve this snapshot and publish it to `output`, exactly as `run` does.
    fn run(self: Box<Self>, output: &Path) -> Outcome;
}

/// What `run` would have printed, and its exit code.
pub(crate) struct Outcome {
    pub code: u8,
    pub diagnostics: Vec<Rendered>,
    pub summary: Option<String>,
}

pub(crate) struct Rendered {
    json: serde_json::Value,
    text: String,
}

impl Rendered {
    pub(crate) fn of<T: Serialize + fmt::Display>(diagnostic: &T) -> Self {
        Self {
            json: serde_json::to_value(diagnostic).unwrap_or_default(),
            text: diagnostic.to_string(),
        }
    }
}

/// A front-end diagnostic, outside any one case.
#[derive(Serialize)]
struct Problem {
    code: &'static str,
    severity: &'static str,
    field: &'static str,
    message: String,
    source: Option<()>,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}]: {} ({})", self.code, self.message, self.field)
    }
}

fn problem(code: &'static str, field: &'static str, message: impl Into<String>) -> Problem {
    Problem {
        code,
        severity: "error",
        field,
        message: message.into(),
        source: None,
    }
}

/// One status line per finished case. A resumed case is skipped only when
/// everything its results depend on matches: case, version, format, options
/// and input bytes, and the results themselves are unmodified.
#[derive(Debug, Serialize, Deserialize)]
struct Record {
    status_version: u32,
    case: String,
    stem: String,
    /// The Pelagic version, which the results record.
    version: String,
    format: String,
    options: serde_json::Value,
    /// SHA-256 over every input file the case read, auxiliary files included.
    input_sha256: Option<String>,
    /// `ok`, `failed` or `skipped`.
    outcome: String,
    exit_code: u8,
    seconds: f64,
    /// The result directory's manifest after this case, when it succeeded.
    manifest_sha256: Option<String>,
    diagnostics: Vec<serde_json::Value>,
}

struct Case {
    path: PathBuf,
    stem: String,
}

/// A finished case: its status line, and what `run` would have printed.
struct Finished {
    record: Record,
    text: Vec<String>,
    summary: Option<String>,
}

pub(crate) fn execute(
    args: &Args,
    threads: Option<NonZeroUsize>,
    diagnostics: Diagnostics,
    model: &dyn Model,
) -> ExitCode {
    let cases = match cases(args) {
        Ok(cases) => cases,
        Err(problem) => {
            diagnostics.render(&[problem]);
            return ExitCode::from(2);
        }
    };
    let shard = args.shard.unwrap_or(Shard { index: 0, count: 1 });
    let cases: Vec<Case> = cases
        .into_iter()
        .skip(shard.index)
        .step_by(shard.count)
        .collect();
    let status = args.output.join(match args.shard {
        Some(Shard { index, count }) => format!("status-{index}-of-{count}.ndjson"),
        None => "status.ndjson".into(),
    });
    let previous = if args.resume {
        match previous(&args.output) {
            Ok(previous) => previous,
            Err(error) => {
                diagnostics.render(&[problem("PG0001", "output", error.to_string())]);
                return ExitCode::from(2);
            }
        }
    } else {
        HashMap::new()
    };
    let file = match open_status(&status, &cases) {
        Ok(file) => file,
        Err(message) => {
            diagnostics.render(&[problem("PG0404", "output", message)]);
            return ExitCode::from(4);
        }
    };
    let available = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    let workers = args
        .workers
        .map_or(
            available / threads.map_or(1, NonZeroUsize::get),
            NonZeroUsize::get,
        )
        .clamp(1, cases.len().max(1));
    let threads = threads
        .unwrap_or_else(|| NonZeroUsize::new(available / workers).unwrap_or(NonZeroUsize::MIN));
    let report = Mutex::new(Report {
        file,
        status: status.clone(),
        diagnostics,
        tally: [0; 3],
        code: 0,
    });
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let drain = || {
        while !stop.load(Ordering::Relaxed) {
            let Some(case) = cases.get(next.fetch_add(1, Ordering::Relaxed)) else {
                return;
            };
            let finished = solve(case, args, &previous, threads, model);
            let mut report = report
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !report.record(case, &finished) {
                stop.store(true, Ordering::Relaxed);
            }
        }
    };
    std::thread::scope(|scope| {
        // A thread the system refuses leaves its share to the others.
        let _workers: Vec<_> = (1..workers)
            .map_while(|_| std::thread::Builder::new().spawn_scoped(scope, drain).ok())
            .collect();
        drain();
    });
    let report = report
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let [ok, failed, skipped] = report.tally;
    println!(
        "{} cases: {ok} ok, {failed} failed, {skipped} skipped; status in {}",
        cases.len(),
        status.display()
    );
    ExitCode::from(report.code)
}

fn cases(args: &Args) -> Result<Vec<Case>, Problem> {
    if args.inputs.is_empty() && args.list.is_none() {
        return Err(problem(
            "PG0002",
            "arguments",
            "give case files, directories or --list",
        ));
    }
    let mut paths = Vec::new();
    for input in &args.inputs {
        if input == Path::new("-") {
            return Err(problem(
                "PG0002",
                "arguments",
                "run-many reads cases from files; use --list - for a list on stdin",
            ));
        }
        if input.is_dir() {
            let unreadable = |error: io::Error| {
                problem(
                    "PG0001",
                    "inputs",
                    format!("unable to read {}: {error}", input.display()),
                )
            };
            for entry in fs::read_dir(input).map_err(unreadable)? {
                let path = entry.map_err(unreadable)?.path();
                if path.is_file()
                    && path
                        .extension()
                        .is_some_and(|extension| extension == "json" || extension == "env")
                {
                    paths.push(path);
                }
            }
        } else {
            paths.push(input.clone());
        }
    }
    if let Some(list) = &args.list {
        let unreadable = |error: io::Error| {
            problem(
                "PG0001",
                "list",
                format!("unable to read {}: {error}", list.display()),
            )
        };
        let reader: Box<dyn BufRead> = if list == Path::new("-") {
            Box::new(io::stdin().lock())
        } else {
            Box::new(BufReader::new(File::open(list).map_err(unreadable)?))
        };
        for line in reader.lines() {
            let line = line.map_err(unreadable)?;
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if !line.is_empty() {
                paths.push(PathBuf::from(line));
            }
        }
    }
    if paths.is_empty() {
        return Err(problem("PG0002", "inputs", "no .json or .env cases found"));
    }
    paths.sort();
    // Result directories must stay distinct on case-insensitive file systems too.
    let mut stems: HashMap<String, &Path> = HashMap::new();
    let mut cases = Vec::with_capacity(paths.len());
    for path in &paths {
        let stem = output::directory::stem(path)
            .map_err(|error| problem("PG0002", "inputs", format!("{}: {error}", path.display())))?;
        if let Some(other) = stems.insert(stem.to_lowercase(), path) {
            return Err(problem(
                "PG0002",
                "inputs",
                format!(
                    "{} and {} would share the result directory {stem}",
                    other.display(),
                    path.display()
                ),
            ));
        }
        cases.push(Case {
            path: path.clone(),
            stem,
        });
    }
    Ok(cases)
}

fn solve(
    case: &Case,
    args: &Args,
    previous: &HashMap<String, Vec<Record>>,
    threads: NonZeroUsize,
    model: &dyn Model,
) -> Finished {
    let start = Instant::now();
    let output = args.output.join(&case.stem);
    let mut record = Record {
        status_version: STATUS_VERSION,
        case: case.path.to_string_lossy().into_owned(),
        stem: case.stem.clone(),
        version: env!("CARGO_PKG_VERSION").into(),
        format: args.format.name().into(),
        options: model.options(),
        input_sha256: None,
        outcome: "skipped".into(),
        exit_code: 0,
        seconds: 0.0,
        manifest_sha256: None,
        diagnostics: Vec::new(),
    };
    let (outcome, warnings) = output::diagnostic::capture(|| {
        ::bellhop::parallelism::with_threads(threads, || {
            ::kraken::parallelism::with_threads(threads, || {
                let loaded = match model.load(&case.path) {
                    Ok(loaded) => loaded,
                    Err(outcome) => return Some(outcome),
                };
                record.input_sha256 = Some(input_sha256(&loaded.inputs()));
                record.manifest_sha256 = current(previous, &record, &output);
                // Results that are still current are skipped.
                record
                    .manifest_sha256
                    .is_none()
                    .then(|| loaded.run(&output))
            })
        })
    });
    let (mut diagnostics, summary) = match outcome {
        None => (
            Vec::new(),
            Some(format!("skipped {}: results are current", output.display())),
        ),
        Some(outcome) => {
            record.exit_code = outcome.code;
            if outcome.code == 0 {
                record.outcome = "ok".into();
                record.manifest_sha256 = output::directory::manifest_sha256(&output).ok().flatten();
            } else {
                record.outcome = "failed".into();
            }
            (outcome.diagnostics, outcome.summary)
        }
    };
    diagnostics.extend(warnings.iter().map(Rendered::of));
    let (json, text) = diagnostics.into_iter().map(|d| (d.json, d.text)).unzip();
    record.diagnostics = json;
    record.seconds = start.elapsed().as_secs_f64();
    Finished {
        record,
        text,
        summary,
    }
}

/// SHA-256 over each input's length, as 8 little-endian bytes, and bytes.
fn input_sha256(inputs: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for input in inputs {
        hash.update((input.len() as u64).to_le_bytes());
        hash.update(input);
    }
    format!("{:x}", hash.finalize())
}

/// The manifest hash, if an earlier `ok` or `skipped` record still describes
/// this case's results: everything they depend on matches, and every artifact
/// is unmodified.
fn current(
    previous: &HashMap<String, Vec<Record>>,
    record: &Record,
    output: &Path,
) -> Option<String> {
    let manifests: Vec<&str> = previous
        .get(&record.stem)?
        .iter()
        .filter(|earlier| {
            earlier.case == record.case
                && earlier.version == record.version
                && earlier.format == record.format
                && earlier.options == record.options
                && earlier.input_sha256 == record.input_sha256
        })
        .filter_map(|earlier| earlier.manifest_sha256.as_deref())
        .collect();
    if manifests.is_empty() {
        return None;
    }
    let manifest = output::directory::verified_manifest_sha256(output)
        .ok()
        .flatten()?;
    manifests.contains(&manifest.as_str()).then_some(manifest)
}

/// `ok` and `skipped` records from every status file in the output directory.
/// Unreadable lines, such as one cut short by a killed job, are ignored.
fn previous(output: &Path) -> io::Result<HashMap<String, Vec<Record>>> {
    let mut previous: HashMap<String, Vec<Record>> = HashMap::new();
    let entries = match fs::read_dir(output) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(previous),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let path = entry?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !(name == "status.ndjson" || name.starts_with("status-") && name.ends_with(".ndjson")) {
            continue;
        }
        for line in BufReader::new(File::open(&path)?).split(b'\n') {
            if let Ok(record) = serde_json::from_slice::<Record>(&line?)
                && record.status_version == STATUS_VERSION
                && (record.outcome == "ok" || record.outcome == "skipped")
            {
                previous
                    .entry(record.stem.clone())
                    .or_default()
                    .push(record);
            }
        }
    }
    Ok(previous)
}

/// Open the status file for appending. A new one is created exclusively. An
/// existing one must already be ours: a regular file, not a symbolic link,
/// with no other hard links, that is no case's input, and whose content starts
/// as a status record does. A line a killed job left unfinished is then ended.
fn open_status(path: &Path, cases: &[Case]) -> Result<File, String> {
    let failed = |error: io::Error| format!("unable to open {}: {error}", path.display());
    let refuse = |why: &str| format!("refusing to append to {}: {why}", path.display());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(failed)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).append(true);
    match options.clone().create_new(true).open(path) {
        Ok(file) => return Ok(file),
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(failed(error)),
        Err(_) => {}
    }
    let link = fs::symlink_metadata(path).map_err(failed)?;
    if link.file_type().is_symlink() || !link.is_file() {
        return Err(refuse("it is a symbolic link or not a regular file"));
    }
    let mut file = options.open(path).map_err(failed)?;
    if links(&file).map_err(failed)? != 1 {
        return Err(refuse("it has other hard links"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = file.metadata().map_err(failed)?;
        if (opened.dev(), opened.ino()) != (link.dev(), link.ino()) {
            return Err(refuse("it was replaced while being opened"));
        }
    }
    let handle = same_file::Handle::from_file(file.try_clone().map_err(failed)?).map_err(failed)?;
    if let Some(case) = cases
        .iter()
        .find(|case| same_file::Handle::from_path(&case.path).is_ok_and(|other| other == handle))
    {
        return Err(refuse(&format!("it is the input {}", case.path.display())));
    }
    let mut head = Vec::new();
    (&file)
        .take(2 * STATUS_PREFIX.len() as u64 + 2)
        .read_to_end(&mut head)
        .map_err(failed)?;
    if !is_status(&head) {
        return Err(refuse("it is not a run-many status file"));
    }
    if file.metadata().map_err(failed)?.len() > 0 {
        let mut last = [0];
        file.seek(SeekFrom::End(-1))
            .and_then(|_| file.read_exact(&mut last))
            .map_err(failed)?;
        if last[0] != b'\n' {
            file.write_all(b"\n").map_err(failed)?;
        }
    }
    Ok(file)
}

/// Whether a file starting with `head` is a status file: empty, or its first
/// line is a status record, or the start of one that a killed job cut short,
/// which a later run ended, followed by the start of another.
fn is_status(head: &[u8]) -> bool {
    let record = |line: &[u8]| line.starts_with(STATUS_PREFIX) || STATUS_PREFIX.starts_with(line);
    match head.iter().position(|&byte| byte == b'\n') {
        None => record(head),
        Some(0) => false,
        Some(end) => {
            let (first, rest) = (&head[..end], &head[end + 1..]);
            first.starts_with(STATUS_PREFIX) || STATUS_PREFIX.starts_with(first) && record(rest)
        }
    }
}

/// How many names the opened file has.
#[cfg(unix)]
fn links(file: &File) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(file.metadata()?.nlink())
}

/// How many names the opened file has.
#[cfg(windows)]
fn links(file: &File) -> io::Result<u64> {
    Ok(winapi_util::file::information(file)?.number_of_links())
}

struct Report {
    file: File,
    status: PathBuf,
    diagnostics: Diagnostics,
    /// ok, failed, skipped
    tally: [usize; 3],
    code: u8,
}

impl Report {
    /// Write the case's status line, then print what `run` would have printed.
    /// Returns false once the status file cannot be written.
    fn record(&mut self, case: &Case, finished: &Finished) -> bool {
        let record = &finished.record;
        let mut line = serde_json::to_vec(record).unwrap_or_default();
        line.push(b'\n');
        let written = self.file.write_all(&line).and_then(|()| self.file.flush());
        if let Some(summary) = &finished.summary {
            println!("{summary}");
        }
        match self.diagnostics {
            Diagnostics::Text => {
                // Every line starts with the case path, once.
                let path = case.path.display().to_string();
                for line in &finished.text {
                    if line.starts_with(&path) {
                        eprintln!("{line}");
                    } else {
                        eprintln!("{path}: {line}");
                    }
                }
            }
            Diagnostics::Json if !record.diagnostics.is_empty() => eprintln!(
                "{}",
                serde_json::json!({"case": record.case, "diagnostics": record.diagnostics})
            ),
            Diagnostics::Json => {}
        }
        match record.outcome.as_str() {
            "ok" => self.tally[0] += 1,
            "failed" => self.tally[1] += 1,
            _ => self.tally[2] += 1,
        }
        self.code = self.code.max(record.exit_code);
        if let Err(error) = written {
            let message = format!("unable to write {}: {error}", self.status.display());
            self.diagnostics
                .render(&[problem("PG0404", "output", message)]);
            self.code = self.code.max(4);
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{Record, STATUS_PREFIX, is_status};

    #[test]
    fn every_record_starts_with_the_status_prefix() {
        let record = Record {
            status_version: super::STATUS_VERSION,
            case: "a.json".into(),
            stem: "a".into(),
            version: "0.0.0".into(),
            format: "legacy".into(),
            options: serde_json::json!({}),
            input_sha256: None,
            outcome: "failed".into(),
            exit_code: 2,
            seconds: 0.0,
            manifest_sha256: None,
            diagnostics: Vec::new(),
        };
        assert!(
            serde_json::to_vec(&record)
                .unwrap()
                .starts_with(STATUS_PREFIX)
        );
    }

    #[test]
    fn status_files_are_recognized_even_with_a_first_record_cut_short() {
        let record = br#"{"status_version":1,"case":"a.json","stem":"a"}"#;
        for (head, expected) in [
            (&b""[..], true),
            (record, true),
            (br#"{"sta"#, true),
            (br#"{"status_version":1,"ca"#, true),
            (b"{\"sta\n", true),
            (b"{\"sta\n{\"status_version\":1,\"case\"", true),
            (b"{\"sta\n{\"stat", true),
            (b"my notes\n", false),
            (b"\n{\"status_version\":1", false),
            (b"{\n  \"pelagic\": 2\n}\n", false),
            (br#"{"pelagic":2,"title":"x"}"#, false),
        ] {
            assert_eq!(
                is_status(head),
                expected,
                "{}",
                String::from_utf8_lossy(head)
            );
        }
    }
}

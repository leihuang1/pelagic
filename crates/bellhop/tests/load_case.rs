use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use bellhop::legacy::load_case;

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("bellhop-rs-test-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn collects_independent_missing_auxiliary_files() {
    let directory = TemporaryDirectory::new();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Gulf_ray_rd.env");
    let environment = directory.join("case.env");
    fs::copy(fixture, &environment).unwrap();

    let report = load_case(&environment).unwrap_err();
    assert_eq!(report.diagnostics().len(), 2);
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.location.path.extension().unwrap() == "ssp")
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.location.path.extension().unwrap() == "bty")
    );
}

#[test]
fn loads_precalculated_internal_reflection_table() {
    let directory = TemporaryDirectory::new();
    let environment = directory.join("internal.env");
    fs::write(
        &environment,
        "'Internal reflection table'\n\
         100.0\n\
         1\n\
         'CVW'\n\
         0 0.0 100.0\n\
         0.0 1500.0 /\n\
         100.0 1500.0 /\n\
         'P' 0.0\n\
         1\n\
         50.0 /\n\
         1\n\
         50.0 /\n\
         2\n\
         0.0 1.0 /\n\
         'R'\n\
         1\n\
         10.0 /\n\
         10.0 101.0 1.0\n",
    )
    .unwrap();
    fs::write(
        directory.join("internal.irc"),
        "'generated table' 100\n2\n0.0 1.0 0.0 1.0 0.0 0\n1.0 2.0 0.0 1.0 0.0 0\n",
    )
    .unwrap();

    let case = load_case(&environment).unwrap().value;
    let table = case.internal_reflection.as_ref().unwrap();
    assert_eq!(table.title, "generated table");
    assert_eq!(table.points.len(), 2);
}

#[test]
fn resolves_top_and_bottom_reflection_tables() {
    let directory = TemporaryDirectory::new();
    let environment = directory.join("reflection.env");
    fs::write(
        &environment,
        "'Reflection tables'\n\
         100.0\n\
         1\n\
         'CFW'\n\
         0 0.0 100.0\n\
         0.0 1500.0 /\n\
         100.0 1500.0 /\n\
         'F' 0.0\n\
         1\n\
         50.0 /\n\
         1\n\
         50.0 /\n\
         2\n\
         0.0 1.0 /\n\
         'C'\n\
         2\n\
         -10.0 10.0 /\n\
         0.0 101.0 1.0\n",
    )
    .unwrap();
    let table = "2\n0.0 1.0 0.0\n90.0 0.5 180.0\n";
    fs::write(directory.join("reflection.brc"), table).unwrap();
    fs::write(directory.join("reflection.trc"), table).unwrap();

    let expected_source = fs::read(&environment).unwrap();
    let input = bellhop::legacy::load_case_with_inputs(&environment).unwrap();
    assert_eq!(input.auxiliary_sources(), [table, table]);
    let (outcome, source, paths) = input.into_parts();
    assert_eq!(source.as_bytes(), expected_source);
    assert_eq!(
        paths,
        [
            environment.clone(),
            environment.with_extension("brc"),
            environment.with_extension("trc")
        ]
    );
    let case = load_case(&environment).unwrap().value;
    assert_eq!(outcome.value, case);
    fs::write(&environment, b"changed after parsing").unwrap();
    assert_eq!(source.as_bytes(), expected_source);
    assert_eq!(case.bottom_reflection.as_ref().unwrap().points.len(), 2);
    assert_eq!(case.top_reflection.as_ref().unwrap().points.len(), 2);
}

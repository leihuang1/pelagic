use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use bellhop::input;

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cases/bellhop/field-g/field-g.json")
}

struct Temporary(PathBuf);

impl Temporary {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("bellhop-intake-{name}-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Failing;

impl Read for Failing {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("broken pipe"))
    }
}

#[test]
fn json_file_and_reader_load_the_same_case_with_their_own_consumed_inputs() {
    let path = example();
    let source = fs::read(&path).unwrap();

    let file = input::load(&path).unwrap();
    assert_eq!(file.path(), path);
    assert_eq!(file.source(), source);
    assert_eq!(file.consumed_inputs(), std::slice::from_ref(&path));
    assert_eq!(file.auxiliary_sources(), [] as [String; 0]);

    let reader = input::load_reader(source.as_slice()).unwrap();
    assert_eq!(reader.path(), Path::new("stdin.json"));
    assert_eq!(reader.source(), source);
    assert_eq!(reader.consumed_inputs(), [] as [PathBuf; 0]);
    assert_eq!(reader.auxiliary_sources(), [] as [String; 0]);

    // Cases and warnings record their named path, so compare the rest of each.
    let exported = |input: &input::CaseInput| {
        serde_json::to_value(bellhop::json::export_case_document(&input.outcome().value).unwrap())
            .unwrap()
    };
    assert_eq!(exported(&file), exported(&reader));
    let warnings = |input: &input::CaseInput| {
        let warnings = &input.outcome().warnings;
        let paths: Vec<_> = warnings.iter().map(|w| w.location.path.clone()).collect();
        (
            warnings
                .iter()
                .map(|w| (w.code, w.message.clone()))
                .collect::<Vec<_>>(),
            paths,
        )
    };
    let (file_warnings, file_paths) = warnings(&file);
    let (reader_warnings, reader_paths) = warnings(&reader);
    assert_eq!(file_warnings, reader_warnings);
    assert!(file_paths.iter().all(|warned| *warned == path));
    assert!(
        reader_paths
            .iter()
            .all(|warned| warned == Path::new("stdin.json"))
    );
}

#[test]
fn legacy_files_consume_their_auxiliary_inputs_in_reading_order() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Gulf_ray_rd.env");
    let input = input::load(&path).unwrap();
    assert_eq!(input.path(), path);
    assert_eq!(input.source(), fs::read(&path).unwrap());
    let expected = [
        path.clone(),
        path.with_extension("ssp"),
        path.with_extension("bty"),
    ];
    assert_eq!(input.consumed_inputs(), expected);
    assert_eq!(
        input.auxiliary_sources(),
        [
            fs::read_to_string(&expected[1]).unwrap(),
            fs::read_to_string(&expected[2]).unwrap()
        ]
    );
    assert_eq!(
        input.outcome().value,
        bellhop::legacy::load_case(&path).unwrap().value
    );
}

#[test]
fn only_a_json_extension_selects_json() {
    let root = Temporary::new("routing");
    let other = root.0.join("case.txt");
    fs::copy(example(), &other).unwrap();
    let report = input::load(&other).unwrap_err();
    let d = &report.diagnostics()[0];
    assert_eq!((d.code, d.field.as_deref()), ("BH0002", Some("input")));
}

#[test]
fn failures_are_located_at_the_named_path() {
    let root = Temporary::new("failures");
    let missing = root.0.join("missing.json");
    let report = input::load(&missing).unwrap_err();
    let d = &report.diagnostics()[0];
    assert_eq!(
        (d.code, d.field.as_deref(), d.location.path.as_path()),
        ("BH0001", Some("input"), missing.as_path())
    );

    let report = input::load_reader(Failing).unwrap_err();
    let d = &report.diagnostics()[0];
    assert_eq!(
        (d.code, d.field.as_deref(), d.location.path.as_path()),
        ("BH0001", Some("input"), Path::new("stdin.json"))
    );
    assert_eq!(d.message, "unable to read input: broken pipe");

    let malformed = root.0.join("malformed.json");
    fs::write(&malformed, b"{").unwrap();
    for (report, path) in [
        (input::load(&malformed).unwrap_err(), malformed.as_path()),
        (
            input::load_reader(&b"{"[..]).unwrap_err(),
            Path::new("stdin.json"),
        ),
    ] {
        let d = &report.diagnostics()[0];
        assert_eq!((d.code, d.location.path.as_path()), ("BH0103", path));
    }
}

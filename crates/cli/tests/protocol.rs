use std::io::Write;
use std::process::{Command, Stdio};

fn stdin_command(model: &str, operation: &str, bytes: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pelagic"))
        .args(["--diagnostics", "json", model, operation, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn version_and_stdin_use_the_public_machine_contract() {
    let output = Command::new(env!("CARGO_BIN_EXE_pelagic"))
        .args(["version", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let version: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(version["case_schema_versions"]["bellhop"], 2);
    assert_eq!(version["case_schema_versions"]["kraken"], 2);
    for (model, bytes) in [
        (
            "bellhop",
            include_bytes!("../../../examples/cases/bellhop/field-g/field-g.json").as_slice(),
        ),
        (
            "kraken",
            include_bytes!("../../../examples/cases/kraken/pekeris/pekeris.json").as_slice(),
        ),
    ] {
        let output = stdin_command(model, "export", bytes);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["pelagic"],
            2
        );
        let output = stdin_command(model, "validate", b"{\"model\":\"x\"}");
        assert_eq!(output.status.code(), Some(2));
        let diagnostic: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(diagnostic["diagnostics"][0]["severity"], "error");
        assert!(
            diagnostic["diagnostics"][0]["code"]
                .as_str()
                .unwrap()
                .ends_with("0103")
        );
        assert_eq!(diagnostic["diagnostics"][0]["source"]["path"], "stdin.json");
    }
}

#[test]
fn kraken_validate_names_the_validated_input() {
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/cases/kraken/pekeris/pekeris.json");
    let output = Command::new(env!("CARGO_BIN_EXE_pelagic"))
        .args(["kraken", "validate"])
        .arg(&example)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("valid 2D KRAKEN case: "), "{stdout}");
    assert!(stdout.contains("pekeris.json"), "{stdout}");
    let output = stdin_command(
        "kraken",
        "validate",
        include_bytes!("../../../examples/cases/kraken/pekeris/pekeris.json"),
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("stdin.json"));
}

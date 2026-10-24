use std::path::Path;
use std::process::Command;

use bellhop::json::CaseDocument;

#[test]
fn exports_a_legacy_case_as_self_contained_json() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../bellhop/tests/fixtures/golden/Field_G.env");
    let output = Command::new(env!("CARGO_BIN_EXE_pelagic"))
        .args(["bellhop", "export"])
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: CaseDocument = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document.pelagic, bellhop::json::SCHEMA_VERSION);
    assert_eq!(document.title, "BELLHOP field influence golden");
    // Default density is omitted, so rows hold only depth and sound speed.
    assert_eq!(document.media[0].ssp[0].len(), 2);
    assert!(document.media[0].range_dependent.is_none());
}

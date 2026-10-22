use super::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn root(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("pelagic-publication-{name}-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    root
}
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(root)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().into_string().unwrap(),
                fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}
fn write(root: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    publish(root, &[], None, |scratch| {
        fs::write(scratch.join(name), bytes).map_err(|e| e.to_string())
    })
}

#[test]
fn ownership_failures_and_writer_failures_preserve_entire_groups() {
    let parent = root("groups");
    let out = parent.join("results");
    write(&out, "a.txt", b"old result").unwrap();
    fs::write(out.join("notes"), b"unrelated").unwrap();
    let old = snapshot(&out);
    let error = publish::<(), String>(&out, &[], None, |scratch| {
        fs::write(scratch.join("b.txt"), b"partial").unwrap();
        Err("late solve/write failure".into())
    })
    .unwrap_err();
    assert!(error.contains("late solve"));
    assert_eq!(snapshot(&out), old);
    assert!(
        publish::<(), String>(&out, &[], Some(1), |scratch| fs::write(
            scratch.join("a.txt"),
            b"new result"
        )
        .map_err(|e| e.to_string()))
        .is_err()
    );
    assert_eq!(snapshot(&out), old);
    assert!(
        write(&out, "notes", b"bad")
            .unwrap_err()
            .contains("unowned")
    );
    assert_eq!(snapshot(&out), old);
    write(&out, "b.txt", b"new result").unwrap();
    assert!(!out.join("a.txt").exists());
    assert_eq!(fs::read(out.join("notes")).unwrap(), b"unrelated");
    fs::write(out.join("b.txt"), b"modified").unwrap();
    let modified = snapshot(&out);
    assert!(
        write(&out, "b.txt", b"bad")
            .unwrap_err()
            .contains("modified")
    );
    assert_eq!(snapshot(&out), modified);
    for name in [".pelagic-stage", ".pelagic-backup", ".pelagic-lock"] {
        let path = out.join(name);
        fs::write(&path, b"not ours").unwrap();
        assert!(write(&out, "c.txt", b"new").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not ours");
        fs::remove_file(path).unwrap();
    }
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn installation_and_rollback_failures_keep_recoverable_data() {
    let parent = root("rollback");
    let out = parent.join("results");
    write(&out, "a.txt", b"old result").unwrap();
    let old = snapshot(&out);
    {
        let mut transaction = Transaction::begin(&out, &[]).unwrap();
        fs::write(transaction.stage.join("a.txt"), b"new result").unwrap();
        transaction.before_install = Some(|t| {
            fs::remove_file(t.stage.join("a.txt")).unwrap();
        });
        assert!(
            transaction
                .install(&[], None)
                .unwrap_err()
                .contains("previous results restored")
        );
    }
    assert_eq!(snapshot(&out), old);
    {
        let mut transaction = Transaction::begin(&out, &[]).unwrap();
        fs::write(transaction.stage.join("a.txt"), b"new result").unwrap();
        transaction.before_install = Some(|t| {
            fs::write(t.output.join("a.txt"), b"late unrelated destination").unwrap();
        });
        let error = transaction.install(&[], None).unwrap_err();
        assert!(error.contains("rollback failed") && error.contains("recover old results"));
    }
    assert_eq!(
        fs::read(out.join("a.txt")).unwrap(),
        b"late unrelated destination"
    );
    assert_eq!(
        fs::read(out.join(".pelagic-backup/a.txt")).unwrap(),
        b"old result"
    );
    assert_eq!(
        fs::read(out.join(".pelagic-stage/a.txt")).unwrap(),
        b"new result"
    );
    assert!(out.join(".pelagic-lock").is_dir());
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn ownership_never_authorizes_replacing_input_aliases_or_symlinks() {
    let parent = root("aliases");
    let input = parent.join("source");
    fs::write(&input, b"input").unwrap();
    let out = parent.join("results");
    write(&out, "a.txt", b"input").unwrap();
    fs::remove_file(out.join("a.txt")).unwrap();
    fs::hard_link(&input, out.join("a.txt")).unwrap();
    let old = snapshot(&out);
    let error = publish::<(), String>(&out, &[&input], None, |scratch| {
        fs::write(scratch.join("a.txt"), b"bad").map_err(|e| e.to_string())
    })
    .unwrap_err();
    assert!(error.contains("input file"));
    assert_eq!(snapshot(&out), old);
    assert_eq!(fs::read(&input).unwrap(), b"input");
    #[cfg(unix)]
    {
        fs::remove_file(out.join("a.txt")).unwrap();
        std::os::unix::fs::symlink(&input, out.join("a.txt")).unwrap();
        assert!(
            write(&out, "a.txt", b"bad")
                .unwrap_err()
                .contains("symlink")
        );
        assert_eq!(fs::read(&input).unwrap(), b"input");
    }
    let controls = parent.join("controls");
    fs::create_dir(&controls).unwrap();
    for name in [
        MANIFEST,
        ".pelagic-lock",
        ".pelagic-stage",
        ".pelagic-backup",
    ] {
        let alias = controls.join(name);
        fs::hard_link(&input, &alias).unwrap();
        assert!(
            publish::<(), String>(&controls, &[&input], None, |_| Ok(()))
                .unwrap_err()
                .contains("input file")
        );
        assert_eq!(fs::read(&alias).unwrap(), b"input");
        fs::remove_file(alias).unwrap();
    }
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn malformed_and_traversing_manifests_never_claim_unrelated_files() {
    let parent = root("manifest");
    let out = parent.join("results");
    fs::create_dir(&out).unwrap();
    fs::write(parent.join("input"), b"input").unwrap();
    for name in [
        "../input",
        "/input",
        ".",
        ".pelagic-stage",
        "pelagic-manifest.json",
    ] {
        let value = serde_json::json!({"schema_version":1,"implementation":"Pelagic","artifacts":[{"name":name,"size_bytes":5,"sha256":"0".repeat(64)}]});
        fs::write(
            out.join("pelagic-manifest.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let old = snapshot(&out);
        assert!(write(&out, "a.txt", b"new").is_err());
        assert_eq!(snapshot(&out), old);
        assert_eq!(fs::read(parent.join("input")).unwrap(), b"input");
    }
    fs::remove_dir_all(parent).unwrap();
}

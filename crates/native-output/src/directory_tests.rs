use super::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "directory/publication_tests.rs"]
mod publication;

pub(super) fn root(name: &str) -> PathBuf {
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
    publish(root, &[], None, SyncPolicy::default(), |scratch| {
        fs::write(scratch.join(name), bytes).map_err(|e| e.to_string())
    })
}

#[test]
fn running_quota_matches_scans_at_each_check_without_flushing_writers() {
    let parent = root("running-quota");
    fs::write(parent.join("initial"), b"123").unwrap();
    let mut quota = Quota::new(&parent, 10).unwrap();
    let path = parent.join("growing");
    let mut writer = crate::create(&path).unwrap();
    for bytes in [b"123".as_slice(), b"4567", b"8"] {
        writer.write_all(bytes).unwrap();
        quota.refresh(&path).unwrap();
        assert_eq!(quota.total, check_quota(&parent, None).unwrap());
        writer.flush().unwrap();
        quota.refresh(&path).unwrap();
        assert_eq!(quota.total, check_quota(&parent, None).unwrap());
        assert_eq!(
            quota.remaining(),
            check_quota(&parent, Some(10)).map(|n| 10 - n)
        );
    }
    drop(writer);
    // Creation and refresh do not enforce a quota ahead of the original check points.
    let mut quota = Quota::new(&parent, 1).unwrap();
    assert_eq!(
        quota.remaining().unwrap_err(),
        "result directory exceeds 1 bytes"
    );
    fs::write(&path, b"").unwrap();
    quota.refresh(&path).unwrap();
    assert_eq!(quota.total, 3);
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn ownership_failures_and_writer_failures_preserve_entire_groups() {
    let parent = root("groups");
    let out = parent.join("results");
    write(&out, "a.txt", b"old result").unwrap();
    fs::write(out.join("notes"), b"unrelated").unwrap();
    let old = snapshot(&out);
    let error = publish::<(), String>(&out, &[], None, SyncPolicy::default(), |scratch| {
        fs::write(scratch.join("b.txt"), b"partial").unwrap();
        Err("late solve/write failure".into())
    })
    .unwrap_err();
    assert!(error.contains("late solve"));
    assert_eq!(snapshot(&out), old);
    assert!(
        publish::<(), String>(&out, &[], Some(1), SyncPolicy::default(), |scratch| {
            fs::write(scratch.join("a.txt"), b"new result").map_err(|e| e.to_string())
        })
        .unwrap_err()
        .contains("result directory exceeds 1 bytes")
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
fn flush_failure_precedes_every_previous_product_move() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SYNCED: AtomicUsize = AtomicUsize::new(0);
    let parent = root("flush");
    let out = parent.join("results");
    write(&out, "old.txt", b"old").unwrap();
    let old = snapshot(&out);
    {
        let mut transaction = Transaction::begin(&out, &[]).unwrap();
        fs::write(transaction.stage.join("a.txt"), b"a").unwrap();
        fs::write(transaction.stage.join("b.txt"), b"bb").unwrap();
        transaction.sync = |file| {
            SYNCED.fetch_add(1, Ordering::SeqCst);
            match file.metadata()?.len() {
                1 => Err(std::io::Error::other("first file failure")),
                2 => Err(std::io::Error::other("second file failure")),
                _ => sync_file(file),
            }
        };
        assert_eq!(
            transaction
                .install(None, SyncPolicy::default())
                .unwrap_err(),
            "first file failure"
        );
        assert_eq!(
            SYNCED.load(Ordering::SeqCst),
            3,
            "manifest and all artifacts attempted"
        );
        assert!(!transaction.backup.exists());
        assert_eq!(fs::read(out.join("old.txt")).unwrap(), b"old");
    }
    assert_eq!(snapshot(&out), old);
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
                .install(None, SyncPolicy::default())
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
        let error = transaction
            .install(None, SyncPolicy::default())
            .unwrap_err();
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

fn hard_link_install(source: &Path, target: &Path) -> std::io::Result<bool> {
    fs::hard_link(source, target).map(|()| false)
}

#[cfg(any(target_vendor = "apple", target_os = "linux"))]
#[test]
fn exclusive_rename_consumes_stage_files_and_fallback_keeps_them() {
    use rustix::io::Errno;
    let parent = root("rename-fallback");
    let source = parent.join("source");
    let target = parent.join("target");
    fs::write(&source, b"new").unwrap();
    assert!(install_file(&source, &target).unwrap());
    assert!(!source.exists());
    assert_eq!(fs::read(&target).unwrap(), b"new");
    fs::remove_file(&target).unwrap();
    for error in [
        Errno::INVAL,
        Errno::NOSYS,
        Errno::NOTSUP,
        Errno::OPNOTSUPP,
        Errno::XDEV,
    ] {
        fs::write(&source, b"new").unwrap();
        assert!(!install_file_with(&source, &target, |_, _| Err(error)).unwrap());
        assert!(same_file::is_same_file(&source, &target).unwrap());
        fs::remove_file(&target).unwrap();
        fs::write(&target, b"late").unwrap();
        assert!(install_file_with(&source, &target, |_, _| Err(error)).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"late");
        assert_eq!(fs::read(&source).unwrap(), b"new");
        fs::remove_file(&target).unwrap();
    }
    for error in [Errno::EXIST, Errno::ACCESS, Errno::IO, Errno::NOENT] {
        let actual = install_file_with(&source, &target, |_, _| Err(error)).unwrap_err();
        assert_eq!(actual.raw_os_error(), Some(error.raw_os_error()));
        assert!(!target.exists());
    }
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn rename_and_hard_link_rollback_restore_the_stage_and_previous_group() {
    for install in [install_file, hard_link_install] {
        let parent = root("rollback-paths");
        let out = parent.join("results");
        write(&out, "a.txt", b"old").unwrap();
        let old = snapshot(&out);
        {
            let mut transaction = Transaction::begin(&out, &[]).unwrap();
            transaction.install_file = install;
            for name in ["a.txt", "b.txt", "c.txt"] {
                fs::write(transaction.stage.join(name), b"new").unwrap();
            }
            transaction.before_install = Some(|t| {
                fs::write(t.output.join("c.txt"), b"late").unwrap();
            });
            assert!(
                transaction
                    .install(None, SyncPolicy::default())
                    .unwrap_err()
                    .contains("previous results restored")
            );
            for name in ["a.txt", "b.txt", "c.txt"] {
                assert_eq!(fs::read(transaction.stage.join(name)).unwrap(), b"new");
            }
        }
        assert_eq!(fs::read(out.join("c.txt")).unwrap(), b"late");
        fs::remove_file(out.join("c.txt")).unwrap();
        assert_eq!(snapshot(&out), old);
        fs::remove_dir_all(parent).unwrap();
    }
}

#[test]
fn rollback_reads_installed_files_before_removing_them() {
    for install in [install_file, hard_link_install] {
        let parent = root("rollback-modified");
        let out = parent.join("results");
        write(&out, "a.txt", b"old").unwrap();
        {
            let mut transaction = Transaction::begin(&out, &[]).unwrap();
            transaction.install_file = install;
            fs::write(transaction.stage.join("a.txt"), b"new").unwrap();
            fs::write(transaction.stage.join("b.txt"), b"new").unwrap();
            transaction.after_install = Some(|t, name| {
                if name == "a.txt" {
                    fs::write(t.output.join(name), b"edited after install").unwrap();
                    fs::write(t.output.join("b.txt"), b"late").unwrap();
                }
            });
            let error = transaction
                .install(None, SyncPolicy::default())
                .unwrap_err();
            assert!(
                error.contains("rollback failed") && error.contains("owned artifact was modified")
            );
        }
        assert_eq!(
            fs::read(out.join("a.txt")).unwrap(),
            b"edited after install"
        );
        assert_eq!(fs::read(out.join(".pelagic-backup/a.txt")).unwrap(), b"old");
        assert!(out.join(".pelagic-lock").is_dir());
        fs::remove_dir_all(parent).unwrap();
    }
}

#[cfg(any(target_vendor = "apple", target_os = "linux"))]
#[test]
fn rollback_never_replaces_a_late_stage_destination() {
    let parent = root("rollback-stage-conflict");
    let out = parent.join("results");
    write(&out, "a.txt", b"old").unwrap();
    {
        let mut transaction = Transaction::begin(&out, &[]).unwrap();
        fs::write(transaction.stage.join("a.txt"), b"new").unwrap();
        fs::write(transaction.stage.join("b.txt"), b"new").unwrap();
        transaction.after_install = Some(|t, name| {
            if name == "a.txt" {
                assert!(!t.stage.join(name).exists());
                fs::write(t.stage.join(name), b"late stage destination").unwrap();
                fs::write(t.output.join("b.txt"), b"late output destination").unwrap();
            }
        });
        assert!(
            transaction
                .install(None, SyncPolicy::default())
                .unwrap_err()
                .contains("rollback failed")
        );
    }
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"new");
    assert_eq!(
        fs::read(out.join(".pelagic-stage/a.txt")).unwrap(),
        b"late stage destination"
    );
    assert_eq!(fs::read(out.join(".pelagic-backup/a.txt")).unwrap(), b"old");
    assert!(out.join(".pelagic-lock").is_dir());
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn destinations_appearing_during_a_run_are_refused_and_kept() {
    let parent = root("late");
    let out = parent.join("results");
    // While the writer runs in an existing directory: seen by the install checks.
    fs::create_dir(&out).unwrap();
    let error = publish::<(), String>(&out, &[], None, SyncPolicy::default(), |scratch| {
        fs::write(out.join("a.txt"), b"arrived during writing").unwrap();
        fs::write(scratch.join("a.txt"), b"new result").map_err(|e| e.to_string())
    })
    .unwrap_err();
    assert!(error.contains("unowned artifact conflict"));
    assert_eq!(
        snapshot(&out),
        BTreeMap::from([("a.txt".to_owned(), b"arrived during writing".to_vec())])
    );
    fs::remove_file(out.join("a.txt")).unwrap();
    write(&out, "a.txt", b"old result").unwrap();
    let old = snapshot(&out);
    // After those checks: only exclusive installation can refuse it.
    {
        let mut transaction = Transaction::begin(&out, &[]).unwrap();
        fs::write(transaction.stage.join("a.txt"), b"new result").unwrap();
        fs::write(transaction.stage.join("b.txt"), b"new result").unwrap();
        transaction.before_install = Some(|t| {
            fs::write(t.output.join("b.txt"), b"arrived before install").unwrap();
        });
        let error = transaction
            .install(None, SyncPolicy::default())
            .unwrap_err();
        assert!(error.contains("unable to install b.txt"));
        assert!(error.contains("previous results restored"));
    }
    assert_eq!(
        fs::read(out.join("b.txt")).unwrap(),
        b"arrived before install"
    );
    fs::remove_file(out.join("b.txt")).unwrap();
    assert_eq!(snapshot(&out), old);
    write(&out, "b.txt", b"retry").unwrap();
    assert_eq!(fs::read(out.join("b.txt")).unwrap(), b"retry");
    assert!(!out.join("a.txt").exists());
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
    let error = publish::<(), String>(&out, &[&input], None, SyncPolicy::default(), |scratch| {
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
            publish::<(), String>(
                &controls,
                &[&input],
                None,
                SyncPolicy::default(),
                |_| Ok(())
            )
            .unwrap_err()
            .contains("input file")
        );
        assert_eq!(fs::read(&alias).unwrap(), b"input");
        fs::remove_file(alias).unwrap();
    }
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn cached_inputs_still_refuse_late_hard_link_aliases() {
    let parent = root("late-alias");
    let input = parent.join("input");
    fs::write(&input, b"input").unwrap();
    let out = parent.join("results");
    fs::create_dir(&out).unwrap();
    {
        let mut transaction = Transaction::begin(&out, &[&input]).unwrap();
        fs::write(transaction.stage.join("a.txt"), b"new").unwrap();
        transaction.before_install = Some(|t| {
            fs::hard_link(&t.inputs[0], t.output.join("a.txt")).unwrap();
        });
        let error = transaction
            .install(None, SyncPolicy::default())
            .unwrap_err();
        assert!(error.contains("output must not replace an input file or its alias"));
        assert!(error.contains("previous results restored"));
    }
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"input");
    assert_eq!(fs::read(&input).unwrap(), b"input");
    fs::remove_dir_all(parent).unwrap();
}

#[cfg(unix)]
#[test]
fn cached_parent_and_inputs_resolve_symlinked_ancestors() {
    let parent = root("parent-alias");
    let real = parent.join("real");
    fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, parent.join("alias")).unwrap();
    let out = parent.join("alias/results");
    fs::create_dir(&out).unwrap();
    fs::write(real.join("results/input"), b"input").unwrap();
    let error = publish::<(), String>(
        &out,
        &[&real.join("results/input")],
        None,
        SyncPolicy::default(),
        |stage| fs::write(stage.join("input"), b"bad").map_err(message),
    )
    .unwrap_err();
    assert!(error.contains("input file or its alias"));
    assert_eq!(fs::read(out.join("input")).unwrap(), b"input");
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

#[test]
fn fingerprints_match_sha256_known_answers_across_read_blocks() {
    let parent = root("sha256");
    let path = parent.join("artifact");
    for (bytes, expected) in [
        (
            Vec::new(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            b"abc".to_vec(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            vec![b'a'; 1_000_000],
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
        ),
    ] {
        fs::write(&path, &bytes).unwrap();
        let actual = fingerprint(&path, "artifact").unwrap();
        assert_eq!(actual.size_bytes, bytes.len() as u64);
        assert_eq!(actual.sha256, expected);
    }
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn manifest_hash_tracks_publication_and_verification_finds_modified_artifacts() {
    let parent = root("manifest-hash");
    let out = parent.join("results");
    assert_eq!(manifest_sha256(&out).unwrap(), None);
    assert_eq!(verified_manifest_sha256(&out).unwrap(), None);
    write(&out, "a.txt", b"first").unwrap();
    let first = manifest_sha256(&out).unwrap().unwrap();
    assert_eq!(first.len(), 64);
    assert_eq!(verified_manifest_sha256(&out).unwrap(), Some(first.clone()));
    write(&out, "a.txt", b"second").unwrap();
    let second = verified_manifest_sha256(&out).unwrap().unwrap();
    assert_ne!(second, first);
    fs::write(out.join("a.txt"), b"edited").unwrap();
    assert_eq!(manifest_sha256(&out).unwrap(), Some(second));
    assert!(
        verified_manifest_sha256(&out)
            .unwrap_err()
            .contains("modified")
    );
    fs::remove_file(out.join("a.txt")).unwrap();
    assert!(verified_manifest_sha256(&out).is_err());
    fs::remove_dir_all(parent).unwrap();
}

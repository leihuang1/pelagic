use super::*;

#[test]
fn no_sync_skips_every_file_sync_and_drive_barrier_for_single_and_batch_publications() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SYNCS: AtomicUsize = AtomicUsize::new(0);
    static FLUSHES: AtomicUsize = AtomicUsize::new(0);
    let root = root("sync-policy");
    assert_eq!(SyncPolicy::default(), SyncPolicy::Sync);
    for workers in [None, Some(1), Some(4), Some(14)] {
        let batch = workers.map(Batch::new);
        let check = || {
            for replace in [false, true] {
                let mut expected = None;
                for policy in [SyncPolicy::default(), SyncPolicy::NoSync] {
                    let out = root.join(format!("{workers:?}-{replace}-{policy:?}"));
                    if replace {
                        write(&out, "stale", b"old").unwrap();
                    }
                    SYNCS.store(0, Ordering::SeqCst);
                    FLUSHES.store(0, Ordering::SeqCst);
                    {
                        let mut t = Transaction::begin(&out, &[]).unwrap();
                        fs::write(t.stage.join("a"), b"a").unwrap();
                        fs::write(t.stage.join("z"), b"zz").unwrap();
                        t.sync = |_| {
                            SYNCS.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        };
                        #[cfg(target_vendor = "apple")]
                        {
                            t.full_sync = |_, _| {
                                assert_eq!(SYNCS.load(Ordering::SeqCst), 3);
                                FLUSHES.fetch_add(1, Ordering::SeqCst);
                                Ok(())
                            };
                            t.wait = |_, _, _| {
                                assert_eq!(SYNCS.load(Ordering::SeqCst), 3);
                                FLUSHES.fetch_add(1, Ordering::SeqCst);
                                Ok(())
                            };
                            assert_eq!(t.batch.is_some(), workers.is_some_and(|w| w > 1));
                        }
                        t.install(None, policy).unwrap();
                    }
                    let syncing = policy == SyncPolicy::Sync;
                    assert_eq!(SYNCS.load(Ordering::SeqCst), if syncing { 3 } else { 0 });
                    assert_eq!(
                        FLUSHES.load(Ordering::SeqCst),
                        usize::from(syncing && cfg!(target_vendor = "apple"))
                    );
                    assert!(verified_manifest_sha256(&out).unwrap().is_some());
                    let files = snapshot(&out);
                    assert_eq!(files.len(), 3, "no stale files or controls remain");
                    if let Some(expected) = &expected {
                        assert_eq!(&files, expected);
                    } else {
                        expected = Some(files);
                    }
                }
            }
        };
        if let Some(batch) = batch {
            batch.run(check);
        } else {
            check();
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn public_publish_observes_the_requested_sync_policy() {
    use sync_observer::{Kind, Registration};
    let root = root("public-sync-policy");
    let observer = Registration::new(&root);
    for workers in [None, Some(1), Some(4), Some(14)] {
        let batch = workers.map(Batch::new);
        for policy in [SyncPolicy::default(), SyncPolicy::NoSync] {
            let out = root.join(format!("{workers:?}-{policy:?}"));
            for _ in 0..2 {
                // Fresh publication and replacement use the same public policy.
                let run = || {
                    publish::<(), String>(&out, &[], None, policy, |stage| {
                        fs::write(stage.join("a"), b"a").map_err(message)?;
                        fs::write(stage.join("b"), b"bb").map_err(message)
                    })
                };
                if let Some(batch) = &batch {
                    batch.run(run)
                } else {
                    run()
                }
                .unwrap();
                let mut actual: Vec<_> = observer
                    .take()
                    .into_iter()
                    .map(|e| {
                        (
                            e.path.file_name().unwrap().to_string_lossy().into_owned(),
                            e.kind,
                        )
                    })
                    .collect();
                actual.sort();
                let mut expected = Vec::new();
                if policy == SyncPolicy::Sync {
                    expected.extend(["a", "b", MANIFEST].map(|name| (name.to_owned(), Kind::File)));
                    if cfg!(target_vendor = "apple") {
                        // Publications here run one at a time, so each barrier flushes once.
                        expected.push((MANIFEST.to_owned(), Kind::DriveFlush));
                        if workers.is_some_and(|w| w > 1) {
                            expected.push((MANIFEST.to_owned(), Kind::SharedBarrier));
                        }
                    }
                }
                expected.sort();
                assert_eq!(actual, expected, "{workers:?} {policy:?}");
                assert!(verified_manifest_sha256(&out).unwrap().is_some());
            }
        }
    }
    drop(observer);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_publication_policies_produce_identical_files_and_manifests() {
    let root = root("publish-policy");
    let batch = Batch::new(4);
    let mut expected = None;
    for policy in [SyncPolicy::default(), SyncPolicy::NoSync] {
        for batched in [false, true] {
            let out = root.join(format!("{policy:?}-{batched}"));
            let run = || {
                publish::<(), String>(&out, &[], None, policy, |stage| {
                    fs::write(stage.join("a"), b"result").map_err(message)
                })
            };
            if batched {
                batch.run(run).unwrap();
            } else {
                run().unwrap();
            }
            let files = snapshot(&out);
            if let Some(expected) = &expected {
                assert_eq!(&files, expected);
            } else {
                expected = Some(files);
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn no_sync_keeps_quotas_ownership_and_rollback() {
    let root = root("no-sync-safety");
    let out = root.join("result");
    write(&out, "a", b"old").unwrap();
    let old = snapshot(&out);
    let publish = |maximum| {
        publish::<(), String>(&out, &[], maximum, SyncPolicy::NoSync, |stage| {
            fs::write(stage.join("a"), b"new").map_err(message)
        })
    };
    assert!(publish(Some(1)).unwrap_err().contains("exceeds"));
    assert_eq!(snapshot(&out), old);
    {
        let mut t = Transaction::begin(&out, &[]).unwrap();
        fs::write(t.stage.join("a"), b"new").unwrap();
        t.sync = |_| panic!("no-sync called sync");
        t.install_directory = |_, _| Ok(false);
        t.install_file = |_, _| Err(std::io::Error::other("injected install failure"));
        assert!(
            t.install(None, SyncPolicy::NoSync)
                .unwrap_err()
                .contains("previous results restored")
        );
    }
    assert_eq!(snapshot(&out), old);
    fs::write(out.join("a"), b"changed").unwrap();
    assert!(publish(None).unwrap_err().contains("modified"));
    assert_eq!(fs::read(out.join("a")).unwrap(), b"changed");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(any(target_vendor = "apple", target_os = "linux"))]
#[test]
fn fresh_directory_has_one_reservation_and_never_overwrites_late_destinations() {
    let root = root("fresh");
    let out = root.join("result");
    let input = root.join("input");
    fs::write(&input, b"input").unwrap();
    for foreign in ["directory", "file", "input alias", "symlink"] {
        let mut t = Transaction::begin(&out, &[&input]).unwrap();
        assert!(t.fresh);
        assert!(!out.exists());
        assert_eq!(t.stage.parent(), out.parent());
        assert!(Transaction::begin(&out, &[&input]).is_err());
        fs::write(t.stage.join("a"), b"new").unwrap();
        match foreign {
            "directory" => {
                fs::create_dir(&out).unwrap();
                fs::write(out.join("notes"), b"foreign").unwrap();
            }
            "file" => fs::write(&out, b"foreign").unwrap(),
            "input alias" => fs::hard_link(&input, &out).unwrap(),
            "symlink" => std::os::unix::fs::symlink(&input, &out).unwrap(),
            _ => unreachable!(),
        }
        assert!(t.install(None, SyncPolicy::default()).is_err());
        drop(t);
        assert!(!sibling_stage(&out).unwrap().exists());
        assert_eq!(fs::read(&input).unwrap(), b"input");
        if foreign == "directory" {
            assert_eq!(
                snapshot(&out),
                BTreeMap::from([("notes".into(), b"foreign".to_vec())])
            );
            fs::remove_dir_all(&out).unwrap();
        } else {
            fs::remove_file(&out).unwrap();
        }
    }
    write(&out, "a", b"new").unwrap();
    assert!(!sibling_stage(&out).unwrap().exists());
    assert!(!out.join(".pelagic-lock").exists());
    assert_eq!(fs::read(out.join("a")).unwrap(), b"new");
    let mut t = Transaction::begin(&out, &[]).unwrap();
    assert!(!t.fresh);
    fs::write(t.stage.join("b"), b"replacement").unwrap();
    t.install(None, SyncPolicy::default()).unwrap();
    drop(t);
    assert!(!out.join("a").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(any(target_vendor = "apple", target_os = "linux"))]
#[test]
fn directory_rename_failure_and_unsupported_fallback_do_not_clobber() {
    let root = root("directory-rename-fault");
    let out = root.join("result");
    {
        let mut t = Transaction::begin(&out, &[]).unwrap();
        fs::write(t.stage.join("a"), b"new").unwrap();
        t.install_directory = |_, _| Err(std::io::Error::other("injected rename failure"));
        assert!(
            t.install(None, SyncPolicy::default())
                .unwrap_err()
                .contains("injected rename failure")
        );
    }
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    {
        let mut t = Transaction::begin(&out, &[]).unwrap();
        fs::write(t.stage.join("a"), b"new").unwrap();
        t.install_directory = |_, _| Ok(false);
        t.install(None, SyncPolicy::default()).unwrap();
    }
    assert_eq!(fs::read(out.join("a")).unwrap(), b"new");
    assert_eq!(snapshot(&out).len(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_vendor = "apple")]
#[test]
fn a_failed_barrier_precedes_installation_and_preserves_previous_results() {
    let root = root("batch-retain");
    for replace in [false, true] {
        let out = root.join(if replace { "replace" } else { "fresh" });
        if replace {
            write(&out, "a", b"old").unwrap();
        }
        let batch = Batch::new(4);
        batch.run(|| {
            let mut t = Transaction::begin(&out, &[]).unwrap();
            fs::write(t.stage.join("a"), b"new").unwrap();
            t.wait = |_, _, _| Err("injected barrier failure".into());
            assert_eq!(
                t.install(None, SyncPolicy::default()).unwrap_err(),
                "injected barrier failure"
            );
            assert!(!t.backup.exists());
            assert!(!t.retain);
        });
        assert!(!out.join(".pelagic-lock").exists());
        if replace {
            assert_eq!(fs::read(out.join("a")).unwrap(), b"old");
        } else {
            assert!(!out.exists());
        }
        write(&out, "a", b"retry").unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_vendor = "apple")]
#[test]
fn the_single_drive_flush_follows_every_file_sync_and_precedes_installation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SYNCED: AtomicUsize = AtomicUsize::new(0);
    let root = root("full-sync-order");
    let out = root.join("result");
    write(&out, "a", b"old").unwrap();
    let old = snapshot(&out);
    {
        let mut t = Transaction::begin(&out, &[]).unwrap();
        fs::write(t.stage.join("a"), b"new").unwrap();
        fs::write(t.stage.join("z"), b"new").unwrap();
        t.sync = |file| {
            sync_file(file)?;
            SYNCED.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };
        t.full_sync = |_, _| {
            assert_eq!(SYNCED.load(Ordering::SeqCst), 3);
            Err(std::io::Error::other("full flush failure"))
        };
        assert_eq!(
            t.install(None, SyncPolicy::default()).unwrap_err(),
            "full flush failure"
        );
        assert!(!t.backup.exists());
    }
    assert_eq!(snapshot(&out), old);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn closed_file_mutations_are_rehashed_even_with_the_same_size_and_modification_time() {
    let root = root("closed-file-rehash");
    for replace_inode in [false, true] {
        let out = root.join(format!("result-{replace_inode}"));
        publish::<(), String>(&out, &[], None, SyncPolicy::default(), |stage| {
            let path = stage.join("a");
            let mut file = crate::create(&path).unwrap();
            file.write_all(b"first").unwrap();
            file.flush().unwrap();
            drop(file);
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            if replace_inode {
                fs::remove_file(&path).unwrap();
            }
            fs::write(&path, b"later").unwrap();
            // Timestamps can collide within one filesystem clock tick; they are not versions.
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(modified)
                .unwrap();
            Ok(())
        })
        .unwrap();
        assert!(verified_manifest_sha256(&out).unwrap().is_some());
        assert_eq!(fs::read(out.join("a")).unwrap(), b"later");
    }
    fs::remove_dir_all(root).unwrap();
}

// Real process exits leave the transaction's destructors unrun, unlike unwinding a panic.
#[test]
fn crash_child() {
    let Ok(root) = std::env::var("PELAGIC_TEST_CRASH_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let out = root.join("result");
    let batch = Batch::new(4);
    batch.run(|| {
        let mut t = Transaction::begin(&out, &[]).unwrap();
        fs::write(t.stage.join("a"), b"new").unwrap();
        t.after_step = Some(|_, step| {
            if std::env::var("PELAGIC_TEST_CRASH_STEP").unwrap() == step {
                std::process::exit(73);
            }
        });
        t.after_install = Some(|_, name| {
            if name == "a" && std::env::var("PELAGIC_TEST_CRASH_STEP").unwrap() == "install" {
                std::process::exit(73);
            }
        });
        t.install(None, SyncPolicy::default()).unwrap();
    });
    // This is the same ordering used by run-many: record only after publish returned.
    fs::write(root.join("status"), b"ok\n").unwrap();
    std::process::exit(73);
}

#[test]
fn crashes_at_each_step_never_leave_an_early_record_or_adopt_abandoned_work() {
    let root = root("crash-steps");
    for replace in [false, true] {
        for step in [
            "staged",
            "synced",
            "renamed",
            "install",
            "installed",
            "barrier",
            "record",
        ] {
            let fresh_rename = cfg!(any(target_vendor = "apple", target_os = "linux")) && !replace;
            if step == "renamed" && !fresh_rename || step == "install" && fresh_rename {
                continue;
            }
            let case = root.join(format!("{replace}-{step}"));
            fs::create_dir(&case).unwrap();
            let out = case.join("result");
            if replace {
                write(&out, "a", b"old").unwrap();
            }
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "directory::tests::publication::crash_child"])
                .env("PELAGIC_TEST_CRASH_ROOT", &case)
                .env("PELAGIC_TEST_CRASH_STEP", step)
                .output()
                .unwrap();
            assert_eq!(
                status.status.code(),
                Some(73),
                "{step}: {}",
                String::from_utf8_lossy(&status.stderr)
            );
            assert_eq!(case.join("status").exists(), step == "record");
            if step == "record" {
                assert!(verified_manifest_sha256(&out).unwrap().is_some());
                write(&out, "a", b"retry").unwrap();
            } else {
                assert!(
                    write(&out, "a", b"retry").is_err(),
                    "must not adopt interrupted {step}"
                );
                if replace && ["staged", "barrier", "synced"].contains(&step) {
                    assert_eq!(fs::read(out.join("a")).unwrap(), b"old");
                }
                if ["renamed", "installed"].contains(&step) {
                    assert_eq!(fs::read(out.join("a")).unwrap(), b"new");
                }
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn writers_run_outside_the_publication_gate() {
    let root = root("gate-writer");
    let out = root.join("result");
    for policy in [SyncPolicy::Sync, SyncPolicy::NoSync] {
        publish::<(), String>(&out, &[], None, policy, |stage| {
            assert_eq!(
                gate::held(),
                0,
                "the writer solves and writes outside the gate"
            );
            fs::write(stage.join("a"), b"result").map_err(message)
        })
        .unwrap();
        assert_eq!(gate::held(), 0, "publication releases its place");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn durable_syncs_and_drive_flushes_wait_outside_the_publication_gate() {
    let root = root("gate-sync");
    for stem in ["fresh", "update"] {
        let out = root.join(stem);
        if stem == "update" {
            write(&out, "a", b"old").unwrap();
        }
        let mut t = Transaction::begin(&out, &[]).unwrap();
        fs::write(t.stage.join("a"), b"new").unwrap();
        t.gate = Some(gate::enter());
        let inside = gate::held();
        assert_eq!(inside, usize::from(cfg!(target_vendor = "apple")));
        t.sync = |_| {
            assert_eq!(gate::held(), 0, "a file sync waited inside the gate");
            Ok(())
        };
        #[cfg(target_vendor = "apple")]
        {
            t.full_sync = |_, _| {
                assert_eq!(gate::held(), 0, "the drive flush waited inside the gate");
                Ok(())
            };
            t.wait = |_, _, _| {
                assert_eq!(gate::held(), 0, "the shared flush waited inside the gate");
                Ok(())
            };
        }
        t.after_step = Some(|_, step| {
            if step == "synced" {
                assert_eq!(gate::held(), usize::from(cfg!(target_vendor = "apple")));
            }
        });
        t.install(None, SyncPolicy::Sync).unwrap();
        assert_eq!(gate::held(), inside, "installation resumes inside the gate");
        drop(t);
        assert_eq!(gate::held(), 0, "cleanup releases the gate");
        assert_eq!(fs::read(out.join("a")).unwrap(), b"new");
    }
    fs::remove_dir_all(root).unwrap();
}

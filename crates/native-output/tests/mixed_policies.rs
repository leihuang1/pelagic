use native_output::directory::{Batch, SyncPolicy, publish, verified_manifest_sha256};
use std::fs;
use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

#[test]
fn mixed_policies_and_nonpublishing_workers_finish_without_a_full_cohort() {
    let root = std::env::temp_dir().join(format!("pelagic-mixed-policies-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    for workers in [4, 14] {
        for others in ["no-sync", "no-publication", "writer-error"] {
            let batch = Batch::new(workers);
            let start = Arc::new(Barrier::new(workers));
            let (finished, received) = mpsc::channel();
            let mut handles = Vec::new();
            for index in 0..workers {
                let batch = Arc::clone(&batch);
                let start = Arc::clone(&start);
                let finished = finished.clone();
                let output = root.join(format!("{workers}-{others}-{index}"));
                handles.push(std::thread::spawn(move || {
                    start.wait();
                    batch.run(|| {
                        if index > 0 && others == "no-publication" {
                            assert!(!output.exists());
                        } else {
                            let policy = if index == 0 {
                                SyncPolicy::Sync
                            } else {
                                SyncPolicy::NoSync
                            };
                            let result =
                                publish::<(), String>(&output, &[], None, policy, |stage| {
                                    fs::write(stage.join("a"), b"result")
                                        .map_err(|e| e.to_string())?;
                                    if index > 0 && others == "writer-error" {
                                        Err("writer failed before barrier".into())
                                    } else {
                                        Ok(())
                                    }
                                });
                            if index > 0 && others == "writer-error" {
                                assert_eq!(result.unwrap_err(), "writer failed before barrier");
                                assert!(!output.exists());
                            } else {
                                result.unwrap();
                                assert!(verified_manifest_sha256(&output).unwrap().is_some());
                            }
                        }
                    });
                    finished.send(index).unwrap();
                }));
            }
            drop(finished);
            // Join only after completion: a broken barrier must fail, not hang a scoped join.
            for _ in 0..workers {
                received.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            for handle in handles {
                handle.join().unwrap();
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

fn staged(path: &std::path::Path) -> File {
    File::open(path).unwrap()
}

#[test]
fn lone_barrier_runs_immediately_and_later_arrivals_share_the_next_barrier() {
    let root = super::super::tests::root("batch-order");
    let path = root.join("file");
    std::fs::write(&path, b"data").unwrap();
    let batch = Batch::empty();
    let calls = AtomicUsize::new(0);
    let recorded = AtomicUsize::new(0);
    let (entered, entering) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Mutex::new(released);
    let barrier = |ready: &[Arc<Request>]| {
        let index = calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(ready.len(), if index == 0 { 1 } else { 3 });
        if index == 0 {
            entered.send(()).unwrap();
            released.lock().unwrap().recv().unwrap();
            assert_eq!(
                recorded.load(Ordering::SeqCst),
                0,
                "no record before its barrier"
            );
        }
        vec![Ok(()); ready.len()]
    };
    std::thread::scope(|s| {
        let finish = || {
            batch.wait_with(staged(&path), &barrier).unwrap();
            recorded.fetch_add(1, Ordering::SeqCst);
        };
        s.spawn(finish);
        entering.recv().unwrap(); // A lone caller cannot be waiting for a timer or another case.
        for _ in 0..3 {
            s.spawn(finish);
        }
        while batch.state.lock().unwrap().pending.len() < 3 {
            std::thread::yield_now();
        }
        release.send(()).unwrap();
    });
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(recorded.load(Ordering::SeqCst), 4);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn barrier_error_reaches_every_covered_case_and_the_next_batch_can_succeed() {
    let root = super::super::tests::root("batch-error");
    let path = root.join("file");
    std::fs::write(&path, b"data").unwrap();
    let batch = Batch::empty();
    let (entered, entering) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Mutex::new(released);
    let calls = AtomicUsize::new(0);
    let barrier = |ready: &[Arc<Request>]| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            entered.send(()).unwrap();
            released.lock().unwrap().recv().unwrap();
        }
        vec![Err("injected barrier failure".into()); ready.len()]
    };
    std::thread::scope(|s| {
        let wait = || {
            assert_eq!(
                batch.wait_with(staged(&path), &barrier).unwrap_err(),
                "injected barrier failure"
            );
        };
        s.spawn(wait);
        entering.recv().unwrap();
        for _ in 0..3 {
            s.spawn(wait);
        }
        while batch.state.lock().unwrap().pending.len() < 3 {
            std::thread::yield_now();
        }
        release.send(()).unwrap();
    });
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    batch
        .wait_with(staged(&path), &|ready| vec![Ok(()); ready.len()])
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_scopes_restore_after_nested_calls_and_unwind() {
    let outer = Arc::new(Batch::empty());
    let inner = Arc::new(Batch::empty());
    assert!(Batch::current().is_none());
    outer.run(|| {
        let _ = std::panic::catch_unwind(|| inner.run(|| panic!("injected")));
        assert!(Arc::ptr_eq(&Batch::current().unwrap(), &outer));
    });
    assert!(Batch::current().is_none());
}

#[test]
fn two_barriers_allow_a_late_case_to_finish_without_borrowing_the_first_barrier() {
    let root = super::super::tests::root("batch-overlap");
    let path = root.join("file");
    std::fs::write(&path, b"data").unwrap();
    let mut batch = Batch::empty();
    batch.limit = 2;
    let (entered, entering) = mpsc::channel();
    let (first, first_gate) = mpsc::channel();
    let (second, second_gate) = mpsc::channel();
    let (third, third_gate) = mpsc::channel();
    let gates = [
        Mutex::new(first_gate),
        Mutex::new(second_gate),
        Mutex::new(third_gate),
    ];
    let calls = AtomicUsize::new(0);
    let recorded = AtomicUsize::new(0);
    let barrier = |ready: &[Arc<Request>]| {
        let index = calls.fetch_add(1, Ordering::SeqCst);
        entered.send((index, ready.len())).unwrap();
        if let Some(gate) = gates.get(index) {
            gate.lock().unwrap().recv().unwrap();
        }
        vec![Ok(()); ready.len()]
    };
    std::thread::scope(|s| {
        let finish = || {
            batch.wait_with(staged(&path), &barrier).unwrap();
            recorded.fetch_add(1, Ordering::SeqCst);
        };
        s.spawn(finish);
        assert_eq!(entering.recv().unwrap(), (0, 1));
        s.spawn(finish);
        assert_eq!(entering.recv().unwrap(), (1, 1));
        for _ in 0..2 {
            s.spawn(finish);
        }
        while batch.state.lock().unwrap().pending.len() < 2 {
            std::thread::yield_now();
        }
        assert!(
            entering.try_recv().is_err(),
            "only two barriers may be active"
        );
        assert_eq!(recorded.load(Ordering::SeqCst), 0);
        second.send(()).unwrap();
        assert_eq!(entering.recv().unwrap(), (2, 2));
        while recorded.load(Ordering::SeqCst) < 1 {
            std::thread::yield_now();
        }
        assert_eq!(
            recorded.load(Ordering::SeqCst),
            1,
            "the second case can finish before the first"
        );
        first.send(()).unwrap();
        while recorded.load(Ordering::SeqCst) < 2 {
            std::thread::yield_now();
        }
        let unexpected = entering.recv_timeout(std::time::Duration::from_millis(50));
        third.send(()).unwrap();
        assert!(
            unexpected.is_err(),
            "covered waiters must not start empty barriers: {unexpected:?}"
        );
    });
    assert_eq!(recorded.load(Ordering::SeqCst), 4);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_drive_barrier_fans_out_errors_to_its_covered_cases() {
    let root = super::super::tests::root("batch-device");
    let old = root.join("old");
    let new = root.join("new");
    std::fs::write(&old, b"1").unwrap();
    std::fs::write(&new, b"22").unwrap();
    let old = staged(&old);
    let new = staged(&new);
    let ready: Vec<_> = [new, old]
        .into_iter()
        .map(|handle| {
            Arc::new(Request {
                handle,
                result: OnceLock::new(),
            })
        })
        .collect();
    let calls = AtomicUsize::new(0);
    let results = filesystem_barriers(&ready, |file| {
        calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(file.metadata().unwrap().len(), 2);
        Err("writeback failure".into())
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(results, vec![Err("writeback failure".into()); 2]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn single_workers_flush_independently_and_pools_bound_their_drive_barriers() {
    let single = Batch::new(1);
    assert_eq!(single.limit, 0);
    let outer = Arc::new(Batch::empty());
    outer.run(|| {
        single.run(|| assert!(Batch::current().is_none()));
        assert!(Arc::ptr_eq(&Batch::current().unwrap(), &outer));
    });
    for workers in [2, 3, 4, 8, 14] {
        let limit = if workers <= 4 { workers } else { 1 };
        assert_eq!(Batch::new(workers).limit, limit);
    }
}

//! Bounded file syncs. A refused helper leaves its work to the calling thread.
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::{self, Scope, ScopedJoinHandle};

const PARTICIPANTS: usize = 16;
/// Helper threads in flight across the whole process, shared by concurrent publications.
const HELPERS: usize = 64;
/// Open files kept for each worker's own inputs, writers, reports and Apple barrier manifest,
/// and for the process's standard streams and status file, before any helper holds one.
const PER_WORKER: u64 = 8;
const PROCESS: u64 = 32;
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// Workers of the live run-many pools. A publication outside a pool counts as one.
static WORKERS: AtomicUsize = AtomicUsize::new(0);

/// A worker pool's share of the open-file headroom, released when the pool ends.
pub(super) struct Workers(usize);

impl Workers {
    pub(super) fn register(count: usize) -> Self {
        WORKERS.fetch_add(count, Ordering::AcqRel);
        Self(count)
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        WORKERS.fetch_sub(self.0, Ordering::AcqRel);
    }
}

/// Why one file's sync did not succeed.
pub(super) enum Failure {
    /// No descriptor was free to open the file, so nothing was synced: trying it again is safe.
    Descriptors(std::io::Error),
    Other(std::io::Error),
}

impl Failure {
    pub(super) fn open(error: std::io::Error) -> Self {
        if out_of_descriptors(&error) {
            Self::Descriptors(error)
        } else {
            Self::Other(error)
        }
    }

    fn message(self) -> String {
        match self {
            Self::Descriptors(error) | Self::Other(error) => error.to_string(),
        }
    }
}

fn out_of_descriptors(error: &std::io::Error) -> bool {
    #[cfg(any(target_vendor = "apple", target_os = "linux"))]
    {
        use rustix::io::Errno;
        Errno::from_io_error(error).is_some_and(|e| e == Errno::MFILE || e == Errno::NFILE)
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
    {
        let _ = error;
        false
    }
}

fn helper_limit() -> usize {
    // Read for every publication: a library host may lower its limit at any time.
    #[cfg(any(target_vendor = "apple", target_os = "linux"))]
    let soft = rustix::process::getrlimit(rustix::process::Resource::Nofile).current;
    #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
    let soft = None;
    helpers_for(soft, WORKERS.load(Ordering::Acquire).max(1))
}

/// Each helper holds one open file, so helpers use at most a quarter of what remains after every
/// worker's and the process's reserve. Without that headroom, each caller syncs its own files one
/// after another, as before concurrent syncs. `None` is an unlimited soft limit.
fn helpers_for(soft_open_files: Option<u64>, workers: usize) -> usize {
    soft_open_files.map_or(HELPERS, |soft| {
        let workers = u64::try_from(workers).unwrap_or(u64::MAX);
        let reserved = PER_WORKER.saturating_mul(workers).saturating_add(PROCESS);
        usize::try_from(soft.saturating_sub(reserved) / 4).map_or(HELPERS, |n| n.min(HELPERS))
    })
}

/// Raise the soft open-file limit to the hard limit, as Go and Node.js programs do at startup,
/// so that sync helpers fit beside a large worker pool. macOS accepts at most `OPEN_MAX` (10240).
/// Neither limit is ever lowered.
///
/// # Errors
///
/// The system refused the change, as some sandboxes do. The limits are then unchanged, and later
/// publications budget their helpers for them.
pub fn raise_open_file_limit() -> std::io::Result<()> {
    #[cfg(any(target_vendor = "apple", target_os = "linux"))]
    {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
        let Rlimit { current, maximum } = getrlimit(Resource::Nofile);
        #[cfg(target_vendor = "apple")]
        let target = Some(maximum.map_or(10_240, |hard| hard.min(10_240)));
        #[cfg(target_os = "linux")]
        let target = maximum;
        if let (Some(soft), Some(target)) = (current, target)
            && soft < target
        {
            setrlimit(
                Resource::Nofile,
                Rlimit {
                    current: Some(target),
                    maximum,
                },
            )?;
        }
    }
    Ok(())
}

type Spawn = for<'scope, 'env> fn(
    &'scope Scope<'scope, 'env>,
    Box<dyn FnOnce() + Send + 'scope>,
) -> std::io::Result<ScopedJoinHandle<'scope, ()>>;

fn spawn<'scope>(
    scope: &'scope Scope<'scope, '_>,
    work: Box<dyn FnOnce() + Send + 'scope>,
) -> std::io::Result<ScopedJoinHandle<'scope, ()>> {
    thread::Builder::new().spawn_scoped(scope, work)
}

/// One reserved helper; dropping it, including with a closure that never ran, frees the slot.
struct Slot<'a>(&'a AtomicUsize);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn reserve(in_flight: &AtomicUsize, limit: usize) -> Option<Slot<'_>> {
    let mut current = in_flight.load(Ordering::Acquire);
    while current < limit {
        match in_flight.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Some(Slot(in_flight)),
            Err(actual) => current = actual,
        }
    }
    None
}

pub(super) fn sync_paths(
    paths: &[PathBuf],
    sync: impl Fn(&std::path::Path) -> Result<(), Failure> + Sync,
) -> Vec<Result<(), String>> {
    sync_with(
        paths,
        PARTICIPANTS,
        (&IN_FLIGHT, helper_limit()),
        sync,
        spawn,
    )
}

fn sync_with(
    paths: &[PathBuf],
    participants: usize,
    (in_flight, limit): (&AtomicUsize, usize),
    sync: impl Fn(&std::path::Path) -> Result<(), Failure> + Sync,
    spawn: Spawn,
) -> Vec<Result<(), String>> {
    let results: Vec<_> = paths.iter().map(|_| OnceLock::new()).collect();
    let next = AtomicUsize::new(0);
    let work = || {
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(path) = paths.get(index) else { break };
            match sync(path) {
                // Left unset, for the caller to try again once every helper has closed its file.
                Err(Failure::Descriptors(_)) => {}
                result => {
                    let _ = results[index].set(result.map_err(Failure::message));
                }
            }
        }
    };
    let work = &work;
    thread::scope(|scope| {
        for _ in 1..participants.min(paths.len()) {
            // Without a free process-wide slot, the caller and existing helpers do the rest.
            let Some(slot) = reserve(in_flight, limit) else {
                break;
            };
            let helper = Box::new(move || {
                let _slot = slot;
                work();
            });
            if spawn(scope, helper).is_err() {
                break;
            }
        }
        work();
    });
    // Every helper has finished. A file that found no descriptor gets one more attempt, alone, as
    // one-at-a-time syncing would have made; that attempt's result is final.
    results
        .into_iter()
        .zip(paths)
        .map(|(result, path)| {
            result
                .into_inner()
                .unwrap_or_else(|| sync(path).map_err(Failure::message))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn concurrent_syncs_include_the_caller_are_bounded_and_keep_file_order() {
        let paths: Vec<_> = (0..12).map(|i| PathBuf::from(i.to_string())).collect();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let calls = AtomicUsize::new(0);
        let caller = thread::current().id();
        let on_caller = AtomicUsize::new(0);
        let gate = Barrier::new(4);
        let in_flight = AtomicUsize::new(0);
        let results = sync_with(
            &paths,
            4,
            (&in_flight, HELPERS),
            |path| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                if thread::current().id() == caller {
                    on_caller.fetch_add(1, Ordering::SeqCst);
                }
                if calls.fetch_add(1, Ordering::SeqCst) < 4 {
                    gate.wait();
                }
                active.fetch_sub(1, Ordering::SeqCst);
                Err(Failure::Other(std::io::Error::other(
                    path.display().to_string(),
                )))
            },
            spawn,
        );
        assert_eq!(peak.load(Ordering::SeqCst), 4);
        assert_eq!(in_flight.load(Ordering::SeqCst), 0);
        assert_eq!(calls.load(Ordering::SeqCst), paths.len());
        assert!(on_caller.load(Ordering::SeqCst) > 0);
        assert_eq!(
            results,
            (0..12).map(|i| Err(i.to_string())).collect::<Vec<_>>()
        );
    }

    #[test]
    fn failed_spawn_falls_back_to_the_caller_and_still_attempts_every_file() {
        let paths: Vec<_> = ["a", "b", "pelagic-manifest.json", "z"]
            .map(PathBuf::from)
            .into();
        let caller = thread::current().id();
        let calls = AtomicUsize::new(0);
        let in_flight = AtomicUsize::new(0);
        let results = sync_with(
            &paths,
            4,
            (&in_flight, HELPERS),
            |path| {
                assert_eq!(thread::current().id(), caller);
                calls.fetch_add(1, Ordering::SeqCst);
                if path == std::path::Path::new("b") {
                    Err(Failure::Other(std::io::Error::other("file error")))
                } else {
                    Ok(())
                }
            },
            |_, _| Err(std::io::Error::other("spawn refused")),
        );
        assert_eq!(calls.load(Ordering::SeqCst), paths.len());
        assert_eq!(
            results,
            vec![Ok(()), Err("file error".into()), Ok(()), Ok(())]
        );
        // A refused spawn drops its closure, which frees the reserved slot.
        assert_eq!(in_flight.load(Ordering::SeqCst), 0);
        let none = sync_with(&[], 4, (&in_flight, HELPERS), |_| unreachable!(), spawn);
        assert_eq!(none, []);
    }

    #[test]
    fn helpers_only_use_open_files_left_after_every_workers_reserve() {
        assert_eq!(helpers_for(None, 1), HELPERS);
        assert_eq!(helpers_for(Some(1_048_576), 4096), HELPERS);
        // A lone run under macOS's default limit, and a 24-worker sweep under Linux's.
        assert_eq!(helpers_for(Some(256), 1), 54);
        assert_eq!(helpers_for(Some(1024), 24), HELPERS);
        assert_eq!(helpers_for(Some(1024), 112), 24);
        // No headroom: every caller syncs its own files, as before concurrent syncs.
        assert_eq!(helpers_for(Some(1024), 124), 0);
        assert_eq!(helpers_for(Some(64), 24), 0);
        assert_eq!(helpers_for(Some(64), 4), 0);
        assert_eq!(helpers_for(Some(64), 1), 6);
        assert_eq!(helpers_for(Some(3), 1), 0);
        assert_eq!(helpers_for(Some(1024), usize::MAX), 0);
    }

    #[cfg(any(target_vendor = "apple", target_os = "linux"))]
    #[test]
    fn raising_the_open_file_limit_never_lowers_either_limit() {
        use rustix::process::{Resource, getrlimit};
        // `None` is unlimited.
        let finite = |limit: Option<u64>| limit.unwrap_or(u64::MAX);
        let before = getrlimit(Resource::Nofile);
        let raised = raise_open_file_limit();
        let after = getrlimit(Resource::Nofile);
        assert_eq!(after.maximum, before.maximum);
        assert!(
            finite(after.current) >= finite(before.current),
            "{before:?} -> {after:?}"
        );
        #[cfg(target_vendor = "apple")]
        let target = before.maximum.map_or(10_240, |hard| hard.min(10_240));
        #[cfg(target_os = "linux")]
        let target = before.maximum.unwrap_or(finite(before.current));
        match raised {
            Ok(()) => assert!(finite(after.current) >= target, "{before:?} -> {after:?}"),
            // Some sandboxes refuse the change; the limits then stay as they were.
            Err(_) => assert_eq!(after.current, before.current),
        }
    }

    #[test]
    fn files_that_found_no_descriptor_are_retried_alone_after_every_helper() {
        let paths: Vec<_> = (0..12).map(|i| PathBuf::from(i.to_string())).collect();
        let caller = thread::current().id();
        // Per file: attempts so far, and whether the first ran on a helper.
        let attempts = std::sync::Mutex::new([(0, false); 12]);
        let helper_attempts = AtomicUsize::new(0);
        let in_flight = AtomicUsize::new(0);
        let results = sync_with(
            &paths,
            4,
            (&in_flight, HELPERS),
            |path| {
                let file: usize = path.to_str().unwrap().parse().unwrap();
                let on_caller = thread::current().id() == caller;
                let attempt = {
                    let mut attempts = attempts.lock().unwrap();
                    attempts[file].0 += 1;
                    if attempts[file].0 == 1 {
                        attempts[file].1 = !on_caller;
                    }
                    attempts[file].0
                };
                if attempt == 2 {
                    assert!(on_caller && in_flight.load(Ordering::SeqCst) == 0);
                } else if on_caller {
                    // Hold the caller until a helper has taken a file, so some are deferred.
                    let start = std::time::Instant::now();
                    while helper_attempts.load(Ordering::SeqCst) == 0
                        && start.elapsed() < std::time::Duration::from_secs(5)
                    {
                        thread::yield_now();
                    }
                } else {
                    helper_attempts.fetch_add(1, Ordering::SeqCst);
                }
                match file {
                    5 => Err(Failure::Descriptors(std::io::Error::other(
                        "still no descriptor",
                    ))),
                    7 => Err(Failure::Other(std::io::Error::other("disk error"))),
                    _ if !on_caller => {
                        Err(Failure::Descriptors(std::io::Error::other("no descriptor")))
                    }
                    _ => Ok(()),
                }
            },
            spawn,
        );
        let mut expected = vec![Ok(()); 12];
        expected[5] = Err("still no descriptor".to_string());
        expected[7] = Err("disk error".to_string());
        assert_eq!(results, expected);
        assert!(helper_attempts.load(Ordering::SeqCst) > 0);
        // Only a missing descriptor earns a second attempt, and never a third.
        for (file, (count, first_on_helper)) in
            attempts.into_inner().unwrap().into_iter().enumerate()
        {
            let deferred = file == 5 || (first_on_helper && file != 7);
            assert_eq!(count, if deferred { 2 } else { 1 }, "file {file}");
        }
    }

    #[test]
    fn a_live_pool_reserves_open_files_for_each_of_its_workers() {
        // So many workers leave no headroom under any finite limit, whatever other tests hold.
        let pool = Workers::register(usize::MAX / 2);
        #[cfg(any(target_vendor = "apple", target_os = "linux"))]
        assert_eq!(helper_limit(), 0);
        drop(pool);
        assert!(WORKERS.load(Ordering::SeqCst) < usize::MAX / 2);
    }

    #[test]
    fn concurrent_publications_share_one_process_wide_helper_limit() {
        thread_local! {
            static CALLER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        let paths: Vec<_> = (0..16).map(|i| PathBuf::from(i.to_string())).collect();
        let in_flight = AtomicUsize::new(0);
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        // Two publications at once, each wanting 15 helpers, may have 3 syncing between them.
        // A finished helper frees its slot for another, so count simultaneous helpers only.
        thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    CALLER.with(|caller| caller.set(true));
                    let results = sync_with(
                        &paths,
                        PARTICIPANTS,
                        (&in_flight, 3),
                        |_| {
                            let helper = !CALLER.with(std::cell::Cell::get);
                            if helper {
                                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                                peak.fetch_max(now, Ordering::SeqCst);
                            }
                            // Hold each file long enough for helpers to overlap.
                            thread::sleep(std::time::Duration::from_millis(1));
                            if helper {
                                active.fetch_sub(1, Ordering::SeqCst);
                            }
                            Ok(())
                        },
                        spawn,
                    );
                    assert_eq!(results, vec![Ok(()); 16]);
                });
            }
        });
        assert!(peak.load(Ordering::SeqCst) <= 3);
        assert_eq!(in_flight.load(Ordering::SeqCst), 0);
        // With every slot taken elsewhere, the caller syncs every file itself.
        let caller = thread::current().id();
        let full = AtomicUsize::new(3);
        let results = sync_with(
            &paths,
            PARTICIPANTS,
            (&full, 3),
            |_| {
                assert_eq!(thread::current().id(), caller);
                Ok(())
            },
            spawn,
        );
        assert_eq!(results, vec![Ok(()); 16]);
        assert_eq!(full.load(Ordering::SeqCst), 3);
    }
}

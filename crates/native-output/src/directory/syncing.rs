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
/// A refused change keeps the current limit.
pub fn raise_open_file_limit() {
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
            let _ = setrlimit(
                Resource::Nofile,
                Rlimit {
                    current: Some(target),
                    maximum,
                },
            );
        }
    }
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
    sync: impl Fn(&std::path::Path) -> Result<(), String> + Sync,
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
    sync: impl Fn(&std::path::Path) -> Result<(), String> + Sync,
    spawn: Spawn,
) -> Vec<Result<(), String>> {
    let results: Vec<_> = paths.iter().map(|_| OnceLock::new()).collect();
    let next = AtomicUsize::new(0);
    let work = || {
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(path) = paths.get(index) else { break };
            let _ = results[index].set(sync(path));
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
    results
        .into_iter()
        .map(|result| result.into_inner().expect("every file was synced"))
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
                Err(path.display().to_string())
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
                    Err("file error".into())
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
    fn the_soft_open_file_limit_rises_to_the_hard_limit() {
        use rustix::process::{Resource, getrlimit};
        raise_open_file_limit();
        let limit = getrlimit(Resource::Nofile);
        #[cfg(target_vendor = "apple")]
        let expected = limit.maximum.map_or(10_240, |hard| hard.min(10_240));
        #[cfg(target_os = "linux")]
        let expected = limit.maximum.unwrap_or(u64::MAX);
        // A limit that was already higher is never lowered.
        assert!(
            limit.current.is_none_or(|soft| soft >= expected),
            "{limit:?}"
        );
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

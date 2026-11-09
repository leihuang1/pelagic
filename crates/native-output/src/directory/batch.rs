//! Completion-driven Apple drive flushes, after each case's concurrent file syncs.
use std::cell::RefCell;
use std::fs::File;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

thread_local! {
    static CURRENT: RefCell<Option<Arc<Batch>>> = const { RefCell::new(None) };
}

/// Shared Apple drive flushes for independent publications. Installation follows the barrier;
/// `publish` returns only after both succeed, unless explicitly using `SyncPolicy::NoSync`,
/// which bypasses the barrier. This does not promise directory durability.
pub struct Batch {
    state: Mutex<State>,
    changed: Condvar,
    limit: usize,
    _workers: super::syncing::Workers,
}

#[derive(Default)]
struct State {
    active: usize,
    pending: Vec<Arc<Request>>,
}

struct Request {
    handle: File,
    /// The publication's manifest, under which test observers see this request's drive flush.
    path: std::path::PathBuf,
    result: OnceLock<Result<(), String>>,
}

impl Batch {
    fn with(limit: usize, workers: usize) -> Self {
        Self {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            limit,
            _workers: super::syncing::Workers::register(workers),
        }
    }

    #[cfg(test)]
    fn empty() -> Self {
        Self::with(1, 0)
    }

    /// Create a drive-flush coordinator for this worker pool. While it lives, sync helpers
    /// leave open files for each of its `workers`.
    #[must_use]
    pub fn new(workers: usize) -> Arc<Self> {
        // Small pools must not wait twice; larger pools benefit from coalescing.
        let limit = match workers {
            0 | 1 => 0,
            2..=4 => workers,
            _ => 1,
        };
        Arc::new(Self::with(limit, workers))
    }

    /// Use this batch on the calling thread, restoring the previous scope even on unwind.
    /// Numerical child threads do not inherit publication state.
    pub fn run<T>(self: &Arc<Self>, operation: impl FnOnce() -> T) -> T {
        struct Scope(Option<Arc<Batch>>);
        impl Drop for Scope {
            fn drop(&mut self) {
                CURRENT.with(|slot| slot.replace(self.0.take()));
            }
        }
        let _scope =
            Scope(CURRENT.with(|slot| slot.replace((self.limit > 0).then(|| Arc::clone(self)))));
        operation()
    }

    pub(super) fn current() -> Option<Arc<Self>> {
        CURRENT.with(|slot| slot.borrow().clone())
    }

    #[cfg(target_vendor = "apple")]
    pub(super) fn wait(&self, handle: File, path: &std::path::Path) -> Result<(), String> {
        // Admission only: `full_sync` observes the drive flush that covers this request.
        #[cfg(any(test, feature = "test-observer"))]
        super::sync_observer::record(path, super::sync_observer::Kind::SharedBarrier);
        self.wait_with(handle, path, &|ready| {
            filesystem_barriers(ready, |request| {
                super::full_sync(&request.handle, &request.path).map_err(super::message)
            })
        })
    }

    fn wait_with(
        &self,
        handle: File,
        path: &std::path::Path,
        barrier: &impl Fn(&[Arc<Request>]) -> Vec<Result<(), String>>,
    ) -> Result<(), String> {
        let request = Arc::new(Request {
            handle,
            path: path.to_path_buf(),
            result: OnceLock::new(),
        });
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending.push(Arc::clone(&request));
        loop {
            if let Some(result) = request.result.get() {
                return result.clone();
            }
            // A covered waiter must wait for its own barrier, not start empty/unrelated work.
            if state.active >= self.limit
                || !state
                    .pending
                    .iter()
                    .any(|pending| Arc::ptr_eq(pending, &request))
            {
                state = self
                    .changed
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                continue;
            }
            state.active += 1;
            let ready = std::mem::take(&mut state.pending);
            drop(state);
            let results = barrier(&ready);
            assert_eq!(results.len(), ready.len());
            state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (ready, result) in ready.into_iter().zip(results) {
                let _ = ready.result.set(result);
            }
            state.active -= 1;
            self.changed.notify_all();
        }
    }
}

fn filesystem_barriers(
    ready: &[Arc<Request>],
    sync: impl Fn(&Request) -> Result<(), String>,
) -> Vec<Result<(), String>> {
    use std::os::unix::fs::MetadataExt;
    let mut groups: std::collections::BTreeMap<u64, Vec<usize>> = std::collections::BTreeMap::new();
    let mut results = vec![Ok(()); ready.len()];
    for (index, request) in ready.iter().enumerate() {
        match request.handle.metadata() {
            Ok(m) => groups.entry(m.dev()).or_default().push(index),
            Err(e) => results[index] = Err(super::message(e)),
        }
    }
    for group in groups.values() {
        let result = sync(&ready[group[0]]);
        for &index in group {
            results[index].clone_from(&result);
        }
    }
    results
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;

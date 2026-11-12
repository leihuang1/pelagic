//! A cap on how many publications run their metadata-heavy phases at once.
//!
//! Each publication creates, lists, renames and removes names: its scratch directory, artifacts,
//! manifest and lock. On macOS, many threads doing that at once contend in the kernel until
//! throughput collapses. A 3,000-case Pekeris `run-many --no-sync` sweep on an Apple M4 Pro took
//! 2.4 s with 14 workers against 1.0 s with 6, and five times the kernel time. Gating `begin`,
//! installation and cleanup removes that collapse while solving and writing stay fully parallel.
//! Only `--no-sync` publications are gated: durable ones are bound by syncs and did not collapse,
//! and gating them staggered their arrival at the shared drive flush, so fewer publications shared
//! each flush (1.45 times slower at 14 workers). A gated transaction still never waits for a sync
//! inside the gate. Linux and Windows showed no such collapse, so other platforms are not gated.
use std::sync::{Condvar, Mutex, PoisonError};

/// Publications in a gated phase at once on Apple platforms. On an M4 Pro, 4 made a 14-worker
/// sweep take 0.46 of its ungated time and changed nothing up to 4 workers. 3 was slightly faster at
/// 10 to 14 workers but slower at 6, and 6 collapsed again.
const APPLE_LIMIT: usize = 4;

/// The process-wide gate shared by every publication.
pub(crate) static PUBLICATIONS: Gate = Gate::new(if cfg!(target_vendor = "apple") {
    APPLE_LIMIT
} else {
    usize::MAX
});

/// A counting semaphore; `usize::MAX` never blocks and takes no lock.
pub(crate) struct Gate {
    limit: usize,
    active: Mutex<usize>,
    freed: Condvar,
}

/// One place in a [`Gate`], released when dropped, including during unwinding.
pub(crate) struct Guard<'a>(Option<&'a Gate>);

impl Gate {
    pub(crate) const fn new(limit: usize) -> Self {
        Self {
            limit,
            active: Mutex::new(0),
            freed: Condvar::new(),
        }
    }

    /// Wait for a place. The guarded section only counts, so a poisoned lock is still consistent.
    pub(crate) fn enter(&self) -> Guard<'_> {
        if self.limit == usize::MAX {
            return Guard(None);
        }
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        while *active >= self.limit {
            active = self
                .freed
                .wait(active)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *active += 1;
        #[cfg(test)]
        HELD.with(|held| held.set(held.get() + 1));
        Guard(Some(self))
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if let Some(gate) = self.0 {
            *gate.active.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
            #[cfg(test)]
            HELD.with(|held| held.set(held.get() - 1));
            gate.freed.notify_one();
        }
    }
}

/// Enter the shared publication gate.
pub(crate) fn enter() -> Guard<'static> {
    PUBLICATIONS.enter()
}

#[cfg(test)]
thread_local! {
    static HELD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Places that the current thread holds in limited gates.
#[cfg(test)]
pub(crate) fn held() -> usize {
    HELD.with(std::cell::Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn a_gate_admits_at_most_its_limit() {
        let gate = Arc::new(Gate::new(2));
        let inside = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (gate, inside, most) = (gate.clone(), inside.clone(), most.clone());
                thread::spawn(move || {
                    for _ in 0..5 {
                        let _guard = gate.enter();
                        let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                        most.fetch_max(now, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(2));
                        inside.fetch_sub(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(most.load(Ordering::SeqCst), 2);
        assert_eq!(*gate.active.lock().unwrap(), 0);
    }

    #[test]
    fn a_panicking_holder_releases_its_place() {
        let gate = Arc::new(Gate::new(1));
        let holder = gate.clone();
        assert!(
            thread::spawn(move || {
                let _guard = holder.enter();
                panic!("holder fails inside the gate");
            })
            .join()
            .is_err()
        );
        let (sent, received) = mpsc::channel();
        let waiter = gate.clone();
        thread::spawn(move || {
            let _guard = waiter.enter();
            sent.send(()).unwrap();
        });
        received
            .recv_timeout(Duration::from_secs(10))
            .expect("the place was not released");
    }

    #[test]
    fn an_unlimited_gate_never_blocks_or_counts() {
        let gate = Gate::new(usize::MAX);
        let guards: Vec<_> = (0..64).map(|_| gate.enter()).collect();
        assert_eq!(held(), 0);
        assert_eq!(*gate.active.lock().unwrap(), 0);
        drop(guards);
    }

    #[test]
    fn only_apple_platforms_limit_publications() {
        let expected = if cfg!(target_vendor = "apple") {
            APPLE_LIMIT
        } else {
            usize::MAX
        };
        assert_eq!(PUBLICATIONS.limit, expected);
    }
}

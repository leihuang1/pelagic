//! Thread-scoped worker count; results are bit-identical for every count.
//!
//! Workers only evaluate independent items (a group of modes' bisections, one
//! FIELD row). The calling thread keeps every order-dependent step: results
//! are returned in item order and combined exactly as a single thread would.
use std::cell::Cell;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Workers per call at most, whatever the thread count; no machine gains from
/// more, and the cap bounds thread creation for absurd counts.
const MAX_WORKERS: usize = 1024;

thread_local! {
    static THREADS: Cell<usize> = const { Cell::new(1) };
}

/// Run on this thread allowing solvers to use up to `threads` threads,
/// restoring the previous scope even on unwind. The default is one thread.
pub fn with_threads<T>(threads: NonZeroUsize, operation: impl FnOnce() -> T) -> T {
    struct Restore(usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            THREADS.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(THREADS.with(|slot| slot.replace(threads.get())));
    operation()
}

/// Threads available to solvers in this scope. Work items report one wherever
/// they run, so nested work never multiplies the thread count.
#[must_use]
pub fn threads() -> usize {
    THREADS.with(Cell::get)
}

/// Evaluate `work(0..count)` on up to [`threads`] threads, in item order.
/// Workers inherit the caller's cancellation token.
pub(crate) fn map<T: Send>(count: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let workers = threads().min(count).min(MAX_WORKERS);
    if workers <= 1 {
        return (0..count).map(work).collect();
    }
    let token = crate::cancellation::current();
    let next = AtomicUsize::new(0);
    let drain = || {
        let mut done = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            if index >= count {
                return done;
            }
            done.push((index, work(index)));
        }
    };
    let mut slots: Vec<Option<T>> = std::iter::repeat_with(|| None).take(count).collect();
    std::thread::scope(|scope| {
        // A thread the system refuses leaves its share to the others.
        let handles: Vec<_> = (1..workers)
            .map_while(|_| {
                let token = token.clone();
                std::thread::Builder::new()
                    .spawn_scoped(scope, || match token {
                        Some(token) => crate::cancellation::with_token(token, drain),
                        None => drain(),
                    })
                    .ok()
            })
            .collect();
        let mut place = |done: Vec<(usize, T)>| {
            for (index, value) in done {
                slots[index] = Some(value);
            }
        };
        place(with_threads(NonZeroUsize::MIN, drain));
        for handle in handles {
            place(
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            );
        }
    });
    slots
        .into_iter()
        .map(|slot| slot.expect("every item is evaluated once"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_keeps_item_order_and_restores_the_scope() {
        let four = NonZeroUsize::new(4).unwrap();
        let squares = with_threads(four, || {
            assert_eq!(threads(), 4);
            map(1000, |i| {
                assert_eq!(threads(), 1, "workers never fan out again");
                i * i
            })
        });
        assert_eq!(squares, (0..1000).map(|i| i * i).collect::<Vec<_>>());
        assert_eq!(threads(), 1);
    }

    #[test]
    fn workers_inherit_cancellation() {
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let seen = crate::cancellation::with_token(token, || {
            with_threads(NonZeroUsize::new(3).unwrap(), || {
                map(64, |_| crate::cancellation::cancelled())
            })
        });
        assert!(seen.into_iter().all(|cancelled| cancelled));
    }
}

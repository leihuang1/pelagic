//! Thread-scoped worker count; results are bit-identical for every count.
//!
//! Workers only evaluate independent items (one ray and the influence it
//! records). The calling thread keeps every order-dependent step: results are
//! returned in item order and applied exactly as a single thread would.
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

/// Evaluate `work(0..count)` in item order on up to [`threads`] threads until
/// the evaluated items' `cost` reaches `budget`, and return that prefix. The
/// prefix exceeds the budget by at most one item per thread, and holds at least
/// one item when `count > 0`. Workers inherit the caller's cancellation token.
pub(crate) fn map_prefix<T: Send>(
    count: usize,
    budget: usize,
    cost: impl Fn(&T) -> usize + Sync,
    work: impl Fn(usize) -> T + Sync,
) -> Vec<T> {
    let workers = threads().min(count).min(MAX_WORKERS);
    let next = AtomicUsize::new(0);
    let spent = AtomicUsize::new(0);
    let drain = || {
        let mut done = Vec::new();
        // Item 0 is always claimed, so the prefix is never empty.
        while next.load(Ordering::Relaxed) == 0 || spent.load(Ordering::Relaxed) < budget {
            let index = next.fetch_add(1, Ordering::Relaxed);
            if index >= count {
                break;
            }
            let value = work(index);
            spent.fetch_add(cost(&value), Ordering::Relaxed);
            done.push((index, value));
        }
        done
    };
    if workers <= 1 {
        let done = with_threads(NonZeroUsize::MIN, drain);
        return done.into_iter().map(|(_, value)| value).collect();
    }
    let token = crate::cancellation::current();
    let mut done = std::thread::scope(|scope| {
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
        let mut done = with_threads(NonZeroUsize::MIN, drain);
        for handle in handles {
            done.extend(
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            );
        }
        done
    });
    // Indices are claimed in order and every claim is evaluated: a prefix.
    done.sort_unstable_by_key(|&(index, _)| index);
    done.into_iter().map(|(_, value)| value).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_prefix_keeps_item_order_and_restores_the_scope() {
        let four = NonZeroUsize::new(4).unwrap();
        let squares = with_threads(four, || {
            assert_eq!(threads(), 4);
            map_prefix(
                1000,
                usize::MAX,
                |_| 0,
                |i| {
                    assert_eq!(threads(), 1, "work items never fan out again");
                    i * i
                },
            )
        });
        assert_eq!(squares, (0..1000).map(|i| i * i).collect::<Vec<_>>());
        assert_eq!(threads(), 1);
    }

    #[test]
    fn map_prefix_stops_near_the_budget() {
        for threads in [1, 3] {
            let prefix = with_threads(NonZeroUsize::new(threads).unwrap(), || {
                map_prefix(100, 10, |_| 1, |i| i)
            });
            assert!(
                (10..10 + threads).contains(&prefix.len()),
                "{threads}: {prefix:?}"
            );
            assert_eq!(prefix, (0..prefix.len()).collect::<Vec<_>>());
            let first = with_threads(NonZeroUsize::new(threads).unwrap(), || {
                map_prefix(100, 0, |_| 1, |i| i)
            });
            assert!((1..=threads).contains(&first.len()), "{threads}: {first:?}");
        }
    }

    #[test]
    fn workers_inherit_cancellation() {
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let seen = crate::cancellation::with_token(token, || {
            with_threads(NonZeroUsize::new(3).unwrap(), || {
                map_prefix(64, usize::MAX, |_| 0, |_| crate::cancellation::cancelled())
            })
        });
        assert!(seen.into_iter().all(|cancelled| cancelled));
    }
}

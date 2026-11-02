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
pub(crate) const MAX_WORKERS: usize = 1024;

/// A map whose items total no more work than this stays on the calling thread,
/// where starting threads would cost more than they save. The unit is one mesh
/// row per lane or one modal term, so this is a scale, not a duration.
const INLINE_WORK: usize = 1 << 15;

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

fn worker_count(count: usize, estimated_work: usize) -> usize {
    if estimated_work.max(count) <= INLINE_WORK {
        1
    } else {
        threads().min(count).min(MAX_WORKERS)
    }
}

/// Evaluate `work(0..count)` on up to [`threads`] threads, in item order.
/// `estimated_work` is the saturating total across all items. Items report one
/// thread even on the calling thread, and workers inherit its cancellation token.
pub(crate) fn map<T: Send>(
    count: usize,
    estimated_work: usize,
    work: impl Fn(usize) -> T + Sync,
) -> Vec<T> {
    let workers = worker_count(count, estimated_work);
    if workers <= 1 {
        return with_threads(NonZeroUsize::MIN, || (0..count).map(work).collect());
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
        for estimated_work in [INLINE_WORK, INLINE_WORK + 1] {
            let squares = with_threads(four, || {
                assert_eq!(threads(), 4);
                let squares = map(1000, estimated_work, |i| {
                    assert_eq!(threads(), 1, "workers never fan out again");
                    i * i
                });
                assert_eq!(threads(), 4);
                squares
            });
            assert_eq!(squares, (0..1000).map(|i| i * i).collect::<Vec<_>>());
        }
        assert_eq!(threads(), 1);
    }

    #[test]
    fn dispatch_uses_work_not_just_item_count() {
        with_threads(NonZeroUsize::new(4).unwrap(), || {
            assert_eq!(worker_count(2, INLINE_WORK), 1);
            assert_eq!(worker_count(2, INLINE_WORK + 1), 2);
            assert_eq!(worker_count(2, usize::MAX), 2);
            assert_eq!(worker_count(INLINE_WORK + 1, 0), 4);
            assert_eq!(worker_count(1, usize::MAX), 1);
            assert_eq!(map::<()>(0, 0, |_| panic!("empty map")), [] as [(); 0]);
        });
        with_threads(NonZeroUsize::MAX, || {
            assert_eq!(worker_count(usize::MAX, usize::MAX), MAX_WORKERS);
        });
    }

    #[test]
    fn inline_work_and_heavy_nested_work_stay_on_the_caller() {
        let caller = std::thread::current().id();
        with_threads(NonZeroUsize::new(4).unwrap(), || {
            for (count, estimated_work) in [(3, INLINE_WORK), (1, usize::MAX)] {
                let rows = map(count, estimated_work, |index| {
                    assert_eq!(threads(), 1);
                    assert_eq!(std::thread::current().id(), caller);
                    map(2, usize::MAX, |nested| {
                        assert_eq!(threads(), 1);
                        assert_eq!(std::thread::current().id(), caller);
                        (index, nested)
                    })
                });
                assert_eq!(rows.len(), count);
                assert_eq!(threads(), 4);
            }
        });
        assert_eq!(threads(), 1);
    }

    #[test]
    fn inline_panic_restores_threads_and_cancellation_scope() {
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        crate::cancellation::with_token(token.clone(), || {
            with_threads(NonZeroUsize::new(4).unwrap(), || {
                let panic = std::panic::catch_unwind(|| {
                    map(2, INLINE_WORK, |_| {
                        assert_eq!(threads(), 1);
                        panic!("inline work failed");
                    })
                });
                assert!(panic.is_err());
                assert_eq!(threads(), 4);
                assert!(std::sync::Arc::ptr_eq(
                    &crate::cancellation::current().unwrap(),
                    &token,
                ));
            });
        });
        assert_eq!(threads(), 1);
        assert!(crate::cancellation::current().is_none());
    }

    #[test]
    fn map_preserves_first_error_in_item_order() {
        with_threads(NonZeroUsize::new(4).unwrap(), || {
            for estimated_work in [INLINE_WORK, INLINE_WORK + 1] {
                let results = map(8, estimated_work, |index| {
                    if index == 2 || index == 5 {
                        Err(index)
                    } else {
                        Ok(index)
                    }
                });
                assert_eq!(results.into_iter().collect::<Result<Vec<_>, _>>(), Err(2));
            }
        });
    }

    #[test]
    fn workers_inherit_cancellation() {
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        for estimated_work in [INLINE_WORK, INLINE_WORK + 1] {
            let seen = crate::cancellation::with_token(token.clone(), || {
                with_threads(NonZeroUsize::new(3).unwrap(), || {
                    map(64, estimated_work, |_| {
                        assert!(std::sync::Arc::ptr_eq(
                            &crate::cancellation::current().unwrap(),
                            &token,
                        ));
                        crate::cancellation::cancelled()
                    })
                })
            });
            assert!(seen.into_iter().all(|cancelled| cancelled));
        }
        assert!(crate::cancellation::current().is_none());
    }
}

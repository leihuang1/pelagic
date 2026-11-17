//! Thread-scoped budget; results are bit-identical for every count.
//!
//! Maps evaluate independent items and return them in order. Their workers are
//! leaves, even when work stays inline. Only solve windows lend capacity: each
//! worker reserves an equal share, then lends it when it runs out of solves.
//! A running solve atomically claims spare capacity for a map or nested window.
//! The map's caller can claim more between items; its workers remain leaves.
//! One loan holds all claims until the scoped workers join, including on unwind.
//! Nested windows return their loan to the parent when they end. Numerical
//! consumption stays in item order, exactly as with one thread.
use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Workers per call at most, whatever the thread count; no machine gains from
/// more, and the cap bounds thread creation for absurd counts.
pub(crate) const MAX_WORKERS: usize = 1024;

/// A map whose items total no more work than this stays on the calling thread,
/// where starting threads would cost more than they save. The unit is one mesh
/// row per lane or one modal term, so this is a scale, not a duration.
const INLINE_WORK: usize = 1 << 15;

/// A solve window's threads: its whole budget, and those no worker holds now.
struct Pool {
    available: AtomicUsize,
    budget: usize,
}

/// This thread's own threads, and the window pool it may claim more from.
struct Budget {
    owned: usize,
    lendable: Option<Arc<Pool>>,
}

thread_local! {
    static BUDGET: RefCell<Budget> = const { RefCell::new(Budget { owned: 1, lendable: None }) };
}

fn with_budget<T>(budget: Budget, operation: impl FnOnce() -> T) -> T {
    struct Restore(Budget);
    impl Drop for Restore {
        fn drop(&mut self) {
            BUDGET.with(|slot| std::mem::swap(&mut *slot.borrow_mut(), &mut self.0));
        }
    }
    let _restore = Restore(BUDGET.with(|slot| slot.replace(budget)));
    operation()
}

/// Run on this thread allowing solvers to use up to `threads` threads,
/// restoring the previous scope even on unwind. The default is one thread.
pub fn with_threads<T>(threads: NonZeroUsize, operation: impl FnOnce() -> T) -> T {
    with_budget(
        Budget {
            owned: threads.get(),
            lendable: None,
        },
        operation,
    )
}

/// Threads available to solvers in this scope. Solve-window items include
/// currently lendable threads, which may change before a map claims them.
/// Ordinary map items report one, so leaf work never fans out again.
#[must_use]
pub fn threads() -> usize {
    BUDGET.with(|slot| {
        let budget = slot.borrow();
        budget.owned
            + budget
                .lendable
                .as_ref()
                .map_or(0, |pool| pool.available.load(Ordering::Relaxed))
    })
}

/// The most threads this scope can use: inside a solve window, the window's
/// whole budget, since loans can arrive after a map starts; otherwise [`threads`].
pub(crate) fn thread_limit() -> usize {
    BUDGET.with(|slot| {
        let budget = slot.borrow();
        budget
            .lendable
            .as_ref()
            .map_or(budget.owned, |pool| pool.budget)
    })
}

struct Loan {
    pool: Arc<Pool>,
    count: usize,
}

impl Loan {
    #[allow(deprecated)] // try_update is newer than our MSRV.
    fn claim_more(&mut self, wanted: usize) -> usize {
        let available = self
            .pool
            .available
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |free| {
                Some(free.saturating_sub(wanted))
            })
            .expect("the update always succeeds");
        let claimed = available.min(wanted);
        self.count += claimed;
        claimed
    }
}

impl Drop for Loan {
    fn drop(&mut self) {
        self.pool.available.fetch_add(self.count, Ordering::Relaxed);
    }
}

fn claim_threads(limit: usize) -> (usize, Option<Loan>) {
    BUDGET.with(|slot| {
        let budget = slot.borrow();
        let owned = budget.owned.min(limit);
        let loan = budget
            .lendable
            .as_ref()
            .filter(|_| owned < limit)
            .map(|pool| {
                let mut loan = Loan {
                    pool: pool.clone(),
                    count: 0,
                };
                loan.claim_more(limit - owned);
                loan
            });
        (owned + loan.as_ref().map_or(0, |loan| loan.count), loan)
    })
}

fn worker_limit(count: usize, estimated_work: usize) -> usize {
    if estimated_work.max(count) <= INLINE_WORK {
        1
    } else {
        thread_limit().min(count).min(MAX_WORKERS)
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
    map_init(count, estimated_work, || (), |(), index| work(index))
}

/// Like `map`, with scratch owned by each worker and reused between its items.
/// Scratch never crosses threads and is dropped when this map returns.
pub(crate) fn map_init<T: Send, S>(
    count: usize,
    estimated_work: usize,
    init: impl Fn() -> S + Sync,
    work: impl Fn(&mut S, usize) -> T + Sync,
) -> Vec<T> {
    // The loan outlives the scope, so unwind joins workers before returning it.
    let (workers, mut loan) = claim_threads(worker_limit(count, estimated_work));
    if workers <= 1 && loan.is_none() {
        return with_threads(NonZeroUsize::MIN, || {
            let mut scratch = init();
            (0..count).map(|index| work(&mut scratch, index)).collect()
        });
    }
    map_workers(count, workers, None, loan.as_mut(), init, work)
}

fn map_workers<T: Send, S>(
    count: usize,
    workers: usize,
    lendable: Option<&Arc<Pool>>,
    mut loan: Option<&mut Loan>,
    init: impl Fn() -> S + Sync,
    work: impl Fn(&mut S, usize) -> T + Sync,
) -> Vec<T> {
    let token = crate::cancellation::current();
    let next = AtomicUsize::new(0);
    // Equal reservations keep balanced solves from starving each other's next map.
    let reservation = lendable.map_or(1, |pool| pool.budget / workers);
    let drain = |before_item: &mut dyn FnMut()| {
        let _lend = lendable.map(|pool| Loan {
            pool: pool.clone(),
            count: reservation,
        });
        with_budget(
            Budget {
                owned: reservation,
                lendable: lendable.cloned(),
            },
            || {
                let mut scratch = init();
                let mut done = Vec::new();
                loop {
                    before_item();
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= count {
                        return done;
                    }
                    done.push((index, work(&mut scratch, index)));
                }
            },
        )
    };
    let mut slots: Vec<Option<T>> = std::iter::repeat_with(|| None).take(count).collect();
    let owned = workers - loan.as_ref().map_or(0, |loan| loan.count);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        let mut spawn = |count| {
            for _ in 0..count {
                let token = token.clone();
                let Ok(handle) = std::thread::Builder::new().spawn_scoped(scope, || {
                    let run = || drain(&mut || {});
                    match token {
                        Some(token) => crate::cancellation::with_token(token, run),
                        None => run(),
                    }
                }) else {
                    break;
                };
                handles.push(handle);
            }
            handles.len()
        };
        let started = spawn(workers - 1);
        if let Some(pool) = lendable {
            pool.available
                .fetch_add((workers - 1 - started) * reservation, Ordering::Relaxed);
        }
        let mut claim_between_items = || {
            if let Some(loan) = &mut loan
                && owned + loan.count < count.min(MAX_WORKERS)
                && next.load(Ordering::Relaxed) < count
                && loan.pool.available.load(Ordering::Relaxed) > 0
            {
                spawn(loan.claim_more(count.min(MAX_WORKERS) - owned - loan.count));
            }
        };
        let mut place = |done: Vec<(usize, T)>| {
            for (index, value) in done {
                slots[index] = Some(value);
            }
        };
        place(drain(&mut claim_between_items));
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

/// Evaluate independent solves in order, lending idle workers to running solves.
pub(crate) fn map_window<T: Send>(count: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    // A lone solve keeps access to its enclosing pool as siblings finish.
    if count <= 1 || threads() == 1 {
        let run = || (0..count).map(work).collect();
        return if threads() > MAX_WORKERS {
            with_threads(NonZeroUsize::new(MAX_WORKERS).unwrap(), run)
        } else {
            run()
        };
    }
    let (budget, _loan) = claim_threads(MAX_WORKERS);
    let workers = budget.min(count);
    let pool = Arc::new(Pool {
        available: AtomicUsize::new(budget % workers),
        budget,
    });
    map_workers(
        count,
        workers,
        Some(&pool),
        None,
        || (),
        |(), index| work(index),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn wait_for_threads(expected: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while threads() != expected {
            assert!(
                std::time::Instant::now() < deadline,
                "expected {expected} threads, got {}",
                threads()
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn a_later_map_uses_a_finished_siblings_thread() {
        with_threads(NonZeroUsize::new(2).unwrap(), || {
            let started = std::sync::Barrier::new(2);
            let results = map_window(2, |index| {
                assert_eq!(threads(), 1);
                started.wait();
                if index == 0 {
                    return Vec::new();
                }
                wait_for_threads(2);
                let running = std::sync::Barrier::new(2);
                map(2, usize::MAX, |_| {
                    assert_eq!(threads(), 1);
                    running.wait();
                    std::thread::current().id()
                })
            });
            assert_eq!(results[0], [] as [std::thread::ThreadId; 0]);
            assert_ne!(results[1][0], results[1][1]);
            assert_eq!(threads(), 2);
        });
    }

    #[test]
    fn a_running_map_claims_later_loans_between_items_and_returns_them_on_panic() {
        for panic in [false, true] {
            with_threads(NonZeroUsize::new(2).unwrap(), || {
                let finish_sibling = std::sync::Barrier::new(2);
                let running = std::sync::Barrier::new(2);
                let pool = std::sync::Mutex::new(None);
                let result = std::panic::catch_unwind(|| {
                    map_window(2, |index| {
                        if index == 0 {
                            finish_sibling.wait();
                            return;
                        }
                        let borrowed = BUDGET.with(|slot| slot.borrow().lendable.clone().unwrap());
                        *pool.lock().unwrap() = Some(borrowed.clone());
                        map_init(
                            3,
                            usize::MAX,
                            || (),
                            |(), item| {
                                assert_eq!(threads(), 1);
                                if item == 0 {
                                    finish_sibling.wait();
                                    let deadline = std::time::Instant::now()
                                        + std::time::Duration::from_secs(5);
                                    while borrowed.available.load(Ordering::Relaxed) == 0 {
                                        assert!(std::time::Instant::now() < deadline);
                                        std::thread::sleep(std::time::Duration::from_millis(1));
                                    }
                                } else {
                                    running.wait();
                                    assert!(!panic || item != 2, "late worker failed");
                                }
                            },
                        );
                    })
                });
                assert_eq!(result.is_err(), panic);
                assert_eq!(
                    pool.lock()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .available
                        .load(Ordering::Relaxed),
                    2
                );
                assert_eq!(threads(), 2);
            });
        }
    }

    #[test]
    fn claims_return_after_maps_and_nested_windows_even_on_unwind() {
        let pool = Arc::new(Pool {
            available: AtomicUsize::new(3),
            budget: 4,
        });
        with_budget(
            Budget {
                owned: 1,
                lendable: Some(pool.clone()),
            },
            || {
                let (_, claim) = claim_threads(4);
                assert_eq!(threads(), 1);
                assert_eq!(thread_limit(), 4, "later loans can still arrive");
                assert_eq!(worker_limit(4, usize::MAX), 4);
                drop(claim);
                for panic in [false, true] {
                    for window in [false, true] {
                        let result = std::panic::catch_unwind(|| {
                            let work = |index| {
                                if window {
                                    assert_eq!(pool.available.load(Ordering::Relaxed), 0);
                                    map(4, usize::MAX, |_| assert_eq!(threads(), 1));
                                } else {
                                    assert_eq!(threads(), 1);
                                }
                                assert!(!panic || index != 1, "work failed");
                                index
                            };
                            if window {
                                map_window(2, work)
                            } else {
                                map_init(4, usize::MAX, || (), |(), index| work(index))
                            }
                        });
                        assert_eq!(result.is_err(), panic);
                        if let Ok(values) = result {
                            assert_eq!(values, (0..if window { 2 } else { 4 }).collect::<Vec<_>>());
                        }
                        assert_eq!(pool.available.load(Ordering::Relaxed), 3);
                        assert_eq!(threads(), 4);
                    }
                }
                let panic = std::panic::catch_unwind(|| {
                    map_init(
                        4,
                        usize::MAX,
                        || panic!("scratch failed"),
                        |(): &mut (), _| (),
                    )
                });
                assert!(panic.is_err());
                assert_eq!(pool.available.load(Ordering::Relaxed), 3);
                assert_eq!(threads(), 4);
            },
        );
        assert_eq!(threads(), 1);
    }

    #[test]
    fn ordinary_map_items_neither_lend_nor_claim() {
        let pool = Arc::new(Pool {
            available: AtomicUsize::new(3),
            budget: 4,
        });
        with_budget(
            Budget {
                owned: 1,
                lendable: Some(pool.clone()),
            },
            || {
                for work in [INLINE_WORK, usize::MAX] {
                    map(2, work, |_| {
                        assert_eq!(threads(), 1);
                        assert_eq!(thread_limit(), 1);
                        assert_eq!(
                            pool.available.load(Ordering::Relaxed),
                            if work == INLINE_WORK { 3 } else { 2 }
                        );
                        let caller = std::thread::current().id();
                        map(4, usize::MAX, |_| {
                            assert_eq!(threads(), 1);
                            assert_eq!(std::thread::current().id(), caller);
                        });
                    });
                    assert_eq!(pool.available.load(Ordering::Relaxed), 3);
                }
            },
        );
    }

    #[test]
    fn nested_windows_and_claiming_maps_never_exceed_the_live_worker_budget() {
        struct Live<'a>(&'a AtomicUsize);
        impl Drop for Live<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        for budget in [2, 3, 4, 8, 14] {
            let live = AtomicUsize::new(0);
            let high = AtomicUsize::new(0);
            let enter = || {
                let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                high.fetch_max(now, Ordering::SeqCst);
                assert!(now <= budget, "{now} live workers exceed {budget}");
                Live(&live)
            };
            let started = std::sync::Barrier::new(budget);
            with_threads(NonZeroUsize::new(budget).unwrap(), || {
                map_window(budget * 3, |index| {
                    {
                        let _live = enter();
                        if index < budget {
                            started.wait();
                        }
                    }
                    map_window(3, |_| {
                        {
                            let _live = enter();
                        }
                        map_init(
                            8,
                            usize::MAX,
                            || (),
                            |(), _| {
                                let _live = enter();
                                assert_eq!(threads(), 1);
                                std::thread::sleep(std::time::Duration::from_micros(50));
                            },
                        );
                        let _live = enter();
                    });
                    let _live = enter();
                });
            });
            assert_eq!(live.load(Ordering::SeqCst), 0);
            assert_eq!(high.load(Ordering::SeqCst), budget);
        }
    }

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
    fn map_init_reuses_worker_owned_scratch() {
        for thread_count in [1, 4] {
            for estimated_work in [INLINE_WORK, INLINE_WORK + 1] {
                let initialized = AtomicUsize::new(0);
                let results = with_threads(NonZeroUsize::new(thread_count).unwrap(), || {
                    map_init(
                        1000,
                        estimated_work,
                        || {
                            initialized.fetch_add(1, Ordering::Relaxed);
                            // Not Send: initialized and consumed on its owning worker.
                            std::rc::Rc::new(Cell::new(0))
                        },
                        |scratch, index| {
                            assert_eq!(threads(), 1);
                            scratch.set(scratch.get() + 1);
                            (index, std::thread::current().id(), scratch.get())
                        },
                    )
                });
                let mut counts = std::collections::HashMap::new();
                for (expected, (index, worker, used)) in results.into_iter().enumerate() {
                    assert_eq!(index, expected);
                    let count = counts.entry(worker).or_insert(0);
                    *count += 1;
                    assert_eq!(used, *count);
                }
                assert_eq!(counts.values().sum::<usize>(), 1000);
                assert!(initialized.load(Ordering::Relaxed) <= thread_count);
            }
        }
    }

    #[test]
    fn dispatch_uses_work_not_just_item_count() {
        with_threads(NonZeroUsize::new(4).unwrap(), || {
            assert_eq!(worker_limit(2, INLINE_WORK), 1);
            assert_eq!(worker_limit(2, INLINE_WORK + 1), 2);
            assert_eq!(worker_limit(2, usize::MAX), 2);
            assert_eq!(worker_limit(INLINE_WORK + 1, 0), 4);
            assert_eq!(worker_limit(1, usize::MAX), 1);
            assert_eq!(map::<()>(0, 0, |_| panic!("empty map")), [] as [(); 0]);
        });
        with_threads(NonZeroUsize::MAX, || {
            assert_eq!(worker_limit(usize::MAX, usize::MAX), MAX_WORKERS);
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

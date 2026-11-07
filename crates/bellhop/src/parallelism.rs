//! Thread-scoped worker count; results are bit-identical for every count.
//!
//! Workers only evaluate independent items (one ray and the influence it
//! records). The calling thread keeps every order-dependent step: it applies
//! results in item order, exactly as a single thread would.
use std::cell::Cell;
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

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

/// Evaluate `work(0..count)` on up to [`threads`] threads and pass each result,
/// with its index, to `apply` on the calling thread in item order. The first
/// error from `apply` stops the sequence and is returned.
///
/// Workers stay up for the whole sequence, and the calling thread evaluates
/// items too whenever none is ready to apply. No item is evaluated `window` or
/// more items ahead of the next to apply, and none is started while evaluated
/// items waiting to be applied hold `budget` or more by `cost`, so they exceed
/// it by at most one item per thread. Workers inherit the caller's
/// cancellation token, and a worker's panic resumes on the calling thread.
pub(crate) fn for_each_ordered<T: Send, E>(
    count: usize,
    window: usize,
    budget: usize,
    cost: impl Fn(&T) -> usize + Sync,
    work: impl Fn(usize) -> T + Sync,
    mut apply: impl FnMut(usize, T) -> Result<(), E>,
) -> Result<(), E> {
    let workers = threads().min(count).min(MAX_WORKERS);
    if workers <= 1 {
        for index in 0..count {
            apply(index, with_threads(NonZeroUsize::MIN, || work(index)))?;
        }
        return Ok(());
    }
    let pipeline = Pipeline {
        count,
        window: window.max(1),
        budget,
        cost,
        work,
        queue: Mutex::new(Queue {
            next_claim: 0,
            next_apply: 0,
            held: 0,
            evaluated: VecDeque::new(),
            stopped: false,
        }),
        changed: Condvar::new(),
    };
    let token = crate::cancellation::current();
    std::thread::scope(|scope| {
        // A thread the system refuses leaves its share to the others.
        let handles: Vec<_> = (1..workers)
            .map_while(|_| {
                let token = token.clone();
                let pipeline = &pipeline;
                std::thread::Builder::new()
                    .spawn_scoped(scope, move || match token {
                        Some(token) => crate::cancellation::with_token(token, || pipeline.serve()),
                        None => pipeline.serve(),
                    })
                    .ok()
            })
            .collect();
        let result = {
            // Stops the workers however driving ends, unwinding included.
            let _stop = Stop {
                pipeline: &pipeline,
                always: true,
            };
            with_threads(NonZeroUsize::MIN, || pipeline.drive(&mut apply))
        };
        for handle in handles {
            if let Err(panic) = handle.join() {
                std::panic::resume_unwind(panic);
            }
        }
        result
    })
}

/// The state [`for_each_ordered`] shares between its threads.
struct Pipeline<T, C, W> {
    count: usize,
    window: usize,
    budget: usize,
    cost: C,
    work: W,
    queue: Mutex<Queue<T>>,
    /// Signalled whenever an item is evaluated or applied, or the run stops.
    changed: Condvar,
}

/// Stops a [`Pipeline`] when dropped: `always`, or only while unwinding.
struct Stop<'a, T, C, W> {
    pipeline: &'a Pipeline<T, C, W>,
    always: bool,
}

impl<T, C, W> Drop for Stop<'_, T, C, W> {
    fn drop(&mut self) {
        if self.always || std::thread::panicking() {
            let mut queue = self
                .pipeline
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            queue.stopped = true;
            drop(queue);
            self.pipeline.changed.notify_all();
        }
    }
}

/// Items from the next to apply to the last claimed.
struct Queue<T> {
    next_claim: usize,
    next_apply: usize,
    /// Total cost of the evaluated items not yet applied.
    held: usize,
    /// Item `next_apply + i` at `i`, once evaluated, with its cost.
    evaluated: VecDeque<Option<(T, usize)>>,
    stopped: bool,
}

impl<T, C: Fn(&T) -> usize, W: Fn(usize) -> T> Pipeline<T, C, W> {
    fn lock(&self) -> MutexGuard<'_, Queue<T>> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, queue: MutexGuard<'a, Queue<T>>) -> MutexGuard<'a, Queue<T>> {
        self.changed
            .wait(queue)
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The next item to evaluate, if the window and budget allow one.
    fn claim(&self, queue: &mut Queue<T>) -> Option<usize> {
        let ahead = queue.next_claim - queue.next_apply;
        let open = !queue.stopped
            && queue.next_claim < self.count
            && ahead < self.window
            && (ahead == 0 || queue.held < self.budget);
        open.then(|| {
            queue.evaluated.push_back(None);
            queue.next_claim += 1;
            queue.next_claim - 1
        })
    }

    fn evaluate(&self, index: usize) {
        let value = (self.work)(index);
        let cost = (self.cost)(&value);
        let mut queue = self.lock();
        let slot = index - queue.next_apply;
        queue.evaluated[slot] = Some((value, cost));
        queue.held += cost;
        drop(queue);
        self.changed.notify_all();
    }

    /// A worker: evaluate items until none is left or the run stops.
    fn serve(&self) {
        // Wakes the calling thread if this worker unwinds.
        let _stop = Stop {
            pipeline: self,
            always: false,
        };
        let mut queue = self.lock();
        while !queue.stopped && queue.next_claim < self.count {
            match self.claim(&mut queue) {
                Some(index) => {
                    drop(queue);
                    self.evaluate(index);
                    queue = self.lock();
                }
                None => queue = self.wait(queue),
            }
        }
    }

    /// The calling thread: apply items in order, evaluating one whenever none
    /// is ready to apply.
    fn drive<E>(&self, apply: &mut impl FnMut(usize, T) -> Result<(), E>) -> Result<(), E> {
        let mut queue = self.lock();
        loop {
            if queue.evaluated.front().is_some_and(Option::is_some) {
                let (value, cost) = queue
                    .evaluated
                    .pop_front()
                    .flatten()
                    .expect("the next item is evaluated");
                let index = queue.next_apply;
                queue.next_apply += 1;
                queue.held -= cost;
                drop(queue);
                self.changed.notify_all();
                apply(index, value)?;
                queue = self.lock();
            } else if queue.next_apply >= self.count || queue.stopped {
                // Stopped with items left only when a worker panicked.
                return Ok(());
            } else if let Some(index) = self.claim(&mut queue) {
                drop(queue);
                self.evaluate(index);
                queue = self.lock();
            } else {
                queue = self.wait(queue);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::panic::AssertUnwindSafe;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    fn collect(threads: usize, count: usize, window: usize, budget: usize) -> Vec<(usize, usize)> {
        let mut applied = Vec::new();
        with_threads(NonZeroUsize::new(threads).unwrap(), || {
            for_each_ordered(
                count,
                window,
                budget,
                |_| 1,
                |i| {
                    assert_eq!(super::threads(), 1, "work items never fan out again");
                    i * i
                },
                |index, value| {
                    applied.push((index, value));
                    Ok::<(), ()>(())
                },
            )
        })
        .unwrap();
        applied
    }

    #[test]
    fn applies_every_item_once_in_order_and_restores_the_scope() {
        for threads in [1, 2, 7] {
            for (window, budget) in [(1, usize::MAX), (3, usize::MAX), (64, 0), (64, 5)] {
                let applied = collect(threads, 500, window, budget);
                assert_eq!(applied, (0..500).map(|i| (i, i * i)).collect::<Vec<_>>());
            }
            assert_eq!(collect(threads, 0, 4, 4), []);
        }
        assert_eq!(threads(), 1);
    }

    #[test]
    fn evaluates_within_the_window_and_budget() {
        // Counting an application lags its removal by one item; in-flight items
        // are not yet held, so the budget is exceeded by up to one per thread.
        for (window, budget, limit) in [(4, usize::MAX, 4 + 1), (64, 3, 3 + 6 + 1)] {
            let applied = AtomicUsize::new(0);
            let most_ahead = AtomicUsize::new(0);
            with_threads(NonZeroUsize::new(6).unwrap(), || {
                for_each_ordered(
                    300,
                    window,
                    budget,
                    |()| 1,
                    |i| {
                        let ahead = i + 1 - applied.load(Ordering::SeqCst);
                        most_ahead.fetch_max(ahead, Ordering::SeqCst);
                        std::thread::yield_now();
                    },
                    |_, ()| {
                        applied.fetch_add(1, Ordering::SeqCst);
                        Ok::<(), ()>(())
                    },
                )
            })
            .unwrap();
            let most_ahead = most_ahead.into_inner();
            assert!(
                (1..=limit).contains(&most_ahead),
                "{window} {budget}: {most_ahead}"
            );
        }
    }

    #[test]
    fn stops_at_the_first_error() {
        for threads in [1, 4] {
            let mut applied = Vec::new();
            let result = with_threads(NonZeroUsize::new(threads).unwrap(), || {
                for_each_ordered(
                    1000,
                    8,
                    usize::MAX,
                    |_| 0,
                    |i| i,
                    |index, value| {
                        if value == 37 {
                            return Err(index);
                        }
                        applied.push(value);
                        Ok(())
                    },
                )
            });
            assert_eq!(result, Err(37));
            assert_eq!(applied, (0..37).collect::<Vec<_>>());
        }
    }

    /// Run `body` on a thread of its own, and fail instead of hanging if the
    /// pipeline never lets it return.
    fn within_a_minute<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(std::panic::catch_unwind(AssertUnwindSafe(body)));
        });
        receiver
            .recv_timeout(Duration::from_secs(60))
            .expect("the pipeline's threads stop within a minute")
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    /// Run `for_each_ordered` on 4 threads, expecting it to panic, and check that
    /// the caller's thread scope survives the unwind.
    fn panic_on_four_threads(run: impl FnOnce() -> Result<(), ()>) -> Box<dyn Any + Send> {
        with_threads(NonZeroUsize::new(4).unwrap(), || {
            let panic = std::panic::catch_unwind(AssertUnwindSafe(run))
                .expect_err("the panic reaches the caller");
            assert_eq!(threads(), 4, "the caller's scope is restored");
            panic
        })
    }

    fn message(panic: &(dyn Any + Send)) -> Option<&str> {
        panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
    }

    #[test]
    fn resumes_a_worker_panic() {
        // Only workers panic, and the caller's own item waits for one to, so a
        // worker always fails an item the caller must then never wait for.
        let panic = within_a_minute(|| {
            let caller = std::thread::current().id();
            let worker_panicked = AtomicBool::new(false);
            panic_on_four_threads(|| {
                for_each_ordered(
                    100,
                    8,
                    usize::MAX,
                    |()| 0,
                    |_| {
                        if std::thread::current().id() == caller {
                            while !worker_panicked.load(Ordering::SeqCst) {
                                std::thread::yield_now();
                            }
                        } else {
                            worker_panicked.store(true, Ordering::SeqCst);
                            panic!("a worker's item");
                        }
                    },
                    |_, ()| Ok(()),
                )
            })
        });
        assert_eq!(message(&*panic), Some("a worker's item"));
    }

    #[test]
    fn resumes_an_apply_panic() {
        // While the caller applies item 37, the workers can claim no further than
        // item 45 of 1000, so they stop only if the caller's panic stops them.
        let panic = within_a_minute(|| {
            panic_on_four_threads(|| {
                for_each_ordered(
                    1000,
                    8,
                    usize::MAX,
                    |_| 0,
                    |i| i,
                    |index, _| {
                        assert!(index != 37, "applying item 37");
                        Ok(())
                    },
                )
            })
        });
        assert_eq!(message(&*panic), Some("applying item 37"));
    }

    #[test]
    fn workers_inherit_cancellation() {
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut seen = Vec::new();
        crate::cancellation::with_token(token, || {
            with_threads(NonZeroUsize::new(3).unwrap(), || {
                for_each_ordered(
                    64,
                    8,
                    usize::MAX,
                    |_| 0,
                    |_| crate::cancellation::cancelled(),
                    |_, cancelled| {
                        seen.push(cancelled);
                        Ok::<(), ()>(())
                    },
                )
            })
        })
        .unwrap();
        assert_eq!(seen.len(), 64);
        assert!(seen.into_iter().all(|cancelled| cancelled));
    }
}

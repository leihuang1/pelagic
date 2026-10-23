//! Thread-scoped cooperative cancellation; ordered frequencies share their original solver state.
use std::cell::RefCell;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

thread_local! {
    static TOKEN: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Run on this thread with a cancellation token, restoring the previous scope even on unwind.
pub fn with_token<T>(token: Arc<AtomicBool>, operation: impl FnOnce() -> T) -> T {
    struct Restore(Option<Arc<AtomicBool>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TOKEN.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let _restore = Restore(TOKEN.with(|slot| slot.replace(Some(token))));
    operation()
}

#[must_use]
pub fn cancelled() -> bool {
    TOKEN.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|t| t.load(Ordering::Relaxed))
    })
}

pub(crate) fn check() -> Result<(), crate::DiagnosticReport> {
    if cancelled() {
        Err(crate::solver::error(
            "KR0304",
            "simulation cancelled",
            "execution",
        ))
    } else {
        Ok(())
    }
}

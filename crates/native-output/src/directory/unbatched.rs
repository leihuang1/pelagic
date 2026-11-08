//! Non-Apple workers sync their own files; there is no cross-case coordinator.
use std::sync::Arc;

/// A publication scope on non-Apple targets without cross-case coordination.
/// Each publication independently syncs its files before installation.
pub struct Batch {
    _workers: super::syncing::Workers,
}

impl Batch {
    /// Create a scope with the same interface as the Apple coordinator. While it lives, sync
    /// helpers leave open files for each of its `workers`.
    #[must_use]
    pub fn new(workers: usize) -> Arc<Self> {
        Arc::new(Self {
            _workers: super::syncing::Workers::register(workers),
        })
    }

    /// Run without cross-case coordination or thread-local publication state.
    #[allow(clippy::unused_self)] // Keep the same calling interface on every platform.
    pub fn run<T>(self: &Arc<Self>, operation: impl FnOnce() -> T) -> T {
        operation()
    }
}

//! In-process test observation only. No runtime input can enable this observer.
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static OBSERVERS: Mutex<Vec<Observer>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    File,
    DriveFlush,
    SharedBarrier,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Event {
    pub path: PathBuf,
    pub kind: Kind,
}

struct Observer {
    root: PathBuf,
    events: Vec<Event>,
}

/// A directory-scoped registration, removed even if a test unwinds.
pub struct Registration(PathBuf);

impl Registration {
    /// Register an absolute directory, using the same spelling as publication paths.
    /// # Panics
    /// Panics for a relative or overlapping registration.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        assert!(root.is_absolute());
        let mut observers = registry();
        if observers
            .iter()
            .any(|o| root.starts_with(&o.root) || o.root.starts_with(root))
        {
            // Release the registry first, so only this test fails.
            drop(observers);
            panic!("overlapping sync observer: {}", root.display());
        }
        observers.push(Observer {
            root: root.into(),
            events: Vec::new(),
        });
        ACTIVE.store(observers.len(), Ordering::Relaxed);
        Self(root.into())
    }

    /// Drain only this registration's records; concurrent tests keep their own.
    #[must_use]
    pub fn take(&self) -> Vec<Event> {
        registry()
            .iter_mut()
            .find(|o| o.root == self.0)
            .map(|o| std::mem::take(&mut o.events))
            .unwrap_or_default()
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut observers = registry();
        observers.retain(|o| o.root != self.0);
        ACTIVE.store(observers.len(), Ordering::Relaxed);
    }
}

/// A test that panics while holding the registry must not abort every later test.
fn registry() -> std::sync::MutexGuard<'static, Vec<Observer>> {
    OBSERVERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn record(path: &Path, kind: Kind) {
    if ACTIVE.load(Ordering::Relaxed) == 0 {
        return;
    }
    let mut observers = registry();
    if let Some(observer) = observers.iter_mut().find(|o| path.starts_with(&o.root)) {
        observer.events.push(Event {
            path: path.into(),
            kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registrations_are_isolated_drained_and_removed_on_unwind() {
        let root = std::env::temp_dir().join(format!("pelagic-observer-{}", std::process::id()));
        let left = Registration::new(&root.join("left"));
        let right = Registration::new(&root.join("right"));
        record(&root.join("left/a"), Kind::File);
        record(&root.join("right/b"), Kind::File);
        record(&root.join("outside"), Kind::File);
        assert_eq!(
            left.take(),
            [Event {
                path: root.join("left/a"),
                kind: Kind::File
            }]
        );
        assert_eq!(left.take(), []);
        assert_eq!(
            right.take(),
            [Event {
                path: root.join("right/b"),
                kind: Kind::File
            }]
        );
        let _ = std::panic::catch_unwind(move || {
            let _registration = left;
            panic!("test unwind");
        });
        record(&root.join("left/unregistered"), Kind::File);
        let left = Registration::new(&root.join("left"));
        assert_eq!(left.take(), []);
    }

    #[test]
    fn an_overlapping_registration_fails_without_breaking_the_registry() {
        let root =
            std::env::temp_dir().join(format!("pelagic-observer-overlap-{}", std::process::id()));
        let outer = Registration::new(&root);
        for overlapping in [root.clone(), root.join("inner")] {
            assert!(std::panic::catch_unwind(|| Registration::new(&overlapping)).is_err());
        }
        record(&root.join("a"), Kind::File);
        assert_eq!(outer.take().len(), 1);
    }
}

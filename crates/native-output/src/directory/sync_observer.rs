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
    /// Panics for a relative or overlapping registration, or a poisoned registry.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        assert!(root.is_absolute());
        let mut observers = OBSERVERS.lock().unwrap();
        assert!(
            observers
                .iter()
                .all(|o| !root.starts_with(&o.root) && !o.root.starts_with(root))
        );
        observers.push(Observer {
            root: root.into(),
            events: Vec::new(),
        });
        ACTIVE.store(observers.len(), Ordering::Relaxed);
        Self(root.into())
    }

    /// Drain only this registration's records; concurrent tests keep their own.
    /// # Panics
    /// Panics if the registry is poisoned.
    #[must_use]
    pub fn take(&self) -> Vec<Event> {
        let mut observers = OBSERVERS.lock().unwrap();
        let observer = observers.iter_mut().find(|o| o.root == self.0).unwrap();
        std::mem::take(&mut observer.events)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut observers = OBSERVERS.lock().unwrap();
        observers.retain(|o| o.root != self.0);
        ACTIVE.store(observers.len(), Ordering::Relaxed);
    }
}

pub(super) fn record(path: &Path, kind: Kind) {
    if ACTIVE.load(Ordering::Relaxed) == 0 {
        return;
    }
    let mut observers = OBSERVERS.lock().unwrap();
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
}

//! Capture publication cleanup warnings without parsing stderr or losing the primary error.
use serde::Serialize;
use std::cell::RefCell;
use std::fmt;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct PublicationWarning {
    pub code: &'static str,
    pub severity: &'static str,
    pub field: &'static str,
    pub message: String,
    pub source: serde_json::Value,
}
impl fmt::Display for PublicationWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "warning[{}]: {} ({})",
            self.code, self.message, self.field
        )
    }
}
thread_local! {
    static WARNINGS: RefCell<Option<Vec<PublicationWarning>>> = const { RefCell::new(None) };
}

/// Capture warnings on this thread while retaining the operation's original error type.
pub fn capture<T>(operation: impl FnOnce() -> T) -> (T, Vec<PublicationWarning>) {
    struct Scope(Option<Vec<PublicationWarning>>);
    impl Drop for Scope {
        fn drop(&mut self) {
            WARNINGS.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let _scope = Scope(WARNINGS.with(|slot| slot.replace(Some(Vec::new()))));
    let result = operation();
    let warnings = WARNINGS.with(|slot| slot.take().unwrap_or_default());
    (result, warnings)
}

/// Record a cleanup failure, or print it for legacy Rust callers without a capture scope.
pub fn cleanup(path: &Path, error: &std::io::Error) {
    let diagnostic = PublicationWarning {
        code: "PG0403",
        severity: "warning",
        field: "output.scratch",
        message: format!("unable to remove owned scratch {}: {error}", path.display()),
        source: serde_json::json!({"path": path.to_string_lossy(), "line": 1, "column": 1}),
    };
    WARNINGS.with(|slot| {
        if let Some(warnings) = slot.borrow_mut().as_mut() {
            warnings.push(diagnostic);
        } else {
            eprintln!("{diagnostic}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_capture_preserves_primary_errors_and_restores_outer_scope() {
        let (result, warnings) = capture(|| {
            let ((), inner) =
                capture(|| cleanup(Path::new("inner"), &std::io::Error::other("test")));
            assert_eq!(inner.len(), 1);
            cleanup(Path::new("outer"), &std::io::Error::other("test"));
            Err::<(), _>("original error")
        });
        assert_eq!(result, Err("original error"));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "PG0403");
        assert_eq!(warnings[0].source["path"], "outer");
    }
}

#![forbid(unsafe_code)]

pub mod cancellation;
pub mod case;
pub mod diagnostic;
pub mod input;
pub mod model;
pub mod result;
pub mod solver;

pub use case::{Case, CaseDefinition};
pub use diagnostic::{Diagnostic, DiagnosticReport, LoadOutcome, Severity};
pub use input::{json, legacy};
pub use model::EnvironmentCase;

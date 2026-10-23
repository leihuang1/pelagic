#![forbid(unsafe_code)]

pub mod cancellation;
pub mod case;
pub mod diagnostic;
pub mod input;
pub mod model;
pub mod result;
pub mod solver;

pub use case::{Case, FieldCase};
pub use diagnostic::{Diagnostic, DiagnosticReport};
pub use input::{json, legacy};
pub use model::*;
pub use result::{ModeSet, NormalMode, PressureField, ProfileSimulationResult, SimulationResult};
pub use solver::{solve, solve_complex_modes, solve_field, solve_frequencies};

pub(crate) use case::{
    MAX_FIELD_SAMPLES, MAX_MESH_POINTS, MAX_MODE_LIMIT, MAX_VECTOR_LENGTH, error,
};
pub(crate) use input::attenuation;

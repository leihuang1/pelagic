//! Explicit, immutable range-independent real-KRAKEN modal reuse.
use crate::{Case, DiagnosticReport, MAX_MODE_LIMIT, ModeSet, ModeSolver, PressureField};

/// Owned modes and the complete validated case that produced them.
///
/// Only receiver ranges and horizontal offsets may change when synthesizing a
/// field. Frequency, source/receiver depths, mode sampling, environment, mesh,
/// convergence controls (including `max_range_m`) and all other options stay
/// frozen. This first API supports real KRAKEN only, not KRAKENC or `FieldCase`.
///
/// Sharing this immutable value across threads is safe. Each query owns its
/// output and scratch tables and uses the caller's thread/cancellation scope.
/// Limits apply per query, not to the aggregate memory of concurrent callers.
/// There is no global cache or retained query scratch.
///
/// ```compile_fail
/// fn mutate(prepared: &mut kraken::PreparedModes) {
///     prepared.modes().modes.clear();
/// }
/// ```
///
/// ```compile_fail
/// fn change_environment(prepared: &mut kraken::PreparedModes) {
///     prepared.case().frequency_hz = 200.0;
/// }
/// ```
#[derive(Debug)]
pub struct PreparedModes {
    case: Case,
    modes: ModeSet,
}

/// Prepare a complete real-KRAKEN modal solution for repeated field queries.
///
/// The input case is cloned once so the prepared value owns its provenance.
/// No pressure field is allocated and no partial value is returned on failure.
///
/// # Errors
/// Returns a validation diagnostic for KRAKENC, or the same modal/cancellation
/// diagnostic as [`super::solve`].
pub fn prepare_modes(case: &Case) -> Result<PreparedModes, DiagnosticReport> {
    crate::cancellation::check()?;
    if case.mode_solver != ModeSolver::Kraken {
        return Err(DiagnosticReport::one(crate::case::error(
            "mode_solver",
            "prepared modes require real KRAKEN input",
        )));
    }
    let modes = super::solve_modes(case, MAX_MODE_LIMIT)?;
    crate::cancellation::check()?;
    Ok(PreparedModes {
        case: case.clone(),
        modes,
    })
}

impl PreparedModes {
    /// The frozen case; its original receiver ranges/offsets are query defaults,
    /// not constraints on subsequent queries.
    #[must_use]
    pub fn case(&self) -> &Case {
        &self.case
    }

    /// The full modal solution, borrowed without copying its eigenfunctions.
    #[must_use]
    pub fn modes(&self) -> &ModeSet {
        &self.modes
    }

    /// Synthesize pressure at new receiver ranges and per-depth offsets.
    ///
    /// Uses exactly the same validation, arithmetic, modal order and bounded
    /// scratch-table sizes as [`super::solve`]. Neither modes nor environment
    /// arrays are cloned. Returned metadata owns the queried ranges/offsets.
    /// A cancelled or failed query leaves this value reusable.
    ///
    /// # Errors
    /// Returns the normal case diagnostics for invalid ranges/offsets, or the
    /// existing field-work-limit, field-arithmetic or cancellation diagnostic.
    pub fn field(
        &self,
        receiver_ranges_m: &[f64],
        receiver_offsets_m: &[f64],
    ) -> Result<PressureField, DiagnosticReport> {
        crate::cancellation::check()?;
        crate::case::validate_field_query(&self.case, receiver_ranges_m, receiver_offsets_m)?;
        super::tabulate_field_query(
            super::FieldQuery {
                case: &self.case,
                receiver_ranges_m,
                receiver_offsets_m,
            },
            &self.modes,
            super::PHASE_TABLE_VALUES,
        )
    }
}

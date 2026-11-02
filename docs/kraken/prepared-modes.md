# Reusing KRAKEN modes from Rust

For repeated range-independent fields with the **same modal input**, prepare
modes once and query different receiver ranges or horizontal offsets. This
avoids repeating the eigenvalue search and mode-shape calculation. It does not
make a single modal calculation faster, and each query still computes its FIELD.
For a one-off field, keep using `solve`: preparation adds ownership and query
validation overhead, so a cold prepare-plus-query can be slower.

This first API is in the repository's Rust `kraken` crate. It does not add a CLI,
Python or MATLAB interface, a global cache, or a persisted mode-file loader.
Rust crate APIs have the stability policy in the
[interface contract](../development/interface-contract.md#public-interfaces).

## Prepare once, query repeatedly

Given a validated `kraken::Case` named `case`:

```rust,ignore
let prepared = kraken::prepare_modes(&case)?;
let original = prepared.field(
    &case.receiver_ranges_m,
    &case.receiver_offsets_m,
)?;
let farther = prepared.field(
    &[1000.0, 2000.0, 4000.0],
    &case.receiver_offsets_m,
)?;
// Borrow the full modal product without copying eigenfunctions.
let modes = prepared.modes();
```

Ranges are metres; offsets are one value per frozen receiver depth. Ranges must
be finite, non-negative and strictly increasing. Offsets must be finite, with
the normal point-source effective-range checks. Existing grid and modal-work
limits still apply. Invalid queries return the same diagnostics as an equivalent
ordinary case, rather than silently adjusting the requested field.

`field` returns an owned `PressureField`, including the actual query geometry,
not another copy of the `ModeSet`. The existing `solve` API and its owned
`SimulationResult` remain unchanged. Both routes use the same field synthesis;
reuse does not reorder modal sums or introduce approximate phases.

## What stays fixed

`PreparedModes` owns a read-only copy of the validated case and its modal
solution. Its accessors return shared references. Only ranges and horizontal
offsets can vary; changing anything else requires preparing a new case:

- frequency, sound-speed and density profiles, boundaries, attenuation and mesh;
- phase-speed bounds, convergence controls and `max_range_m`;
- mode limit, mode-sample depths, source and receiver depths;
- source geometry, source pattern and coherent/incoherent addition.

In particular, `max_range_m` controls eigenvalue convergence: it is not just a
plotting range and is not changed by a field query. Default mode-sample depths
can depend on source and receiver depths, so this API does not permit changing
those depths even when an application regards them as field-only settings.

Only single-case **real KRAKEN** is supported. KRAKENC is rejected; there is no
prepared `FieldCase` or range-dependent profile API. Do not substitute this API
for an ordered frequency or profile sequence with cross-solve continuation.

## Ownership, cancellation and memory

Preparation clones the case once and retains its full modal product. Queries
borrow this state: neither environment arrays nor eigenfunctions are cloned.
Each query allocates its own output and uses the existing bounded scratch
blocks. Query scratch is not retained in the prepared value.

The prepared value can be shared with `Arc`. Each caller must choose its own
`parallelism::with_threads` and `cancellation::with_token` scopes. Limits apply
**per query**, not to all simultaneous callers; bound concurrency and avoid
retaining unnecessary output grids if aggregate memory matters. Dropping the
prepared value releases its modes when the last owner is gone.

Failed preparation returns no partial prepared value. A failed or cancelled
query does not modify the prepared modes; a later query can still use them.

## Runnable example

From the repository root:

```sh
cargo run --locked --release -p kraken --example prepared_modes -- \
  crates/kraken/tests/fixtures/Pekeris.env
```

The [example source](../../crates/kraken/examples/prepared_modes.rs) checks two
queries against complete solves, comparing pressure bits, then prints separate
preparation, first-query and warm-query times. It also contrasts eight complete
solves with preparation plus eight queries, dropping outputs between queries.
These single-pass timings illustrate the cost split; they are not a controlled
benchmark or a promised speedup. Use repeated, interleaved measurements and
include preparation and peak memory when assessing a real workload.

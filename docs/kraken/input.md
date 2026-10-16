# KRAKEN/KRAKENC legacy input and budgets

- Quotes, comments, comma separators, `D` exponents, explicit vectors spanning
  lines, and endpoint-subtabulated vectors (`count ≥ 3`, one or two endpoint
  values followed by `/`) are accepted. Depth vectors use upstream single
  precision; ranges and offsets use double precision. Like `ReadVector`, each
  legacy vector is sorted independently, including receiver offsets.
- Each `N/C/P/S` SSP point occupies one record; `A` has no SSP point records
  and requires an explicit bottom sound speed and density for a fluid bottom.
  Omitted trailing material values retain the previous point's values (Fortran
  defaults on the first). `/` ends that
  record, **not** the SSP; the interface depth ends the SSP. The acoustic bottom
  half-space record may similarly inherit omitted trailing values; a vacuum,
  rigid or tabulated boundary has **no** half-space record. In a direct `CaseDefinition`,
  its bottom sound speed, density, and loss must all be zero (absent material).
- Nonnegative RMS roughness in medium headers and the bottom-options record is
  supported for [fluid-interface scattering](roughness.md). The canonical optional
  `interface_roughness_m` vector contains fluid top, internal interfaces and bottom;
  empty means smooth. Nonzero roughness with solids or tables remains rejected.
- Fortran null slots and repetition syntax remain unsupported and are rejected;
  this is not a general Fortran list-directed reader. All parsed numbers must
  be finite, SSP depths strictly increase between each layer's interfaces, and
  semantic diagnostics retain input-file records.
- Each input file is capped at 1 MiB; vectors at 100,000 entries; frequency
  count at 1,000; finite fluid layers at 500; total SSP nodes and loss values
  each at 100,000; cloned frequency-case input vectors at 5,000,000 values.
  Per frequency, across all layers combined: mesh at 1,000,000 grid intervals; roots at 20,000 modes;
  mode shapes at 5,000,000 values; all KRAKEN mesh searches at 2,500,000,000
  conservative operations and KRAKENC at 300,000,000 counted operations;
  pressure grids at 1,000,000 samples and 550,000,000 modal contributions.
  The larger KRAKEN/FIELD work bounds cover original BroadBand/MunkK at
  500 Hz (2,273,677,857 conservative root operations, 513,035,523 FIELD
  contributions); KRAKENC's spacing predictor uses 152,546,495 root operations
  for that same 500 Hz input, below its unchanged 300M ceiling. Numerical
  tolerances are unchanged. The loader bounds
  cumulative input copies. The CLI additionally bounds cumulative output
  payload/file size (default 256 MiB), solving and writing one frequency at
  a time. Numerical work limits apply to each `solve`, not cumulatively
  across separate calls; the output quota is not a CPU timeout.
- `mesh_points` = 0 selects the reference's automatic base mesh (at least ten,
  ~20 points per wavelength); otherwise 10–1,000,000 is allowed if not too
  coarse. `max_range_m` = 0 uses the base mesh only; larger values control
  the reference-style extrapolation convergence criterion. If limits prevent
  convergence, the solver returns a diagnostic rather than an unverified mode.

Modes are computed in double precision and shapes stored at reference `.mod`
sampling precision. FIELD uses single-precision
wavenumbers, modal products, and accumulation, with separate range/offset phases,
following the reference's `.mod`/FIELD rounding points. Returned pressure values
are promoted to `Complex64`; that does not imply double-precision FIELD arithmetic.

These input limits do not reduce the [fixed support target](compatibility.md).
Measured differences and fixture provenance are recorded
[with the goldens](../../crates/kraken/tests/fixtures/golden/README.md).

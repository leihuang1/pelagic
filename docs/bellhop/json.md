# Self-contained JSON input

Schema version 1 is the modern input format for the CLI and HTTP service. It
represents one complete two-dimensional case in a single document; no
same-stem auxiliary files are resolved for JSON inputs.

See [`examples/field-g.json`](../../examples/field-g.json) for a complete example.
The server's generated OpenAPI document at `/openapi.json` is the exhaustive
machine-readable schema.

## Conventions

- `schema_version` must be `1`.
- Object fields are strict. Unknown fields, missing required fields, and unknown
  enum values are malformed input rather than forward-compatible extensions.
- Enum values use `snake_case`.
- Variant data uses a `type` discriminator, for example
  `{"type":"vacuum"}` and
  `{"type":"acousto_elastic","half_space":{...}}`.
- Distances use metres, speeds metres per second, frequencies hertz, and
  densities kilograms per cubic metre. Attenuation values follow the explicit
  `attenuation_unit` because BELLHOP supports several conventions.
- Geometric angles and reflection-table phases use degrees and are named with a
  `_degrees` suffix.
- Complex values are objects with `real` and `imaginary` fields.
- `Case.source_path`, fixed-width legacy option strings, and legacy output
  encodings are intentionally not serialized. They are transport metadata, not
  simulation inputs.

The document contains inline forms of all legacy auxiliary resources:

| Legacy resource | JSON location |
| --- | --- |
| `.ssp` | `sound_speed.range_dependent` |
| `.ati` | `top_boundary.shape` |
| `.bty` | `bottom_boundary.shape` |
| `.trc` / `.brc` | the boundary condition's `table` |
| `.irc` | bottom `precalculated_internal_reflection.table` |
| `.sbp` | `source_beam_pattern` |

A reflection-table condition therefore looks like:

```json
{
  "roughness_m": 0.0,
  "condition": {
    "type": "reflection_coefficients",
    "table": {
      "points": [
        { "angle_degrees": 0.0, "magnitude": 1.0, "phase_degrees": 0.0 },
        { "angle_degrees": 90.0, "magnitude": 0.0, "phase_degrees": 180.0 }
      ]
    }
  }
}
```

All geometric coordinates are retained at double precision. Arrays that
represent axes must be strictly increasing. A range-dependent sound-speed
matrix is indexed as `[depth_index][range_index]`; its depth axis
must match the base profile. Receiver depths and ranges must have equal lengths
for an `irregular` receiver grid. Beam families are omitted for ray runs and
required for all other run kinds. Cerveny families require `trace.cerveny`.

Legacy `RG` ray runs retain reference-specific internal beam-state initialization
that JSON does not encode. `crates/bellhop/tests/ray_solver.rs` checks actual solver
outcomes for 12 derived inputs, with beam shifts on/off and normal/two-step limits:
exported ray values agree bit-for-bit, and a non-finite reflection reached by the
shifted Munk fan is rejected through both input paths. Only legacy provenance is
excluded from result comparison. This bounded characterization is not a proof for
all extreme inputs; the legacy initialization and finiteness guards are preserved.

## CLI conversion and use

Convert a legacy environment and all of its referenced auxiliary files:

```console
pelagic bellhop export path/to/case.env > case.json
```

Warnings are written to stderr, so redirected stdout remains valid JSON. The
CLI accepts the resulting file anywhere it accepts a legacy `.env` file:

```console
pelagic bellhop validate case.json
pelagic bellhop run case.json --output results/case --format both
```

Exporting an existing JSON case validates and canonicalizes it. The legacy `W`
reflection-table generation option cannot be exported because it has no
complete simulation semantics in the compatibility reference.

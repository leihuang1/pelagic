# History

These records explain how earlier decisions were reached. They are **frozen**:
do not update them to match later code. For what is supported today, use the
[model compatibility pages](../README.md#model-reference). For current numerical
results, use the [coverage records](../README.md#numerical-evidence).

| Record | Subject |
| --- | --- |
| [KRAKEN/KRAKENC implementation checkpoints](kraken-checkpoints.md) | Each implementation slice, its acceptance paths and counts |
| [KRAKEN input-only capability audit](kraken-capability-audit.md) | Input coverage before the numerical fixes |
| [KRAKENC three-layer refinement](krakenc-three-layer-refinement.md) | Diagnosis of one fixed three-layer refinement case |
| [Fluid KRAKEN root search](kraken-fluid-roots.md) | Count-isolated Illinois refinement, exact fallbacks, numerical changes and solver timings |
| [Range-dependent KRAKEN FIELD lanes](kraken-rd-field-lanes.md) | Coupled FIELD sums 16 same-profile ranges at once; an adiabatic prototype was not retained |
| [BELLHOP boundary analysis](bellhop-boundary-analysis.md) | Boundary-step analysis; the repair was deferred |
| [Official 2026 BELLHOP review](bellhop-official-2026-review.md) | Review of upstream 2026 changes; the pinned oracle was kept |
| [BELLHOP performance experiments](bellhop-performance-experiments.md) | Three isolated speedup experiments; inlining the contribution recorder was retained |
| [BELLHOP tracing and scheduling experiments](bellhop-trace-and-scheduling-experiments.md) | Inlined ray steps and a pipelined scheduler were retained; six candidates were rejected |

When an investigation is finished, add its record here, and move any lasting
conclusion into the page that owns that topic (see the table in
[CONTRIBUTING](../../CONTRIBUTING.md#documentation)).

# HTTP service

`pelagic-server` is a stateless, synchronous HTTP facade. Each simulation
request supplies one [self-contained JSON case](../bellhop/json.md) and waits for the
solver. A caller can receive the complete versioned HDF5 result or focused JSON
arrival and pressure-field results. The service does not retain cases or
results.

## Start the service

```console
RUST_LOG=info cargo run -p server --release
```

The default listener is `0.0.0.0:8080`. Generated documentation is public:

- OpenAPI 3 document: `GET /openapi.json`
- Swagger UI: `GET /docs`
- Readiness: `GET /healthz`

The service provides plain HTTP only. Terminate TLS in a reverse proxy or load
balancer when traffic crosses a trusted-network boundary.

## Endpoints

### `POST /v1/validate`

Accepts `application/json`. A valid case returns:

```json
{
  "valid": true,
  "warnings": []
}
```

Validation parses the complete document and checks its semantic consistency;
it does not run the numerical solver.

### `POST /v1/run`

Accepts the same JSON document. Success returns the HDF5 bytes with:

```text
Content-Type: application/x-hdf5
Content-Disposition: attachment; filename="bellhop-result.h5"
```

Example:

```console
curl --fail-with-body \
  -H 'Content-Type: application/json' \
  --data-binary @examples/cases/bellhop/field-g/field-g.json \
  http://localhost:8080/v1/run \
  --output result.h5
```

When authentication is configured, add
`-H "Authorization: Bearer $BELLHOP_AUTH_TOKEN"`.

### `POST /v1/arrivals`

Accepts the same JSON document but requires `bellhop.run` to be `arrivals`.
Success returns `application/json` without creating or parsing an intermediate
HDF5 file:

```json
{
  "schema_version": 1,
  "title": "BELLHOP field influence golden",
  "frequency_hz": 1000.0,
  "warnings": [],
  "sources": [
    {
      "source_depth_m": 50.0,
      "receivers": [
        {
          "range_m": 1000.0,
          "depth_m": 50.0,
          "arrivals": [
            {
              "amplitude": 0.001,
              "phase_radians": 0.0,
              "travel_time_s": 0.6666667,
              "attenuation_time_s": 0.0,
              "source_angle_degrees": 0.0,
              "receiver_angle_degrees": 0.0,
              "top_bounces": 0,
              "bottom_bounces": 0
            }
          ]
        }
      ]
    }
  ]
}
```

The source and receiver nesting preserves the input position axes and supports
both rectilinear and irregular receiver grids. Empty receiver arrival arrays
are retained. Floating-point values use the solver's native precision.

- depths and ranges are metres, with depth positive downward;
- `amplitude` is relative pressure amplitude;
- `phase_radians` is the accumulated reflection/caustic phase;
- travel and attenuation times are seconds;
- source and receiver angles are degrees, positive downward;
- bounce counts correspond to the top and bottom boundaries.

Example using the included field case with its run kind changed to arrivals:

```console
jq '.bellhop.run = "arrivals"' examples/cases/bellhop/field-g/field-g.json |
  curl --fail-with-body \
    -H 'Content-Type: application/json' \
    --data-binary @- \
    http://localhost:8080/v1/arrivals
```

A case with any other run kind returns `422` with error code
`unsupported_run_kind`.

For both JSON result endpoints, simulation, finite-value validation, and
serialization run while the request holds a bounded worker slot. A borrowing
serialization view writes the solver result directly into a size-limited
buffer; it does not materialize a second result object graph. Exceeding
`BELLHOP_MAX_JSON_RESPONSE_BYTES` returns `429`; a non-finite solver value
returns a structured `500` instead of emitting schema-invalid JSON `null`.

### `POST /v1/field`

Accepts the same JSON document and requires `bellhop.run` to be `coherent`,
`semi_coherent`, or `incoherent`. Success returns the solver's relative complex
pressure directly as `application/json`:

```json
{
  "schema_version": 1,
  "title": "BELLHOP field influence golden",
  "frequency_hz": 1000.0,
  "run_kind": "coherent",
  "warnings": [],
  "sources": [
    {
      "source_depth_m": 50.0,
      "receivers": [
        {
          "range_m": 1000.0,
          "depth_m": 50.0,
          "pressure": {
            "real": 0.001,
            "imaginary": -0.002
          }
        }
      ]
    }
  ]
}
```

Source and receiver coordinates are metres, with depth positive downward.
Pressure components are dimensionless single-precision values. Receiver order
matches the input grid: range-major for rectilinear grids and input order for
irregular grids. The same finite-value and response-size protections as
`/v1/arrivals` apply. Large pressure fields should use `/v1/run` and its HDF5
response instead.

```console
curl --fail-with-body \
  -H 'Content-Type: application/json' \
  --data-binary @examples/cases/bellhop/field-g/field-g.json \
  http://localhost:8080/v1/field
```

Other run kinds return `422` with error code `unsupported_run_kind`.

## Errors

Errors use `application/json`:

```json
{
  "error": {
    "code": "invalid_case",
    "message": "request body describes an invalid case",
    "diagnostics": [
      {
        "severity": "error",
        "code": "BH0201",
        "message": "at least one launch angle is required",
        "field": "bellhop.angles",
        "location": { "path": "request.json", "line": 1, "column": 1 }
      }
    ]
  }
}
```

| Status | Meaning |
| --- | --- |
| `400` | malformed JSON, wrong object shape, or unknown field/enum |
| `401` | configured Bearer token is missing or incorrect |
| `413` | request exceeds `BELLHOP_MAX_BODY_BYTES` |
| `415` | `Content-Type` is not JSON |
| `422` | semantic validation, unsupported run option, or simulation failure |
| `429` | a server-controlled simulation or response-size limit was exceeded |
| `500` | worker, HDF5, JSON serialization, or non-finite output failure |
| `504` | queueing plus execution exceeded the request timeout |

## Configuration

Every option is available as a command-line flag and environment variable.
Run `pelagic-server --help` for flag names.

| Environment variable | Default | Purpose |
| --- | ---: | --- |
| `BELLHOP_LISTEN` | `0.0.0.0:8080` | listener address |
| `BELLHOP_WORKERS` | logical CPU count | maximum admitted blocking requests |
| `BELLHOP_REQUEST_TIMEOUT_SECONDS` | `120` | queue plus execution timeout |
| `BELLHOP_MAX_BODY_BYTES` | `16777216` | request-body limit |
| `BELLHOP_MAX_JSON_RESPONSE_BYTES` | `67108864` | JSON response limit |
| `BELLHOP_AUTH_TOKEN` | unset | optional static Bearer token |
| `BELLHOP_MAX_RAYS` | `1000000` | total launch-ray limit |
| `BELLHOP_MAX_STEPS_PER_RAY` | `100000` | per-ray integration limit |
| `BELLHOP_MAX_TOTAL_POINTS` | `20000000` | ray/eigenray point limit |
| `BELLHOP_MAX_ARRIVALS_PER_RECEIVER` | `20000000` | per-receiver arrival limit |
| `BELLHOP_MAX_TOTAL_ARRIVALS` | `20000000` | total arrival limit per request |
| `BELLHOP_MAX_FIELD_CELLS` | `20000000` | pressure-field cell limit |

Logging uses `tracing` and the standard `RUST_LOG` filter; its fallback is
`info`.

CPU work runs in Tokio's blocking pool behind a Tower concurrency limit. A
request timeout returns `504` immediately, but Rust blocking work cannot be
forcefully cancelled: that simulation continues in the background until it
finishes and retains its worker slot. Size, concurrency, and solver limits
remain active. Cooperative solver cancellation
is intentionally deferred to a later version.

The optional token is deployment-wide, not per-user authorization. It is
compared on every `/v1/*` request; health and documentation remain public.

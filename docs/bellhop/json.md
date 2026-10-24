# BELLHOP JSON input

A JSON case is one complete two-dimensional BELLHOP calculation in a single
document, designed to be written by hand. It follows the `.env` file closely,
with names instead of positions:

```json
{
  "pelagic": 2,
  "model": "bellhop",
  "title": "Munk sound channel: coherent transmission loss",
  "frequency": 50,
  "interpolation": "pchip",
  "media": [
    {"to": 5000, "ssp": [[0, 1548.52], [200, 1530.29], [1200, 1500.14], [5000, 1551.91]]}
  ],
  "surface": "vacuum",
  "bottom": {"type": "halfspace", "cp": 1600},
  "source": {"depth": 1000},
  "receivers": {
    "depth": {"from": 0, "to": 5000, "count": 201},
    "range": {"from": 0, "to": 100, "count": 501, "unit": "km"}
  },
  "bellhop": {
    "run": "coherent",
    "beam": "geometric_gaussian_cartesian",
    "angles": {"from": -20, "to": 20},
    "box": {"depth": 5500, "range": 101000}
  }
}
```

Every example case in [`examples/cases/bellhop`](../../examples/cases/bellhop)
has a JSON file next to its `.env`. KRAKEN uses the same structure and names;
see [KRAKEN JSON input](../kraken/json.md).

## Conventions

- **Units are fixed**: metres, m/s, Hz, g/cm³ and degrees. Attenuation uses
  `attenuation.unit`.
- **Defaults** are what the `.env` itself implies. Leave out anything you
  don't need; `export` leaves it out too.
- **Strict**: unknown fields, unknown names and wrong types are errors, reported
  with the JSON path of the field (`bellhop.angles`, `media[0].ssp[3]`).
- **Lists** (depths, ranges, angles) are a number, an array, or a grid generated
  from its end points:
  `{"from": 0, "to": 100, "count": 501, "unit": "km"}`. A grid expands exactly
  like a `.env` vector given by two end points:
  `from + i * (to - from) / (count - 1)`, computed in `unit` (`m` or `km`)
  and then converted to metres.
- **Tables are rows** of numbers: `[depth, cp]`, `[angle, magnitude, phase]`.
  Each table's rows have the same width, so they map to a NumPy array or a
  MATLAB matrix.
- `"vacuum"` and `"rigid"` may be written as names; every other boundary is an
  object with a `"type"`.

## Top level

| Field | Required | Meaning |
| --- | --- | --- |
| `pelagic` | yes | Schema version, `2` |
| `model` | yes | `"bellhop"` |
| `title` | | Text, default empty |
| `frequency` | yes | Hz |
| `interpolation` | yes | `c_linear`, `n2_linear`, `pchip`, `spline`, `quadrilateral` (with `range_dependent`) or `analytic_munk` |
| `attenuation` | | `{"unit", "volume"}`; see below |
| `media` | yes | The water column: exactly one medium |
| `surface`, `bottom` | yes | Boundaries |
| `source` | yes | `{"depth", "geometry", "pattern"}` |
| `receivers` | yes | `{"depth", "range", "grid"}` |
| `bellhop` | yes | Run and beam options |

### `attenuation`

- `unit`: `db_per_wavelength` (default), `db_per_meter_khz`, `db_per_meter`,
  `nepers_per_meter`, `quality_factor` or `loss_parameter`. It applies to every
  `ap` and `as`.
- `volume`: `"none"` (default), `"thorp"`,
  `{"type": "francois_garrison", "temperature": 4, "salinity": 35, "ph": 8, "depth": 1000}`
  or `{"type": "biological", "layers": [[top, bottom, resonance_hz, Q, a0], ...]}`.

### `media`

```json
{"to": 5000, "ssp": [[0, 1548.52], [5000, 1551.91]], "mesh": 0}
```

- `to`: depth of the bottom of the water column.
- `ssp`: rows of `[depth, cp, cs, rho, ap, as]`. Trailing values may be left
  out: the first row defaults to `cs = 0, rho = 1, ap = as = 0`, and every later
  row repeats the row above, exactly as a `/` does in the `.env`. Omit `ssp` for
  `analytic_munk`.
- `mesh`: the `.env` NMESH value, kept for native output. BELLHOP does not use it.
- `range_dependent`: the `.ssp` file, for `quadrilateral` interpolation:
  `{"ranges": [0, 10000, 20000], "c": [[...], ...]}`. `c` has one row per `ssp`
  depth and one column per range.

### `surface` and `bottom`

| `type` | Fields | `.env` |
| --- | --- | --- |
| `vacuum`, `rigid` | | `V`, `R` |
| `halfspace` | `cp` (required), `cs`, `rho` (default 1), `ap`, `as`, `depth` (default: the boundary depth) | `A` |
| `grain_size` | `phi`, `depth` | `G` |
| `reflection` | `table`: rows of `[angle, magnitude, phase]`, in degrees | `F` with `.trc`/`.brc` |
| `precalculated` (bottom only) | `frequency`, `table`: rows of `[k², Re f, Im f, Re g, Im g, power]`, optional `title` | `P` with `.irc` |

Every boundary also takes:

- `roughness`: RMS roughness, m.
- `shape`: the `.ati`/`.bty` file. It is
  `{"interpolation": "linear", "points": [[range, depth], ...]}`, with
  `interpolation` set to `linear` (default) or `curvilinear`. For a bottom whose
  material changes with range, use rows of `[range, depth, cp, cs, rho, ap, as]`.

### `source` and `receivers`

- `source.depth`: a list (see Conventions).
- `source.geometry`: `point` (default) or `line`.
- `source.pattern`: the `.sbp` file, as rows of `[angle, level_dB]`.
- `receivers.depth`, `receivers.range`: lists.
- `receivers.grid`: `rectilinear` (default) or `irregular`. An irregular grid
  pairs each depth with one range.

### `bellhop`

| Field | Meaning |
| --- | --- |
| `run` | `rays`, `eigenrays`, `coherent`, `semi_coherent`, `incoherent` or `arrivals` |
| `beam` | Required for every run except `rays`: `geometric_gaussian_cartesian`, `geometric_gaussian_ray_centered`, `geometric_hat_cartesian`, `geometric_hat_ray_centered`, `simple_gaussian`, `cerveny_cartesian` or `cerveny_ray_centered` |
| `angles` | Launch angles, degrees, positive downward. A grid without `count` uses BELLHOP's automatic count (NBEAMS = 0). A full-circle fan drops its repeated last angle, as the `.env` reader does. |
| `box` | `{"depth", "range"}`, m: rays stop outside it |
| `step` | Step size, m; 0 (default) chooses automatically |
| `beam_shift` | Default `false` |
| `selected_angle` | One-based index of a single angle to trace |
| `cerveny` | Required for Cerveny beams: `width`, `curvature`, `epsilon_multiplier`, `loop_range`, `image_count`, `beam_window`, `component` |

## From `.env` files

```console
pelagic bellhop export path/to/case.env > case.json
pelagic bellhop run case.json --output results/case --format both
```

`export` reads the `.env` and every auxiliary file it uses (`.ssp`, `.ati`, `.bty`,
`.trc`, `.brc`, `.irc`, `.sbp`). It writes the shortest exact document:

- defaults are left out;
- a list becomes a grid when one reproduces every value bit for bit;
- sound-speed rows keep only the columns that hold non-default values.

Warnings go to stderr, so redirected stdout is valid JSON. Exporting a JSON
case validates and normalizes it. The `.env` `W` option (write a reflection
table) cannot be exported.

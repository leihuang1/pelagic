# KRAKEN JSON input

A JSON case is a complete KRAKEN/KRAKENC and FIELD calculation in one document:
the `.env` and `.flp` pair, with names instead of positions. It uses the same
structure and conventions as [BELLHOP JSON input](../bellhop/json.md): fixed
units (m, m/s, Hz, g/cm³, degrees), defaults from the `.env`, lists as grids,
and tables as rows.

```json
{
  "pelagic": 2,
  "model": "kraken",
  "title": "Pekeris waveguide, 50 to 150 Hz",
  "frequency": {"from": 50, "to": 150, "count": 101},
  "interpolation": "n2_linear",
  "media": [{"to": 100, "ssp": [[0, 1500], [100, 1500]]}],
  "surface": "vacuum",
  "bottom": {"type": "halfspace", "cp": 1700, "rho": 1.5, "ap": 0.8},
  "source": {"depth": 25},
  "receivers": {"depth": [25, 50], "range": {"from": 1, "to": 10, "count": 10, "unit": "km"}},
  "kraken": {"solver": "kraken", "phase_speed": [1400, 1700], "max_range": 10000}
}
```

Every example case in [`examples/cases/kraken`](../../examples/cases/kraken) has
a JSON file next to its `.env` and `.flp`.

## Frequencies and losses

`frequency` is one value or a list (any grid), solved in the order given;
repeats are kept. The whole environment is written once.

Losses are stored the way the `.env` states them: a unit for every `ap`/`as`, an
optional volume model, and power-law parameters. Pelagic converts them for each
solve frequency with the same code as `.env` input. Writing `0.8` with the
default `db_per_wavelength` therefore means 0.8 dB/λ at every frequency, while
`db_per_meter_khz` or `"volume": "thorp"` make the loss depend on frequency, as
they do in the Toolbox.

## Top level

| Field | Required | Meaning |
| --- | --- | --- |
| `pelagic` | yes | Schema version, `2` |
| `model` | yes | `"kraken"` |
| `title` | | Text, default empty |
| `frequency` | yes | Hz: a number, list or grid |
| `interpolation` | yes | `n2_linear`, `c_linear`, `pchip`, `spline` or `analytic_munk` |
| `attenuation` | | `{"unit", "volume", "reference_frequency"}` |
| `media` | yes | Finite media from the surface down |
| `surface`, `bottom` | yes | Boundaries |
| `source` | yes | `{"depth", "geometry", "pattern"}` |
| `receivers` | yes | `{"depth", "range", "offset"}` |
| `kraken` | yes | Mode-solver and FIELD options |
| `profiles` | | Further environments, for range-dependent FIELD |

### `attenuation`

- `unit`: `db_per_wavelength` (default), `db_per_meter_khz`, `db_per_meter`,
  `db_per_meter_power_law`, `nepers_per_meter`, `quality_factor` or
  `loss_parameter`.
- `volume`: as for BELLHOP. Use `"none"` (default), `"thorp"`,
  `{"type": "francois_garrison", ...}` or
  `{"type": "biological", "layers": [...]}`.
- `reference_frequency`: a positive frequency, Hz. Required for `db_per_meter_power_law`, whose
  `[beta, transition_frequency]` is given by each medium's `power_law` and by the
  bottom half-space's `power_law`.

### `media`

The `.env` media in order: optional elastic layers above the water (ice), the
water and any further fluids, then elastic sediments. A medium whose first row
has `cs > 0` is elastic.

```json
{"to": 120, "ssp": [[100, 1600, 400, 1.8, 0.2, 0.5], [120]], "mesh": 0, "roughness": 0.2}
```

- `to`: depth of the medium's lower interface.
- `ssp`: rows of `[depth, cp, cs, rho, ap, as]` from the medium's top depth to
  `to`. The first row defaults to `cs = 0, rho = 1, ap = as = 0`, and later rows
  repeat the row above, as a `/` does. A homogeneous layer is a full first row
  and a one-value last row, as above.
- `mesh`: mesh points; 0 (default) chooses automatically.
- `roughness`: RMS roughness of the medium's upper interface, m. The first
  medium's upper interface is the surface, so use `surface.roughness` there.
  The `.env` roughness of each finite medium maps here (see
  [rough fluid interfaces](roughness.md)).
- `power_law`: `[beta, transition_frequency]` for `db_per_meter_power_law`.

### `surface` and `bottom`

| `type` | Fields | `.env` |
| --- | --- | --- |
| `vacuum`, `rigid` | | `V`, `R` |
| `halfspace` | `cp` (required), `cs`, `rho` (default 1), `ap`, `as`; bottom `power_law` | `A` (elastic when `cs > 0`) |
| `reflection` (KRAKENC, single medium) | `table`: rows of `[angle, magnitude, phase]`, degrees | `F` with `.trc`/`.brc` |
| `precalculated` (KRAKENC, bottom, single medium) | `frequency`, `table`: rows of `[k², Re f, Im f, Re g, Im g, power]` | `P` with `.irc` |

Both take `roughness`, m.

### `source` and `receivers`

- `source.depth`, `receivers.depth`: lists. Like the Toolbox, KRAKEN stores
  these depths in single precision. A depth equal to a fluid interface (in single
  precision) is placed exactly on it.
- `source.geometry`: `point` (default), `line` or `scaled_cylindrical`.
- `source.pattern`: the `.sbp` file, as rows of `[angle, level_dB]`.
- `receivers.range`: a list, in metres or as a `km` grid.
- `receivers.offset`: horizontal offset of each receiver depth, m; default 0.

### `kraken`

| Field | Meaning |
| --- | --- |
| `solver` | `kraken` (real) or `krakenc` (complex). `--solver`, if given, must match |
| `phase_speed` | `[low, high]` phase-speed window, m/s |
| `max_range` | Largest range of interest, m |
| `mesh_frequency` | Frequency the meshes are sized for, Hz; default each solve frequency. The `.env` broadband option sets it to each profile's header frequency. A profile sets its own, or `null` for each solve frequency |
| `mode_depths` | Where modes are sampled; default the source and receiver depths |
| `mode_limit` | Most modes FIELD sums; default all |
| `addition` | `coherent` (default) or `incoherent` |
| `propagation` | `adiabatic` or `coupled`; required with `profiles` |

### `profiles`

The top level describes the environment at 0 m. Each entry in `profiles` adds
an environment at a greater `range` (m). An entry replaces whole top-level keys:
`title`, `interpolation`, `attenuation`, `media`, `surface`, `bottom` and, inside
`kraken`, `phase_speed`, `max_range`, `mode_depths` and `mesh_frequency`. Keys it leaves out are
the top level's. Nothing is merged inside a key, so a profile that changes one
medium repeats the full `media` list.

```json
"kraken": {"solver": "kraken", "phase_speed": [0, 1600], "max_range": 25000, "propagation": "coupled"},
"profiles": [
  {"range": 12500, "title": "Gulf at 12.5 km", "media": [...]},
  {"range": 25000, "media": [...], "bottom": {"type": "halfspace", "cp": 1800, "rho": 2, "ap": 0.1}}
]
```

Source, receivers, frequencies, solver and FIELD options are shared by every
profile.

## From `.env` and `.flp` files

```console
pelagic kraken export case.env --solver krakenc > case.json
pelagic kraken run case.json --output results/case --format both
```

`export` reads the `.env`, the `.flp` (same stem, or `--flp PATH`) and the
`.trc`/`.brc`/`.irc`/`.sbp` files they use. It keeps the loss recipe and writes
only what differs from the defaults and, for each profile, from the top level.
The cases it builds are identical to the `.env` path. Every curated fixture is
tested for this.

## Limits

JSON input is at most 1 MiB. At most 1000 frequencies are allowed, and the
expanded case data across all frequencies and profiles is bounded at
5,000,000 values. HDF5 results record the input bytes and their SHA-256 under
`/inputs/json`.

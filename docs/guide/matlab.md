# MATLAB and Octave guide

The Pelagic toolbox runs BELLHOP and KRAKEN/KRAKENC from MATLAB or GNU Octave.
Cases are plain structs, and results are plain structs of numeric arrays. The
[MATLAB examples](../../examples/README.md#matlaboctave-scripts) are complete,
runnable scripts for each topic below.

## Install

**MATLAB R2023a or newer:** download `pelagic-vX.Y.Z.mltbx` from
[Releases](https://github.com/leihuang1/pelagic/releases), then double-click it
or run `matlab.addons.install('pelagic-vX.Y.Z.mltbx')`. The toolbox includes the
`pelagic` program for Windows, macOS and Linux.

**GNU Octave 7 or newer:** follow the
[Octave steps in Getting started](../getting-started.md#gnu-octave).

Both need Java, which MATLAB enables by default; in Octave, `usejava('jvm')`
must return true. Check the installation with:

```matlab
pelagic.version()
```

## Load, change and run a case

```matlab
c = pelagic.bellhop.load('munk-tl.json');        % or a legacy .env file
c.frequency_hz = 200;
c.positions.receiver_ranges_m = linspace(0, 10e3, 501);
r = pelagic.bellhop.run(c);

imagesc(r.ranges_m, r.depths_m, squeeze(r.field.transmission_loss_db(1,:,:)));
colorbar; xlabel('Range (m)'); ylabel('Depth (m)');
```

For KRAKEN/KRAKENC, legacy input is a `.env` plus `.flp` pair:

```matlab
c = pelagic.kraken.load('pekeris.env', 'solver', 'krakenc');   % uses pekeris.flp
c = pelagic.kraken.load('pekeris.env', 'flp', 'geometry.flp');
r = pelagic.kraken.run(c);
modes = r.frequencies{1}.profiles{1}.modes;
plot(real(modes.wavenumbers), 'o');
```

`load` converts the input to Pelagic's JSON format, so the struct has the
fields described in [BELLHOP JSON](../bellhop/json.md) and
[KRAKEN JSON](../kraken/json.md). You can also build a struct from scratch, or
read one with `jsondecode(fileread('case.json'))`.

### How struct values map to JSON

You can mostly write ordinary MATLAB values. The details:

- **Numeric arrays** are row vectors. Assigning a column vector or a scalar
  also works: `c.positions.source_depths_m = 25` is sent as `[25]`.
- **Lists of objects**, such as KRAKEN `frequencies` and `profiles`, and nested
  rows such as `speeds_mps`, are cell arrays. Add an entry with
  `c.frequencies{end+1} = ...`.
- An empty list is `{}`, and JSON `null` is `NaN`.
- Numbers are sent with full double precision.

All validation happens in Pelagic, so a wrong field name or value gives a
clear error naming the field.

## Results

**BELLHOP pressure fields:**

| Field | Contents |
| --- | --- |
| `r.pressure` | complex single, `[source, receiver_depth, receiver_range]` |
| `r.ranges_m`, `r.depths_m`, `r.source_depths_m` | Coordinates in metres, depth positive down |
| `r.field.transmission_loss_db` | `-20*log10(abs(pressure))` |
| `r.field.geometry` | `'rectilinear'` or `'irregular'`; irregular pressure is `[source, receiver]` |

MATLAB drops trailing singleton dimensions from `size`, but the axis order
above always holds. Use `squeeze` when you want a 2D slice for plotting.

**BELLHOP rays, eigenrays and arrivals** are returned as `r.rays`, `r.eigenrays`
or `r.arrivals`. Each is a struct of flat column vectors with offset arrays
(`point_offset`, `receiver_offset`, `arrival_offset`) that mark where each
ray or receiver starts. **Offsets are zero-based**, so add 1 for MATLAB
indexing. To get the points of ray `k`:

```matlab
o = r.rays.point_offset;
points = (o(k) + 1):o(k + 1);
plot(r.rays.range_m(points), r.rays.depth_m(points));
set(gca, 'YDir', 'reverse');
```

**KRAKEN** returns one cell per input frequency, in order:

```matlab
f = r.frequencies{1};
f.frequency_hz
f.profiles{1}.range_m
f.profiles{1}.modes      % depths_m, wavenumbers, eigenfunctions [depth, mode],
                         % phase_speed_mps, group_speed_mps, attenuation_nepers_per_m
f.field.pressure         % [source, receiver_depth, receiver_range]
```

`r.provenance` records the Pelagic version, the pinned reference and the exact
JSON that ran (`case_json`). `r.diagnostics` holds any warnings from the run.

## Errors and warnings

Errors have identifiers such as `pelagic:BH0201`. To see every problem the
input has, not only the first, use `pelagic.diagnostics`:

```matlab
try
    r = pelagic.bellhop.run(c);
catch err
    d = pelagic.diagnostics(err);   % struct array: code, severity, field, message, source
    disp({d.code; d.field})
end
```

`[c, warnings] = pelagic.bellhop.load(...)` returns load warnings in the second
output. Codes are explained in [Troubleshooting](troubleshooting.md#error-codes).

## Saving files

`run` returns results in memory and removes its temporary files. To keep
Toolbox-format or HDF5 files, use the [command-line program](cli.md) of the
same version:

```matlab
system('pelagic bellhop run case.env --format both --output results');
```

## Which `pelagic` program is used

The toolbox looks, in order, for:

1. the program named by the `PELAGIC_CLI` environment variable;
2. the copy bundled in the installed toolbox;
3. `pelagic` on the system `PATH`.

The program's version must match the toolbox exactly. Otherwise every call
fails with `pelagic:PG0202`. The check runs once per program file, not on every
call. The toolbox never downloads or updates anything.

The bundled Linux programs need glibc 2.35+ (x86-64) or 2.39+ (ARM64). On older
systems, build `pelagic` from source and set `PELAGIC_CLI`.

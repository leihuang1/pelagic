# Intentional compatibility deviations

Only intentional differences that affect numerical or discrete behavior belong here. Ordinary porting defects are fixed directly and covered by tests.

## Double-precision source and receiver geometry

Pelagic stores and interpolates source and receiver depths in `f64` instead
of reproducing Acoustics Toolbox's single-precision position grids. This keeps
all geometric coordinates at the solver's native precision, preserves modern
JSON inputs, and avoids rejecting decimal coordinates that lie exactly on an
`f64` water-column boundary.

The pinned `sbcx_Arr_asc` comparison changes four of 50,000 receiver arrival
counts by one at influence-window edges. Receivers with matching counts remain
within `3.5e-7` amplitude, `2.0e-5` degrees, and `2.0e-6` seconds of the
single-precision reference. Critical rays and the 501,501-sample Munk coherent
field retain their previous tolerances. Arrival values and pressure remain
single precision at their documented output rounding points.

The same-version Linux business diagnostic isolates this difference in the two
Ellipse fields, SBCX pressure, and Arctic pressure: replacing only receiver
depths in diagnostic JSON with the oracle's actual grid makes all four pass the
unchanged `5e-8` pressure tolerance. Both Ellipse fields then agree exactly;
SBCX/Arctic maximum errors are below `6e-11`. Original ENV cases remain failed,
not waived. See the [business report](../development/bellhop-business-coverage.md)
for the controls and depth discrepancies; production inputs remain `f64`.

## Precalculated `.irc` bottom reflection

Acoustics Toolbox BELLHOP `v2023.5` reads a bottom `P` option and loads the
same-stem `.irc` file through `misc/RefCoef.f90`, but
`bellhop.f90::Reflect2D` has no `P` branch and terminates with an unknown
boundary condition when the first such reflection occurs.

Pelagic implements the evidently intended behavior: it uses
`RefCoef.f90::InterpolateIRC`'s power-scaled quadratic interpolation and the
complex reflection-coefficient formula used by `Kraken/bounce.f90`. A reduced
BOUNCE-generated `.irc`/`.brc` pair verifies amplitude and phase. This makes a
traditional input useful where the pinned BELLHOP oracle is internally
incomplete.

The top-boundary `P` option remains rejected because the `.irc` generation and
loading path defines a bottom impedance only.

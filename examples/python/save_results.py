"""Save results: Toolbox-format files, NumPy, CSV and netCDF.

    python examples/python/save_results.py

- write_native() writes .shd/.ray/.arr/.mod files that Acoustics Toolbox
  readers (plotshd, read_arrivals_asc, ...) understand, plus the exact case JSON.
- Arrays save directly with NumPy.
- pandas and xarray export to CSV and netCDF (netCDF needs scipy or netCDF4).
For HDF5, use the command line: pelagic bellhop run case.json --format hdf5.
"""
from pathlib import Path

import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

# Toolbox-format files, written from the result in memory: nothing is solved again.
result = pg.bellhop.load(CASES / "bellhop/munk-tl/munk-tl.json").run()
result.write_native(OUT / "munk-tl")
print("native files:", sorted(path.name for path in (OUT / "munk-tl").iterdir()))

# Plain NumPy, with the coordinates alongside.
field = result.field
np.savez_compressed(OUT / "munk-tl.npz", pressure=field.pressure, ranges_m=field.ranges_m,
                    depths_m=field.depths_m, source_depths_m=field.source_depths_m)
print("wrote", OUT / "munk-tl.npz")

# Provenance: enough to reproduce the result later.
provenance = result.provenance
print(f"Pelagic {provenance.version}, {provenance.compatibility_reference}, case sha256 {provenance.case_sha256[:12]}…")

# Arrivals as a table.
arrivals = pg.bellhop.load(CASES / "bellhop/munk-arrivals/munk-arrivals.json").run()
try:
    arrivals.arrivals.to_pandas().to_csv(OUT / "munk-arrivals.csv", index=False)
    print("wrote", OUT / "munk-arrivals.csv")
except ImportError:
    print("install pandas for CSV export")

# A labelled field as netCDF.
try:
    dataset = result.to_xarray().to_dataset()
    with np.errstate(divide="ignore"):  # zero pressure -> infinite loss
        dataset["transmission_loss_db"] = -20 * np.log10(abs(dataset["pressure"]))
    dataset = dataset.drop_vars("pressure")  # netCDF has no complex type
    dataset.to_netcdf(OUT / "munk-tl.nc")
    print("wrote", OUT / "munk-tl.nc")
except (ImportError, ValueError) as error:
    print(f"skipped netCDF: {error}")

# KRAKEN results write .mod and .shd.
pg.kraken.load(CASES / "kraken/pekeris/pekeris.json").run().write_native(OUT / "pekeris")
print("KRAKEN files:", sorted(path.name for path in (OUT / "pekeris").iterdir()))

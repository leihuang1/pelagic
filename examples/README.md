# Examples

| File | What it is |
| --- | --- |
| [`field-g.json`](field-g.json) | A small BELLHOP coherent pressure field (7 beams, so it warns `BH1003`) |
| [`kraken-pekeris.json`](kraken-pekeris.json) | KRAKEN modes and FIELD for a Pekeris waveguide |
| [`notebooks/bindings-gallery.ipynb`](notebooks/bindings-gallery.ipynb) | Python gallery: a pressure field, a ray, KRAKEN modes and FIELD, and auxiliary inputs, with pinned Fortran comparisons |

Both JSON cases are self-contained: they were produced with `pelagic … export`
and need no auxiliary files. They are used by the CLI, Python and HTTP examples
in the [documentation](../docs/README.md). The notebook runs in CI, so it stays
current.

Larger legacy inputs, including Toolbox `.env`/`.flp` files with auxiliary
data, are under `crates/*/tests/fixtures/`.

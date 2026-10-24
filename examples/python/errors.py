"""Errors and warnings: what Pelagic reports, and how to handle it in code.

    python examples/python/errors.py

Every problem carries a stable code (see docs/guide/troubleshooting.md), the
JSON path of the field involved, and a message. Match on the code, not the text.
"""
import warnings
from pathlib import Path

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
spec = pg.bellhop.load(CASES / "bellhop/munk-tl/munk-tl.json").to_dict()

# 1. Invalid input: every problem is reported, not only the first.
bad = dict(spec, frequency=-50.0)
try:
    pg.bellhop.Case.from_dict(bad)
except pg.ValidationError as error:
    for diagnostic in error.diagnostics:
        print(f"{diagnostic.code} at {diagnostic.field}: {diagnostic.message}")

# 2. Malformed JSON structure: an unknown field.
try:
    pg.bellhop.Case.from_dict(dict(spec, frequency_hz=50.0))
except pg.ValidationError as error:
    print(f"{error.code} at {error.field}")

# 3. Warnings never stop a run. They are Python warnings and are also kept on the case.
with warnings.catch_warnings(record=True) as caught:
    warnings.simplefilter("always")
    case = pg.bellhop.load(CASES / "bellhop/field-g/field-g.json")
print("warnings:", [w.message.diagnostic.code for w in caught], "| on the case:", [d.code for d in case.warnings])

# 4. Treat warnings as errors when a script must not continue past one.
with warnings.catch_warnings():
    warnings.simplefilter("error", pg.PelagicWarning)
    try:
        pg.bellhop.load(CASES / "bellhop/field-g/field-g.json")
    except pg.PelagicWarning as warning:
        print("stopped on", warning.diagnostic.code)

# 5. All Pelagic exceptions share one base class.
pekeris = pg.kraken.load(CASES / "kraken/pekeris/pekeris.json").to_dict()
for problem in (lambda: pg.kraken.load(CASES / "missing.env"),            # file not found
                lambda: pg.kraken.Case.from_dict(dict(pekeris, pelagic=3))):  # newer schema
    try:
        problem()
    except pg.PelagicError as error:
        print(f"{type(error).__name__}: {error.code} ({error.field})")

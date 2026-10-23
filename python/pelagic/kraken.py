"""KRAKEN/KRAKENC JSON v1: ordered frequencies, profile modes and FIELD."""
from ._case import BaseCase
from ._sweep import run_many


class Case(BaseCase):
    _model = "kraken"


def load(path, *, flp=None, solver=None):
    """Load JSON or an ENV/FLP pair. solver defaults to kraken for legacy only."""
    return Case._load(path, flp=flp, solver=solver)

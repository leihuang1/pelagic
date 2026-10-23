"""BELLHOP's own JSON v1 contract and immutable Cases."""
from ._case import BaseCase
from ._sweep import run_many


class Case(BaseCase):
    _model = "bellhop"


def load(path):
    """Load JSON or normalize legacy ENV plus consumed auxiliary files through JSON."""
    return Case._load(path)

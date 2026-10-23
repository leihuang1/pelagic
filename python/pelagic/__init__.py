"""Pelagic: in-process underwater acoustics, without HDF5 or a CLI dependency."""
from ._core import __version__, compatibility_reference, reference_commit, case_schema_versions
from . import bellhop, kraken
from .errors import (Diagnostic, ValidationError, SimulationError, OutputError,
                     CancelledError, PelagicError, PelagicWarning)


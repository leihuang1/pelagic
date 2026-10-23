"""Stable structured diagnostics; messages are descriptive, not an API to parse."""
from dataclasses import dataclass
import json
import warnings
from . import _core


@dataclass(frozen=True)
class Source:
    path: str
    line: int
    column: int


@dataclass(frozen=True)
class Diagnostic:
    code: str
    severity: str
    field: str
    message: str
    source: Source = None

    @classmethod
    def from_dict(cls, value):
        value = dict(value)
        if value.get("source") is not None:
            value["source"] = Source(**value["source"])
        return cls(**value)


class PelagicError(Exception):
    def __init__(self, diagnostics, *, case_index=None):
        self.diagnostics = tuple(diagnostics)
        self.case_index = case_index
        first = self.diagnostics[0]
        self.code, self.severity, self.field = first.code, first.severity, first.field
        self.path = self.field  # Field path, not the source filename.
        super().__init__("\n".join(f"{d.severity}[{d.code}]: {d.message} ({d.field})" for d in self.diagnostics))


class ValidationError(PelagicError):
    pass


class SimulationError(PelagicError):
    pass


class OutputError(PelagicError):
    pass


class CancelledError(SimulationError):
    pass


class PelagicWarning(UserWarning):
    def __init__(self, diagnostic):
        self.diagnostic = diagnostic
        super().__init__(f"{diagnostic.code}: {diagnostic.message} ({diagnostic.field})")


def diagnostics(text):
    return tuple(Diagnostic.from_dict(value) for value in json.loads(text))


def call(function, *args, **kwargs):
    try:
        return function(*args, **kwargs)
    except _core.NativeError as cause:
        payload = json.loads(str(cause))
        items = tuple(Diagnostic.from_dict(value) for value in payload["diagnostics"])
        error = {"validation": ValidationError, "simulation": SimulationError, "output": OutputError}[payload["kind"]]
        if any(d.code in ("BH0304", "KR0304") for d in items):
            error = CancelledError
        raise error(items) from None


def emit(items, stacklevel=3):
    """stacklevel counts frames from emit's caller up to user code."""
    for item in items:
        warnings.warn(PelagicWarning(item), stacklevel=stacklevel)

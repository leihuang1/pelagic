from dataclasses import dataclass
import json
import numpy as np
from . import _core
from .errors import Diagnostic, ValidationError, call, diagnostics, emit


def _json_value(value):
    # Transport conversion only. Rust owns all acoustic/schema validation.
    if isinstance(value, np.ndarray):
        return value.tolist()
    if isinstance(value, np.generic):
        return value.item()
    raise TypeError(f"{type(value).__name__} is not JSON serializable")


@dataclass(frozen=True)
class BaseCase:
    _native: object

    @classmethod
    def from_dict(cls, document):
        try:
            text = json.dumps(document, default=_json_value, ensure_ascii=False, separators=(",", ":"))
        except (TypeError, ValueError, OverflowError) as error:
            raise ValidationError([Diagnostic("PG0103", "error", "json", str(error))]) from None
        native = call(_core.NativeCase.from_json, cls._model, text)
        emit(diagnostics(native.warnings))
        return cls(native)

    @classmethod
    def _load(cls, path, *, flp=None, solver=None):
        native = call(_core.NativeCase.load, cls._model, path, flp, solver)
        # User -> bellhop/kraken.load -> _load -> emit.
        emit(diagnostics(native.warnings), stacklevel=4)
        return cls(native)

    @property
    def warnings(self):
        return diagnostics(self._native.warnings)

    def to_dict(self):
        return json.loads(self._native.document)

    def run(self):
        from ._sweep import run_many
        with run_many([self], workers=1) as results:
            return next(results)

    def _run_native(self, token):
        return call(self._native.run, token)

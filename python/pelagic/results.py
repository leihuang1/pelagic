"""Immutable numerical snapshots. Units and axes are part of the public API."""
from dataclasses import dataclass
import numpy as np
from . import _core
from .errors import call, diagnostics, emit


@dataclass(frozen=True)
class Field:
    geometry: str
    pressure: np.ndarray
    ranges_m: np.ndarray
    depths_m: np.ndarray
    source_depths_m: np.ndarray
    receiver_offsets_m: np.ndarray = None

    def transmission_loss(self):
        """Relative transmission loss in dB; zero pressure gives +inf, without clipping."""
        with np.errstate(divide="ignore"):
            return -20 * np.log10(np.abs(self.pressure))

    def to_xarray(self):
        import xarray as xr
        if self.geometry == "irregular":
            dims = ("source_depth", "receiver")
            coords = {"source_depth": self.source_depths_m,
                      "range_m": ("receiver", self.ranges_m),
                      "depth_m": ("receiver", self.depths_m)}
        else:
            dims = ("source_depth", "receiver_depth", "receiver_range")
            coords = dict(zip(dims, (self.source_depths_m, self.depths_m, self.ranges_m)))
            if self.receiver_offsets_m is not None:
                coords["receiver_offset_m"] = ("receiver_depth", self.receiver_offsets_m)
        # A writable conversion is independent of the immutable solver snapshot.
        value = xr.DataArray(self.pressure.copy(), dims=dims, coords=coords, name="pressure",
                             attrs={"units": "1", "geometry": self.geometry})
        for coordinate in value.coords.values():
            coordinate.attrs["units"] = "m"
        return value


@dataclass(frozen=True)
class Ray:
    points: np.ndarray  # range, depth, travel time, attenuation time, amplitude, phase
    source_index: int
    source_depth_m: float
    launch_angle_degrees: float
    top_bounces: int
    bottom_bounces: int
    termination: str
    receiver_index: int = None
    range_m: float = None
    depth_m: float = None


@dataclass(frozen=True)
class Arrivals:
    source_depths_m: np.ndarray
    receiver_offset: np.ndarray
    arrival_offset: np.ndarray
    ranges_m: np.ndarray
    depths_m: np.ndarray
    amplitude: np.ndarray
    phase_radians: np.ndarray
    travel_time_s: np.ndarray
    attenuation_time_s: np.ndarray
    source_angle_degrees: np.ndarray
    receiver_angle_degrees: np.ndarray
    top_bounces: np.ndarray
    bottom_bounces: np.ndarray

    def to_pandas(self):
        import pandas as pd
        counts = np.diff(self.arrival_offset).astype(np.intp)
        sources = np.repeat(np.arange(len(self.source_depths_m)), np.diff(self.receiver_offset).astype(np.intp))
        receiver = np.repeat(np.arange(len(self.ranges_m)), counts)
        source = np.repeat(sources, counts)
        values = {name: getattr(self, name).copy() for name in (
            "amplitude", "phase_radians", "travel_time_s", "attenuation_time_s",
            "source_angle_degrees", "receiver_angle_degrees", "top_bounces", "bottom_bounces")}
        values.update(source_index=source, receiver_index=receiver,
                      source_depth_m=self.source_depths_m[source],
                      range_m=self.ranges_m[receiver], depth_m=self.depths_m[receiver])
        return pd.DataFrame(values)


@dataclass(frozen=True)
class Modes:
    depths_m: np.ndarray
    wavenumbers: np.ndarray  # complex128, radians/m
    eigenfunctions: np.ndarray  # complex128 [sample_depth, mode], reference normalized
    phase_speed_mps: np.ndarray
    group_speed_mps: np.ndarray
    attenuation_nepers_per_m: np.ndarray

    def to_xarray(self):
        import xarray as xr
        return xr.Dataset({
            "wavenumber": ("mode", self.wavenumbers.copy(), {"units": "rad/m"}),
            "eigenfunction": (("sample_depth", "mode"), self.eigenfunctions.copy(), {"units": "reference_normalized"}),
            "phase_speed": ("mode", self.phase_speed_mps.copy(), {"units": "m/s"}),
            "group_speed": ("mode", self.group_speed_mps.copy(), {"units": "m/s"}),
            "attenuation": ("mode", self.attenuation_nepers_per_m.copy(), {"units": "neper/m"}),
        }, coords={"sample_depth": self.depths_m.copy()})


@dataclass(frozen=True)
class Profile:
    range_m: float
    title: str
    modes: Modes


@dataclass(frozen=True)
class Frequency:
    frequency_hz: float
    title: str
    solver: str
    profiles: tuple
    field: Field

    def to_xarray(self):
        return {"field": self.field.to_xarray(),
                "profiles": tuple(p.modes.to_xarray() for p in self.profiles)}


@dataclass(frozen=True)
class Provenance:
    version: str
    compatibility_reference: str
    reference_commit: str
    input_sha256: str
    case_sha256: str
    case_json: str


class _Export:
    def write_native(self, directory, *, max_output_bytes=268435456):
        """Publish native files from this result, without solving again or invoking a CLI."""
        emit(diagnostics(call(self._native.write_native, directory, max_output_bytes)))


@dataclass(frozen=True)
class BellhopResult(_Export):
    _native: object
    provenance: Provenance
    warnings: tuple
    frequency_hz: float
    title: str
    field: Field
    rays: tuple
    eigenrays: tuple
    arrivals: Arrivals

    def to_xarray(self):
        if self.field is None:
            raise ValueError("this run has no pressure field; use rays or arrivals")
        value = self.field.to_xarray()
        value.attrs.update(version=self.provenance.version,
                           compatibility_reference=self.provenance.compatibility_reference,
                           case_sha256=self.provenance.case_sha256)
        return value


@dataclass(frozen=True)
class KrakenResult(_Export):
    _native: object
    provenance: Provenance
    warnings: tuple
    frequencies: tuple

    def to_xarray(self):
        """Preserve frequency order/repetitions and each profile's separate mode dimensions."""
        return tuple(frequency.to_xarray() for frequency in self.frequencies)


def from_native(native):
    data = native.arrays()
    provenance = Provenance(_core.__version__, _core.compatibility_reference, _core.reference_commit,
                            data.pop("input_sha256"), data.pop("case_sha256"), data.pop("case_json"))
    warnings = diagnostics(data.pop("warnings_json"))
    if data.pop("model") == "bellhop":
        data["field"] = None if data["field"] is None else Field(**data["field"])
        data["rays"] = tuple(Ray(**r) for r in data["rays"])
        data["eigenrays"] = tuple(Ray(**r) for r in data["eigenrays"])
        data["arrivals"] = Arrivals(**data["arrivals"])
        return BellhopResult(native, provenance, warnings, **data)
    frequencies = []
    for frequency in data["frequencies"]:
        frequency["profiles"] = tuple(Profile(p["range_m"], p["title"], Modes(**p["modes"])) for p in frequency["profiles"])
        frequency["field"] = Field(**frequency["field"])
        frequencies.append(Frequency(**frequency))
    return KrakenResult(native, provenance, warnings, tuple(frequencies))

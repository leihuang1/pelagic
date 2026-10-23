"""Broadband KRAKEN: a received pulse from 101 single-frequency solutions.

    python examples/python/broadband_pulse.py

The case solves 50-150 Hz in 1 Hz steps for one receiver 5 km away. Weighting
each frequency's pressure P(f) by a source spectrum S(f) and summing
S(f)·P(f)·exp(i2πft) gives the received waveform. Modal dispersion spreads the
pulse: higher modes travel more slowly. Writes results/broadband_pulse.png.
"""
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

result = pg.kraken.load(CASES / "kraken/broadband/broadband.json").run()
frequency = np.array([block.frequency_hz for block in result.frequencies])
pressure = np.array([block.field.pressure[0, 0, 0] for block in result.frequencies])
modes = [len(block.profiles[0].modes.wavenumbers) for block in result.frequencies]
print(f"{len(frequency)} frequencies, {min(modes)} to {max(modes)} modes")

source = np.hanning(len(frequency))  # a smooth band-limited source spectrum
range_m, reference_speed = result.frequencies[0].field.ranges_m[0], 1500.0
reduced_time = np.linspace(-0.1, 0.4, 2001)  # t - r/c0; 1 Hz steps repeat every 1 s
time = reduced_time + range_m / reference_speed
waveform = np.real(np.exp(2j * np.pi * np.outer(time, frequency)) @ (source * pressure))

fig, (spectrum, signal) = plt.subplots(2, 1, figsize=(8, 6))
spectrum.plot(frequency, 20 * np.log10(abs(pressure)))
spectrum.set(xlabel="Frequency (Hz)", ylabel="20 log10 |P| (dB)", title="Transfer function at 5 km")
signal.plot(reduced_time * 1e3, waveform, lw=0.8)
signal.set(xlabel=f"Reduced time t - r/{reference_speed:.0f} (ms)", ylabel="Pressure", title="Received pulse")

fig.tight_layout()
fig.savefig(OUT / "broadband_pulse.png", dpi=150)
plt.show()

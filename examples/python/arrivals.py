"""Arrivals: the channel impulse response at one receiver, and a received pulse.

    python examples/python/arrivals.py

Each arrival has an amplitude A, a phase φ and a travel time τ. A transmitted
signal s(t) is received as  r(t) = Re Σ A·exp(iφ)·ŝ(t − τ),  where ŝ is the
analytic signal of s. Writes results/arrivals.png and results/arrivals.csv
(the CSV needs pandas: pip install "pelagic[pandas]").
"""
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

case = pg.bellhop.load(CASES / "bellhop/munk-arrivals/munk-arrivals.env")
arrivals = case.run().arrivals
amplitude = arrivals.amplitude.astype(float)
phase = arrivals.phase_radians.astype(float)
delay = arrivals.travel_time_s.astype(float)
print(f"{len(delay)} arrivals between {delay.min():.3f} s and {delay.max():.3f} s")

# Consistency check: at the case frequency, the arrivals add up to the coherent field
# (BELLHOP's field carries an extra overall sign).
spec = case.to_dict()
omega = 2 * np.pi * spec["frequency"]
coherent = pg.bellhop.Case.from_dict(dict(spec, bellhop=dict(spec["bellhop"], run="coherent"))).run()
from_arrivals = np.sum(amplitude * np.exp(1j * (phase - omega * delay)))
print(f"|p| from arrivals {abs(from_arrivals):.3e}, coherent field {abs(coherent.field.pressure[0, 0, 0]):.3e}")

# A 50 Hz, four-cycle Hann-windowed tone burst, received through the channel.
fs, f0, cycles = 2000.0, spec["frequency"], 4
t = np.arange(0, delay.max() - delay.min() + 0.5, 1 / fs)
start = delay.min() - 0.1


def burst(time):
    """Analytic tone burst: exp(iωt) under a Hann window, zero outside it."""
    duration = cycles / f0
    inside = (time >= 0) & (time <= duration)
    return np.where(inside, np.sin(np.pi * time / duration) ** 2 * np.exp(1j * 2 * np.pi * f0 * time), 0)


received = np.real(sum(a * np.exp(1j * p) * burst(t + start - d) for a, p, d in zip(amplitude, phase, delay)))

fig, (top, bottom) = plt.subplots(2, 1, figsize=(8, 6), sharex=True)
top.stem(delay, amplitude, basefmt=" ")
top.set(ylabel="Amplitude", title="Arrivals (impulse response)")
bottom.plot(t + start, received, lw=0.8)
bottom.set(xlabel="Travel time (s)", ylabel="Pressure", title=f"Received {f0:.0f} Hz tone burst")
fig.tight_layout()
fig.savefig(OUT / "arrivals.png", dpi=150)

try:
    arrivals.to_pandas().to_csv(OUT / "arrivals.csv", index=False)
    print(f"wrote {OUT / 'arrivals.csv'}")
except ImportError:
    print("install pandas to also write arrivals.csv")
plt.show()

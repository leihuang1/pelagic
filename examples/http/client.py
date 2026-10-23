"""Call the BELLHOP HTTP service from Python, using only the standard library.

Start the server first, from a checkout:

    cargo run --release -p server               # listens on http://localhost:8080

then:

    python examples/http/client.py [http://localhost:8080]

It validates a case, fetches a focused JSON pressure field and an arrival list,
and downloads the complete HDF5 result to results/http-munk-tl.h5. Set
BELLHOP_AUTH_TOKEN if the server requires a token. See docs/guide/http.md.
"""
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

CASES = Path(__file__).resolve().parent.parent / "cases"
SERVER = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8080"


def post(path, case):
    headers = {"Content-Type": "application/json"}
    if os.environ.get("BELLHOP_AUTH_TOKEN"):
        headers["Authorization"] = f"Bearer {os.environ['BELLHOP_AUTH_TOKEN']}"
    request = urllib.request.Request(SERVER + path, data=json.dumps(case).encode(), headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            return response.read()
    except urllib.error.HTTPError as error:  # errors carry the usual diagnostics
        sys.exit(f"{path}: HTTP {error.code}: {error.read().decode()}")


case = json.loads((CASES / "bellhop/munk-tl/munk-tl.json").read_text(encoding="utf-8"))

print("validate:", json.loads(post("/v1/validate", case)))

# A small field as JSON: a few receivers at 1000 m depth.
small = dict(case, positions=dict(case["positions"], receiver_depths_m=[1000.0],
                                  receiver_ranges_m=[10e3, 25e3, 50e3, 100e3]))
field = json.loads(post("/v1/field", small))
for receiver in field["sources"][0]["receivers"]:
    pressure = complex(receiver["pressure"]["real"], receiver["pressure"]["imaginary"])
    print(f"  {receiver['range_m'] / 1e3:5.0f} km: |p| = {abs(pressure):.3e}")

# Arrivals for one receiver.
arrivals_case = json.loads((CASES / "bellhop/munk-arrivals/munk-arrivals.json").read_text(encoding="utf-8"))
receiver = json.loads(post("/v1/arrivals", arrivals_case))["sources"][0]["receivers"][0]
first = min(receiver["arrivals"], key=lambda arrival: arrival["travel_time_s"])
print(f"{len(receiver['arrivals'])} arrivals at {receiver['range_m'] / 1e3:g} km; "
      f"first after {first['travel_time_s']:.3f} s")

# The complete result as HDF5 (read it with examples/cli/read_hdf5.py or h5py).
Path("results").mkdir(exist_ok=True)
Path("results/http-munk-tl.h5").write_bytes(post("/v1/run", case))
print("wrote results/http-munk-tl.h5")

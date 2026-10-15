#!/usr/bin/env python3
"""Small orchestration checks; command stubs test guards, never numerical parity."""
import hashlib
import importlib.util
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
spec = importlib.util.spec_from_file_location("goldens", HERE / "verify-goldens.py")
goldens = importlib.util.module_from_spec(spec)
sys.dont_write_bytecode = True
spec.loader.exec_module(goldens)


def rejected(action):
    try:
        action()
    except ValueError:
        return
    raise AssertionError("invalid reference evidence was accepted")


with tempfile.TemporaryDirectory(prefix="pelagic-reference-tools-") as temporary:
    base = Path(temporary)
    artifact = base / "sample.mod"
    artifact.write_bytes(b"Fortran reference")
    entries = [(hashlib.sha256(artifact.read_bytes()).hexdigest(), artifact.name)]
    goldens.verify(base, entries, [artifact])
    rejected(lambda: goldens.verify(base, [], [artifact]))
    artifact.write_bytes(b"damaged reference")
    rejected(lambda: goldens.verify(base, entries, [artifact]))

    # Exercise the shell guard with controlled libtest output and process status.
    cargo = base / "cargo"
    environment = dict(os.environ, PATH=f"{base}{os.pathsep}{os.environ['PATH']}")
    for passed, exit_code, accepted in [(1, 0, True), (0, 0, False), (2, 0, False), (1, 7, False)]:
        cargo.write_text(f"#!/bin/sh\nprintf 'test result: ok. {passed} passed; 0 failed; 0 ignored;\\n'\nexit {exit_code}\n")
        cargo.chmod(0o755)
        result = subprocess.run(["bash", str(HERE / "test-one.sh")], env=environment, capture_output=True)
        assert (result.returncode == 0) == accepted, result

    cargo.write_text("#!/bin/sh\nprintf 'test result: ok. 1 passed; 0 failed; 0 ignored;\\n%.0s' 1 2\n")
    result = subprocess.run(["bash", str(HERE / "test-one.sh")], env=environment, capture_output=True)
    assert result.returncode != 0, "two one-test binaries must not count as exactly one test"

    # Failed external runs must retain their print files, not delete all evidence.
    docker = base / "docker"
    docker.write_text('''#!/bin/sh
[ "$1" = image ] && exit 0
while [ "$#" -gt 0 ]; do
  if [ "$1" = --volume ]; then
    work=${2%:/work}
    printf 'reference failure detail\\n' > "$work/failure.prt"
    exit 7
  fi
  shift
done
exit 99
''')
    docker.chmod(0o755)
    (base / "bad.env").write_text("invalid ENV")
    for helper, arguments in [("run-case.sh", []), ("run-kraken-case.sh", ["kraken"])]:
        output = base / helper
        result = subprocess.run(["bash", str(HERE / helper), *arguments, str(base / "bad.env"), str(output)],
                                env=environment, capture_output=True)
        assert result.returncode == 7, result
        retained = list(output.glob("failed-work-*/failure.prt"))
        assert len(retained) == 1 and retained[0].read_text() == "reference failure detail\n"

    # Fail at the first real stage: every later stage must be explicitly NOT RUN.
    rustc = base / "rustc"
    environment.update(KRAKEN_HDF5_RESULT="stale.h5", BELLHOP_NATIVE_INPUT="stale.env",
                       CARGO_TARGET_DIR="wrong-target")
    rustc.write_text('''#!/bin/sh
[ -z "${KRAKEN_HDF5_RESULT+x}" ] && [ -z "${BELLHOP_NATIVE_INPUT+x}" ] &&
  [ "$CARGO_TARGET_DIR" = "$PWD/target" ] || exit 99
exit 17
''')
    rustc.chmod(0o755)
    result = subprocess.run(["bash", str(HERE / "full-acceptance.sh"), "--allow-dirty"],
                            cwd=ROOT, env=environment, capture_output=True, text=True)
    assert result.returncode == 17, result
    report = Path(re.search(r"^Report: (.+)$", result.stdout, re.MULTILINE)[1])
    text = report.read_text()
    assert "\nFAILED\n" in text and "| Toolchain and committed references | FAIL |" in text
    stages = len(re.findall(r"^step '", (HERE / "full-acceptance.sh").read_text(), re.MULTILINE))
    assert text.count("| NOT RUN |") == stages - 1, text
    assert (report.parent / "run.log").is_file()
    shutil.rmtree(report.parent)  # Only this self-check's uniquely created output.

subprocess.run([sys.executable, str(HERE / "bellhop-business.py"), "--self-test"], check=True)
print("reference tooling guards: passed")

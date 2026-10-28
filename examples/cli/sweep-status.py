"""Summarize the status files of a `run-many` results folder.

    python3 examples/cli/sweep-status.py results [cases.txt]

Reads results/status.ndjson and every results/status-I-of-N.ndjson, the files
`--resume` reads, and takes each case's last line in each file. Prints how many
cases are done (solved now or skipped as current) and failed, then each failure
with its first error. Lines carry no time, so a case that is done in one file
and failed in another is unclear: which line is newer cannot be told. Given the
case list, it also prints the cases with no line yet, such as those of a task
that has not run or was killed first. Exits with 1 if any case failed, is
unclear or is missing. Needs Python 3.6 or newer, and nothing else.
"""
import json
import sys
from pathlib import Path


def status_files(folder):
    paths = Path(folder).glob("status*.ndjson")
    return sorted(path for path in paths if path.name == "status.ndjson" or path.name.startswith("status-"))


def main(folder, listing=None):
    recorded = {}  # each case's last record in each file, by stem and then file name
    for path in status_files(folder):
        for line in path.read_bytes().split(b"\n"):
            try:
                record = json.loads(line.decode("utf-8"))
            except ValueError:
                continue  # a line cut short when a task was killed, even inside a character
            recorded.setdefault(record["stem"], {})[path.name] = record  # later lines are later runs
    failed, unclear = [], []
    for files in recorded.values():
        states = {name: "failed" if record["outcome"] == "failed" else "done" for name, record in files.items()}
        last = list(files.values())[-1]
        if len(set(states.values())) > 1:
            unclear.append((last["case"], states))
        elif "failed" in states.values():
            failed.append(last)
    missing = []
    if listing:
        # One path per line, as `run-many --list` reads it.
        lines = Path(listing).read_text(encoding="utf-8").split("\n")
        lines = [line[:-1] if line.endswith("\r") else line for line in lines]
        cases = [line for line in lines if line]
        missing = [case for case in cases if Path(case).stem not in recorded]
    done = len(recorded) - len(failed) - len(unclear)
    print(f"{len(recorded)} cases recorded: {done} done, {len(failed)} failed"
          + (f", {len(unclear)} unclear" if unclear else "")
          + (f"; {len(missing)} of {len(cases)} listed cases have no record" if listing else ""))
    for record in sorted(failed, key=lambda record: record["case"]):
        errors = [d for d in record["diagnostics"] if d["severity"] == "error"] or record["diagnostics"]
        reason = f"{errors[0]['code']} {errors[0]['message']}" if errors else "no diagnostic"
        print(f"failed  {record['case']}: {reason} (exit code {record['exit_code']})")
    for case, states in sorted(unclear, key=lambda item: item[0]):
        print(f"unclear {case}: " + ", ".join(f"{state} in {name}" for name, state in states.items()))
    for case in missing:
        print(f"missing {case}")
    return 1 if failed or unclear or missing else 0


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3):
        sys.exit("usage: sweep-status.py RESULTS [CASE-LIST]")
    sys.exit(main(*sys.argv[1:]))

#!/usr/bin/env python3
"""Small standard-library tests for fail-closed CI planning and complete shards."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


def module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


changes = module("changes")
shards = module("pytest-shard")
coverage = module("check-shards")
results = module("check-results")


class PlanningTests(unittest.TestCase):
    def test_aggregate_accepts_only_explicit_documentation_skips(self):
        needs = {"plan": {"result": "success", "outputs": {"full": "false"}},
                 "windows-check": {"result": "success"}, "wheel-tests": {"result": "skipped"}}
        results.check(needs)
        needs["plan"]["outputs"]["full"] = "true"
        with self.assertRaises(AssertionError):
            results.check(needs)
        needs["wheel-tests"]["result"] = "success"
        results.check(needs)
        for state in ("failure", "cancelled", "skipped"):
            needs["windows-check"]["result"] = state
            with self.assertRaises(AssertionError):
                results.check(needs)

    def test_prose_only(self):
        self.assertFalse(changes.needs_full_validation(["README.md", "docs/guide.md", "docs/images/schema.svg"]))

    def test_unknown_and_functional_files_fail_closed(self):
        for name in ["Cargo.lock", ".github/workflows/ci.yml", "tools/ci/changes.py", "docs/example.py",
                     "examples/notebooks/tutorial.ipynb", "python/README.md", "LICENSE", "matlab/VERSION"]:
            self.assertTrue(changes.needs_full_validation(["README.md", name]), name)
        self.assertTrue(changes.needs_full_validation([]))

    def test_shards_cover_each_test_once(self):
        nodes = [f"test_cases.py::test_case[{n}]" for n in range(1000)]
        for count in (1, 2, 3, 7):
            manifests = [{"index": i, "count": count, "all": nodes,
                          "selected": [n for n in nodes if shards.shard_for(n, count) == i]}
                         for i in range(count)]
            self.assertEqual(coverage.check(manifests), len(nodes))
            if count > 1:
                with self.assertRaises(AssertionError):
                    coverage.check(manifests[:-1])
            manifests[0]["selected"].append(nodes[0])
            with self.assertRaises(AssertionError):
                coverage.check(manifests)

    def test_different_collections_are_rejected(self):
        with self.assertRaises(AssertionError):
            coverage.check([{"index": 0, "count": 2, "all": ["a"], "selected": ["a"]},
                            {"index": 1, "count": 2, "all": ["b"], "selected": []}])

    def test_real_git_events_fail_closed(self):
        script = Path(__file__).with_name("changes.py").resolve()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.check_output(["git", "-C", directory, *args], stderr=subprocess.DEVNULL).decode().strip()
            git("init", "-q")
            git("config", "user.name", "CI tests")
            git("config", "user.email", "ci-tests@example.invalid")
            (root / "code.py").write_text("print('executable')\n")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            (root / "docs").mkdir()
            git("mv", "code.py", "docs/code.md")
            git("commit", "-qm", "move code to docs")
            head = git("rev-parse", "HEAD")
            event = root / "event.json"
            output = root / "output"
            for event_name, payload in [
                ("push", {"before": base}),
                ("pull_request", {"pull_request": {"base": {"sha": base}}}),
                ("push", {"before": "f" * 40}),
                ("push", {"before": "0" * 40}),
                ("push", {"before": head}),
                ("workflow_dispatch", {}),
                ("release", {}),
            ]:
                event.write_text(json.dumps(payload))
                output.write_text("")
                subprocess.run([sys.executable, str(script)], cwd=root, check=True,
                    stdout=subprocess.DEVNULL, env={**os.environ, "GITHUB_EVENT_NAME": event_name,
                    "GITHUB_EVENT_PATH": str(event), "GITHUB_OUTPUT": str(output), "GITHUB_SHA": head})
                self.assertEqual(output.read_text(), "full=true\n", (event_name, payload))


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Small standard-library tests for fail-closed CI planning and complete shards."""
import importlib.util
import copy
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
artifacts = module("wait-artifacts")


class PlanningTests(unittest.TestCase):
    def test_artifacts_are_pinned_to_latest_successful_producer(self):
        sha = "a" * 40
        job = {"name": "windows-wheel", "run_id": 123, "head_sha": sha,
               "status": "completed", "conclusion": "success", "run_attempt": 1,
               "started_at": "2026-01-01T10:00:00Z", "completed_at": "2026-01-01T10:02:00Z"}
        artifact = {"name": "cli-windows-x64", "id": 456, "expired": False,
                    "workflow_run": {"id": 123, "head_sha": sha},
                    "created_at": "2026-01-01T10:01:00Z", "digest": "sha256:" + "b" * 64}
        def select(a=artifact, j=job):
            return artifacts.select([a], [j], ["windows-x64"], 123, sha, 1)
        self.assertEqual(select()[0][0]["id"], 456)
        # Previous attempts are never accepted, including transient API responses.
        with self.assertRaises(ValueError):
            artifacts.select([artifact], [job], ["windows-x64"], 123, sha, 2)
        newer = {**job, "run_attempt": 2, "started_at": "2026-01-01T11:00:00Z",
                 "completed_at": "2026-01-01T11:02:00Z"}
        with self.assertRaises(ValueError):
            select(j=newer)
        self.assertIsNone(artifacts.select([artifact], [newer], ["windows-x64"], 123, sha, 2))
        self.assertIsNone(select(j={**job, "status": "in_progress", "conclusion": None}))
        for replacement in ({"expired": True}, {"digest": None}, {"name": "cli-linux-x64"},
                            {"workflow_run": {"id": 999, "head_sha": sha}},
                            {"workflow_run": {"id": 123, "head_sha": "c" * 40}}):
            self.assertIsNone(select(a={**artifact, **replacement}))
        # A missing earlier platform must not hide a known later failure.
        with self.assertRaises(ValueError):
            artifacts.select([], [{**job, "conclusion": "failure"}],
                             ["linux-x64", "windows-x64"], 123, sha, 1)
        for state in ("failure", "cancelled", "timed_out", "skipped"):
            with self.assertRaises(ValueError):
                select(j={**job, "conclusion": state})
        with self.assertRaises(ValueError):
            artifacts.select([artifact, copy.deepcopy(artifact)], [job], ["windows-x64"], 123, sha, 1)
        self.assertTrue(artifacts.producer_name({"name": "wheels (macos-15, aarch64-apple-darwin, macos-arm64)"}, "macos-arm64"))
        self.assertFalse(artifacts.producer_name({"name": "wheel-tests (macos-15, aarch64-apple-darwin, macos-arm64)"}, "macos-arm64"))

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
            self.assertEqual(coverage.check(manifests, count), len(nodes))
            with self.assertRaises(AssertionError):
                coverage.check(manifests, count + 1)
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

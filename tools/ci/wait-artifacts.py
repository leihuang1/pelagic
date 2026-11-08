#!/usr/bin/env python3
"""Overlap MATLAB installation with builds, then pin verified same-run artifacts.

Only successful producers from the exact current attempt are accepted.
Use Re-run all jobs rather than Re-run failed jobs: an earlier attempt
can never satisfy a producer in the current attempt. Throttling, 5xx
responses and network errors are retried until the timeout.
"""
import argparse
from datetime import datetime
from http.client import HTTPException
import json
import os
import re
import time
from urllib.error import HTTPError, URLError
from urllib.request import HTTPRedirectHandler, Request, build_opener

PLATFORMS = {"linux-x64", "linux-arm64", "macos-arm64", "macos-x64", "windows-x64"}
TRANSIENT_STATUS = {429, 500, 502, 503, 504}


def producer_name(job, platform):
    if platform == "linux-x64":
        return job["name"] == "linux-wheel"
    if platform == "windows-x64":
        return job["name"] == "windows-wheel"
    return job["name"].startswith("wheels (") and job["name"].endswith(", " + platform + ")")


def timestamp(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def select(artifacts, jobs, platforms, run_id, head_sha, attempt):
    selected = []
    pending = False
    for platform in platforms:
        producers = [job for job in jobs if producer_name(job, platform)]
        if not producers:
            pending = True
            continue
        if len(producers) != 1:
            raise ValueError(f"ambiguous latest producer for {platform}")
        producer = producers[0]
        if producer["run_attempt"] != attempt:
            raise ValueError("producer attempt mismatch; use Re-run all jobs")
        if producer["run_id"] != run_id or producer["head_sha"] != head_sha:
            raise ValueError("producer run/commit mismatch")
        if producer["status"] != "completed":
            pending = True
            continue
        if producer["conclusion"] != "success":
            raise ValueError(f"{producer['name']} ended as {producer['conclusion']}")
        matches = [artifact for artifact in artifacts
            if artifact["name"] == "cli-" + platform
            and not artifact["expired"]
            and artifact.get("workflow_run", {}).get("id") == run_id
            and artifact["workflow_run"].get("head_sha") == head_sha
            and timestamp(producer["started_at"]) <= timestamp(artifact["created_at"]) <= timestamp(producer["completed_at"])
            and re.fullmatch(r"sha256:[0-9a-f]{64}", artifact.get("digest") or "")]
        if not matches:
            pending = True
            continue
        if len(matches) != 1:
            raise ValueError(f"ambiguous artifact for {platform}")
        selected.append((matches[0], producer))
    return None if pending else selected


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class API:
    def __init__(self, repository, token):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
            raise ValueError("invalid repository")
        self.base = "https://api.github.com/repos/" + repository
        self.token = token

    def request(self, path):
        request = Request(self.base + path, headers={"Authorization": "Bearer " + self.token,
            "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"})
        # Never forward the ephemeral token through a redirect to another host.
        return build_opener(NoRedirect).open(request, timeout=30)

    def get(self, path):
        with self.request(path) as response:
            return json.load(response)

    def pages(self, path, key):
        result = []
        for page in range(1, 11):
            items = self.get(path + ("&" if "?" in path else "?") + f"per_page=100&page={page}")[key]
            result.extend(items)
            if len(items) < 100:
                return result
        raise ValueError("unexpectedly large workflow run")


def transient(error):
    if isinstance(error, HTTPError):
        return error.code in TRANSIENT_STATUS
    return isinstance(error, (URLError, HTTPException, ConnectionError, TimeoutError))


def retrying(call, deadline, pause=10):
    """Return call(), retrying transient API failures until the deadline."""
    while True:
        try:
            return call()
        except (URLError, HTTPException, ConnectionError, TimeoutError) as error:
            if not transient(error) or time.monotonic() >= deadline:
                raise
            print(f"Transient GitHub API failure ({error}); retrying...", flush=True)
            time.sleep(pause)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platforms", choices=sorted(PLATFORMS), nargs="+", required=True)
    parser.add_argument("--timeout", type=int, default=1800)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 1800:
        parser.error("timeout must be 1..1800 seconds")
    run_id = int(os.environ["GITHUB_RUN_ID"])
    attempt = int(os.environ["GITHUB_RUN_ATTEMPT"])
    head_sha = os.environ["EXPECTED_HEAD_SHA"]
    api = API(os.environ["GITHUB_REPOSITORY"], os.environ["GH_TOKEN"])
    deadline = time.monotonic() + args.timeout
    run = retrying(lambda: api.get(f"/actions/runs/{run_id}/attempts/{attempt}"), deadline)
    if run["head_sha"] != head_sha or run["run_attempt"] != attempt:
        raise ValueError("workflow run/attempt/commit mismatch")
    while True:
        jobs = retrying(lambda: api.pages(f"/actions/runs/{run_id}/attempts/{attempt}/jobs", "jobs"), deadline)
        artifacts = retrying(lambda: api.pages(f"/actions/runs/{run_id}/artifacts", "artifacts"), deadline)
        selected = select(artifacts, jobs, args.platforms, run_id, head_sha, attempt)
        if selected is not None:
            for artifact, producer in selected:
                print(f"{artifact['name']}: artifact {artifact['id']}, successful producer attempt {producer['run_attempt']}, commit {head_sha}")
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                print("ids=" + ",".join(str(artifact["id"]) for artifact, _ in selected), file=output)
            return
        if time.monotonic() >= deadline:
            raise TimeoutError("matching producer artifacts did not become ready")
        print("Waiting for matching successful producers and immutable artifacts...", flush=True)
        time.sleep(10)


if __name__ == "__main__":
    main()

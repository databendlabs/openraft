"""Artifacts, host observations and paired statistics shared by benchmark scripts."""

import collections
import dataclasses
import hashlib
import json
import os
import platform
import random
import signal
import statistics
import subprocess
import threading
import time
import uuid
from pathlib import Path

SCHEMA_VERSION = 1
SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parents[2]
DEFAULT_CACHE = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "openraft-bench"
COMPILERS = {"cargo", "rustc", "rust-analyzer", "clang", "clang++", "cc", "c++", "cmake", "ninja"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_hash(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def json_bytes(value):
    return json.dumps(value, sort_keys=True, allow_nan=False, default=str).encode()


def write_json(path, value):
    Path(path).write_bytes(json_bytes(value) + b"\n")


def git(root, *args):
    result = subprocess.run(["git", *args], cwd=root, capture_output=True, check=True, timeout=30)
    return result.stdout.decode().strip()


def source_state(root):
    revision = git(root, "rev-parse", "HEAD")
    patch = subprocess.check_output(["git", "diff", "--no-ext-diff", "--no-textconv", "--binary", "HEAD"], cwd=root, timeout=30)
    names = git(root, "ls-files", "--others", "--exclude-standard", "-z")
    untracked = {}
    for name in names.split("\0") if names else []:
        path = Path(root) / name
        if path.is_symlink():
            untracked[name] = digest(("symlink:" + os.readlink(path)).encode())
        elif path.is_file():
            untracked[name] = file_hash(path)
    dirty = {"patch_sha256": digest(patch), "untracked": untracked}
    return {"revision": revision, "dirty_sha256": digest(json_bytes(dirty)), "dirty": bool(patch or untracked),
            "untracked": untracked}


def harness_state():
    files = {path.name: file_hash(path) for path in sorted(SCRIPT_DIR.glob("*.py"))}
    return {"source": source_state(ROOT), "files": files, "sha256": digest(json_bytes(files))}


def machine():
    return {"platform": platform.platform(), "architecture": platform.machine(), "cpu_count": os.cpu_count(),
            "python": platform.python_version(), "hostname": platform.node()}


def host_sample():
    sample = {"time_ns": time.time_ns(), "monotonic_ns": time.monotonic_ns(), "load": os.getloadavg()}
    try:
        result = subprocess.run(["ps", "-axo", "pid=,ppid=,comm="], capture_output=True, text=True, check=True, timeout=3)
        processes = [line.split(maxsplit=2) for line in result.stdout.splitlines()]
        sample["compilers"] = [{"pid": int(pid), "parent_pid": int(parent), "executable": name}
                               for pid, parent, name in processes if Path(name).name in COMPILERS]
    except (OSError, subprocess.SubprocessError) as error:
        sample["sampling_error"] = str(error)
    return sample


class HostMonitor:
    def __init__(self, path):
        self.path = Path(path)
        self.stop = threading.Event()
        self.samples = collections.deque(maxlen=4096)
        self.error = None

    def sample(self, output):
        sample = host_sample()
        self.samples.append(sample)
        print(json.dumps(sample), file=output, flush=True)

    def collect(self):
        try:
            while not self.stop.wait(1):
                self.sample(self.output)
        except BaseException as error:
            self.error = error

    def __enter__(self):
        self.output = self.path.open("x")
        self.sample(self.output)
        self.thread = threading.Thread(target=self.collect)
        self.thread.start()
        return self

    def __exit__(self, *error):
        self.stop.set()
        self.thread.join()
        try:
            self.sample(self.output)
        finally:
            self.output.close()
        if self.error is not None:
            raise RuntimeError("host sampler failed") from self.error

    def report(self):
        return {"samples": len(self.samples), "external_build_observed": any(item.get("compilers") for item in self.samples),
                "sampling_errors": [item["sampling_error"] for item in self.samples if "sampling_error" in item],
                "load_scope": "recorded condition; active benchmark load is not an invalidation rule",
                "log": str(self.path)}


@dataclasses.dataclass
class Command:
    args: list
    cwd: Path = ROOT
    timeout: float = 420
    env: dict = None

    def run(self, directory):
        directory = Path(directory)
        directory.mkdir(parents=True, exist_ok=False)
        record = {"schema_version": SCHEMA_VERSION, "command": self.args, "cwd": str(self.cwd),
                  "timeout_seconds": self.timeout, "started_ns": time.time_ns(), "status": "starting"}
        write_json(directory / "command.json", record)
        with (directory / "stdout").open("wb") as stdout, (directory / "stderr").open("wb") as stderr:
            self.execute(record, stdout, stderr)
        record["finished_ns"] = time.time_ns()
        write_json(directory / "command.json", record)
        return record

    def execute(self, record, stdout, stderr):
        try:
            process = subprocess.Popen(self.args, cwd=self.cwd, env=self.env, stdout=stdout, stderr=stderr,
                                       start_new_session=True)
        except OSError as error:
            record.update(status="spawn_error", error=str(error))
            return
        try:
            process.wait(timeout=self.timeout)
            record["status"] = "success" if process.returncode == 0 else "failed"
        except subprocess.TimeoutExpired:
            self.kill(process)
            record["status"] = "timeout"
        except BaseException:
            self.kill(process)
            raise
        record["returncode"] = process.returncode

    @staticmethod
    def kill(process):
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass  # The owned process group already exited.
        process.wait()


def require_success(record, directory):
    if record["status"] != "success":
        raise RuntimeError(f"command {record['status']}; complete output: {directory}")


def schedule(rows, modes, rounds):
    for round_index in range(rounds):
        offset = round_index % len(rows)
        indexes = list(range(len(rows)))
        row_order = indexes[offset:] + indexes[:offset]
        for row_index in row_order:
            row = rows[row_index]
            start = (round_index + row_index) % len(modes)
            for mode in modes[start:] + modes[:start]:
                yield round_index + 1, row, mode


def paired_change(base, target, seed=1, samples=10000):
    if len(base) != len(target):
        raise ValueError("paired statistics require the same rounds")
    changes = [100 * (right / left - 1) if left and right is not None else None for left, right in zip(base, target)]
    if any(value is None for value in changes) or not changes:
        return {"paired_percent": changes, "median_percent": None, "ci95_percent": None}
    randomizer = random.Random(seed)
    bootstrapped = [statistics.median(randomizer.choices(changes, k=len(changes))) for _ in range(samples)]
    bootstrapped.sort()
    interval = [bootstrapped[int(samples * 0.025)], bootstrapped[min(samples - 1, int(samples * 0.975))]]
    return {"paired_percent": changes, "median_percent": statistics.median(changes), "ci95_percent": interval,
            "method": "percentile bootstrap of paired round medians", "resampling_unit": "complete paired round",
            "rounds": len(changes), "bootstrap_samples": samples, "seed": seed}


class Artifacts:
    def __init__(self, args):
        self.args = args
        identifier = time.strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:8]
        self.path = args.output or args.cache_dir / "runs" / identifier
        self.path.mkdir(parents=True, exist_ok=False)
        if args.raw:
            args.raw.parent.mkdir(parents=True, exist_ok=True)
            args.raw.touch(exist_ok=False)
        self.records = []
        self.manifest = {"schema_version": SCHEMA_VERSION, "harness": harness_state(), "machine": machine(),
                         "arguments": vars(args), "status": "running", "runs": [],
                         "environment_policy": "retain all runs; sampled compiler activity or changing inputs makes a group diagnostic; active load is descriptive"}
        write_json(self.path / "manifest.json", self.manifest)

    def record(self, record):
        self.records.append(record)
        with (self.path / "runs.jsonl").open("a") as output:
            print(json.dumps(record, default=str, allow_nan=False), file=output, flush=True)
        if self.args.raw:
            with self.args.raw.open("a") as output:
                print(json.dumps(record, default=str, allow_nan=False), file=output, flush=True)

    def finish(self, summary, error=None):
        self.manifest.update(status="failed" if error else "success", error=error, runs=self.records)
        write_json(self.path / "manifest.json", self.manifest)
        write_json(self.path / "summary.json", summary)

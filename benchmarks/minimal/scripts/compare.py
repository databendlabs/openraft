#!/usr/bin/env python3
"""Compare `bench` between two git revisions on the same machine.

Builds `bench` at the base and the target revision, runs the same bench
arguments with both binaries `--runs` times, and prints the medians side by
side. The two binaries run back to back for every row, and their order swaps
on every run, so a drift in machine load affects both alike.

A revision is built in a temporary git worktree, and its binary is cached
under `benchmarks/minimal/target/compare/<commit>`. Without `--target`, the
target is the current checkout, including uncommitted changes.

Examples:
    # Any bench command line; it must include --latency:
    compare.py --base a5c043ef -- --latency -c 256 -n 30k --apply-io-delay-us 500
    # The grid of `make bench_apply_delays`:
    compare.py --base a5c043ef --apply-delays
"""

import argparse
import json
import os
import platform
import shlex
import shutil
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
BENCH_DIR = SCRIPT_DIR.parent
CACHE_DIR = BENCH_DIR / "target" / "compare"

sys.path.insert(0, str(SCRIPT_DIR))
from apply_delays import DELAYS, LOADS, grid_args  # noqa: E402

# (column header, field of the bench JSON report, divisor)
METRICS = [
    ("writes/s", "operations_per_second", 1),
    ("p50 µs", "request_p50_ns", 1000),
    ("p99 µs", "request_p99_ns", 1000),
]


def main():
    args = parse_args()
    features = ["--no-default-features"] if args.no_default_features else []
    root = Path(git(BENCH_DIR, "rev-parse", "--show-toplevel"))

    base = build_revision(root, args.base, features)
    if args.target:
        target = build_revision(root, args.target, features)
    else:
        target = build_checkout(root, features)

    if args.apply_delays:
        rows = [
            (f"{clients} client{'s' * (clients != 1)}, read {read_us} µs, I/O {io_us} µs", grid_args(clients, ops, read_us, io_us))
            for clients, ops in LOADS
            for read_us, io_us in DELAYS
        ]
        scenario = "the grid of `make bench_apply_delays`"
    else:
        rows = [("bench args", args.bench_args)]
        scenario = "bench " + shlex.join(args.bench_args)

    load_start = os.getloadavg()[0]
    results = run_all([base, target], rows, args.runs, args.raw)
    load_end = os.getloadavg()[0]

    print(f"base:    {base[0]}")
    print(f"target:  {target[0]}")
    print(f"machine: {cpu_name()}, {platform.platform()}")
    print(f"runs:    {args.runs} per build of {scenario}")
    print(f"load average: {load_start:.1f} at start, {load_end:.1f} at end")
    print()
    print_table(rows, base[0], target[0], results)


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", required=True, help="revision to compare against")
    parser.add_argument("--target", help="revision to measure [default: the current checkout]")
    parser.add_argument("--runs", type=int, default=5, help="runs per build and row [default: 5]")
    parser.add_argument("--no-default-features", action="store_true", help="build bench without default features")
    parser.add_argument("--apply-delays", action="store_true", help="run the grid of `make bench_apply_delays`")
    parser.add_argument("--raw", type=Path, help="also write every run as one JSON line to this file")
    parser.add_argument("bench_args", nargs="*", help="bench arguments, after `--`")
    args = parser.parse_args()

    if not args.base:
        parser.error("--base must name a revision, e.g. `make bench_compare_apply_delays BASE=<rev>`")
    if args.apply_delays and args.bench_args:
        parser.error("--apply-delays takes no bench arguments")
    if not args.apply_delays and "--latency" not in args.bench_args:
        parser.error("bench arguments must include --latency, which makes bench print a JSON report")
    if args.runs < 1:
        parser.error("--runs must be positive")
    return args


def git(cwd, *args):
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f"git {shlex.join(args)} failed:\n{result.stderr.strip()}")
    return result.stdout.strip()


def cargo_build(bench_dir, features):
    command = ["cargo", "build", "--release", "--bin", "bench", *features]
    subprocess.run(command, cwd=bench_dir, check=True, stdout=sys.stderr)


def build_revision(root, revision, features):
    """Build `bench` at a commit in a temporary worktree; return its label and binary path."""
    commit = git(root, "rev-parse", "--verify", f"{revision}^{{commit}}")
    subject = git(root, "log", "-1", "--format=%s", commit)
    key = commit + "".join(features)
    binary = CACHE_DIR / key / "bench"

    if binary.exists():
        print(f"using cached bench of {commit[:8]}: {binary}", file=sys.stderr)
    else:
        print(f"building bench of {commit[:8]} in a temporary worktree", file=sys.stderr)
        with tempfile.TemporaryDirectory(prefix="bench-compare-") as tmp:
            worktree = Path(tmp) / "openraft"
            git(root, "worktree", "add", "--detach", str(worktree), commit)
            try:
                bench_dir = worktree / "benchmarks" / "minimal"
                cargo_build(bench_dir, features)
                copy_binary(bench_dir / "target" / "release" / "bench", binary)
            finally:
                git(root, "worktree", "remove", "--force", str(worktree))

    return f"{commit[:8]} {subject}", binary


def build_checkout(root, features):
    """Build `bench` in the current checkout; return its label and binary path."""
    print("building bench of the current checkout", file=sys.stderr)
    cargo_build(BENCH_DIR, features)
    # Copy it, so that a build in the checkout during the runs does not replace it.
    binary = CACHE_DIR / "checkout" / "bench"
    copy_binary(BENCH_DIR / "target" / "release" / "bench", binary)

    commit = git(root, "rev-parse", "HEAD")
    subject = git(root, "log", "-1", "--format=%s", commit)
    dirty = git(root, "status", "--porcelain", "--untracked-files=no")
    changes = " + uncommitted changes" if dirty else ""
    return f"checkout {commit[:8]}{changes} {subject}", binary


def copy_binary(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    partial = destination.with_suffix(".partial")
    shutil.copy2(source, partial)
    partial.replace(destination)


def run_all(builds, rows, runs, raw_path):
    """Run every row with every build; return the measurements keyed by (build, row)."""
    results = {(build, label): [] for build, _ in builds for label, _ in rows}
    raw = raw_path.open("w") if raw_path else None
    for run in range(runs):
        order = builds if run % 2 == 0 else builds[::-1]
        for label, bench_args in rows:
            for build, binary in order:
                print(f"run {run + 1}/{runs}: {label}: {build}", file=sys.stderr, flush=True)
                measured = run_bench(binary, bench_args)
                results[(build, label)].append(measured)
                if raw:
                    line = {"build": build, "row": label, "run": run + 1, **measured}
                    print(json.dumps(line), file=raw, flush=True)
    return results


def run_bench(binary, bench_args):
    command = [str(binary), *bench_args]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f"bench failed: {shlex.join(command)}\n{result.stderr[-2000:]}")
    report = json.loads(result.stdout.strip().splitlines()[-1])
    return {field: report[field] / divisor for _, field, divisor in METRICS}


def print_table(rows, base, target, results):
    headers = ["row"]
    for name, _, _ in METRICS:
        headers += [f"{name} base", f"{name} target", "change"]
    lines = []
    for label, _ in rows:
        cells = [label]
        for _, field, _ in METRICS:
            base_values = [run[field] for run in results[(base, label)]]
            target_values = [run[field] for run in results[(target, label)]]
            cells += [f"{statistics.median(base_values):,.0f}", f"{statistics.median(target_values):,.0f}"]
            cells.append(change(base_values, target_values))
        lines.append(cells)

    widths = [max(len(header), *(len(line[i]) for line in lines)) for i, header in enumerate(headers)]
    print("| " + " | ".join(header.ljust(width) for header, width in zip(headers, widths)) + " |")
    aligns = [":--"] + ["--:"] * (len(headers) - 1)
    print("| " + " | ".join(align.ljust(width) for align, width in zip(aligns, widths)) + " |")
    for line in lines:
        cells = [line[0].ljust(widths[0])] + [cell.rjust(width) for cell, width in zip(line[1:], widths[1:])]
        print("| " + " | ".join(cells) + " |")
    print()
    print("Medians of the runs. `~` marks a change whose min-max range overlaps the other build's: it is")
    print("within run-to-run noise.")


def change(base_values, target_values):
    base = statistics.median(base_values)
    target = statistics.median(target_values)
    percent = f"{(target - base) / base * 100:+.1f}%" if base else "n/a"
    overlap = min(target_values) <= max(base_values) and min(base_values) <= max(target_values)
    return f"~{percent}" if overlap else percent


def cpu_name():
    if platform.system() == "Darwin":
        result = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True)
        return result.stdout.strip()
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.exists():
        for line in cpuinfo.read_text().splitlines():
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    return platform.processor() or "unknown CPU"


if __name__ == "__main__":
    main()

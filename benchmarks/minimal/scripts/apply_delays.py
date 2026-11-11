#!/usr/bin/env python3
"""Run the apply-path delay grid of `bench` and print the README table.

Builds `bench` in release mode, then runs every combination of client load and
apply-path delay `--runs` times under `/usr/bin/time` (`-l` on macOS, `-v` on
Linux). Prints one markdown row per combination with the median of the runs.
The defaults reproduce the "Apply-path delays" section of
`benchmarks/minimal/README.md`.

Usage:
    python3 benchmarks/minimal/scripts/apply_delays.py [--runs N] [--raw FILE]
"""

import argparse
import json
import platform
import re
import statistics
import subprocess
import sys
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent.parent
BENCH = BENCH_DIR / "target" / "release" / "bench"

# (read delay µs, I/O delay µs) of each apply batch.
DELAYS = [(0, 0), (500, 0), (0, 500), (500, 500), (1000, 0), (0, 1000)]

# (write clients, write operations) of each run.
LOADS = [(256, "30k"), (1, "2k")]

COLUMNS = [
    ("clients", "clients"),
    ("read µs", "read_us"),
    ("I/O µs", "io_us"),
    ("writes/s", "ops_per_s"),
    ("p50 µs", "p50_us"),
    ("p99 µs", "p99_us"),
    ("apply cmds", "apply_cmds"),
    ("entries/apply p50, p90", "apply_batch"),
    ("max RSS MB", "max_rss_mb"),
    ("invol. ctx switches", "invol_csw"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--runs", type=int, default=3, help="runs per combination [default: 3]")
    parser.add_argument("--raw", type=Path, help="also write every run as one JSON line to this file")
    args = parser.parse_args()

    build = ["cargo", "build", "--release", "--bin", "bench"]
    subprocess.run(build, cwd=BENCH_DIR, check=True)

    raw = args.raw.open("w") if args.raw else None
    rows = []
    for clients, operations in LOADS:
        for read_us, io_us in DELAYS:
            runs = []
            for i in range(args.runs):
                label = f"clients={clients} read={read_us}us io={io_us}us run={i + 1}/{args.runs}"
                print(label, file=sys.stderr, flush=True)
                run = run_bench(clients, operations, read_us, io_us)
                runs.append(run)
                if raw:
                    print(json.dumps(run), file=raw, flush=True)
            rows.append(median_row(runs))

    print_table(rows)


def run_bench(clients, operations, read_us, io_us):
    """Run `bench` once and return its measurements."""
    time_flag = "-l" if platform.system() == "Darwin" else "-v"
    command = [
        "/usr/bin/time", time_flag, str(BENCH), "--latency",
        "-c", str(clients), "-n", operations,
        "--apply-read-delay-us", str(read_us), "--apply-io-delay-us", str(io_us),
    ]  # fmt: skip
    result = subprocess.run(command, capture_output=True, text=True, check=True)
    report = json.loads(result.stdout.strip().splitlines()[-1])
    stderr = result.stderr

    # The runtime-stats feature prints the final stats after "Final:".
    final = stderr[stderr.rindex("Final:") :]
    apply_cells = re.search(r"│ Apply\s+│(.*)│ Entries", final).group(1).split("│")
    apply_cmds = re.search(r"SaveCommittedAndApply\s*│\s*([\d,]+)", final).group(1)

    return {
        "clients": clients,
        "read_us": read_us,
        "io_us": io_us,
        "ops_per_s": round(report["operations_per_second"]),
        "p50_us": report["request_p50_ns"] // 1000,
        "p99_us": report["request_p99_ns"] // 1000,
        "apply_cmds": int(apply_cmds.replace(",", "")),
        # Cells: samples, P0.1, P1, P5, P10, P50, P90, ...
        "apply_batch_p50": int(apply_cells[5]),
        "apply_batch_p90": int(apply_cells[6]),
        "max_rss_mb": max_rss_mb(stderr),
        "invol_csw": involuntary_context_switches(stderr),
        "seconds": report["seconds"],
    }


def max_rss_mb(time_output):
    """Max RSS in MiB: macOS reports bytes, GNU time reports KiB."""
    mac = re.search(r"(\d+)\s+maximum resident set size", time_output)
    if mac:
        return round(int(mac.group(1)) / 2**20, 1)
    gnu = re.search(r"Maximum resident set size \(kbytes\): (\d+)", time_output)
    return round(int(gnu.group(1)) / 2**10, 1)


def involuntary_context_switches(time_output):
    mac = re.search(r"(\d+)\s+involuntary context switches", time_output)
    if mac:
        return int(mac.group(1))
    gnu = re.search(r"Involuntary context switches: (\d+)", time_output)
    return int(gnu.group(1))


def median_row(runs):
    """Combine the runs of one combination into a table row of medians."""
    row = {key: statistics.median(run[key] for run in runs) for key in runs[0]}
    batch_p50 = round(row.pop("apply_batch_p50"))
    batch_p90 = round(row.pop("apply_batch_p90"))
    row["apply_batch"] = f"{batch_p50}, {batch_p90}"
    return row


def print_table(rows):
    cells = [[format_cell(key, row[key]) for _, key in COLUMNS] for row in rows]
    headers = [header for header, _ in COLUMNS]
    widths = [max(len(header), *(len(line[i]) for line in cells)) for i, header in enumerate(headers)]
    print("| " + " | ".join(header.ljust(width) for header, width in zip(headers, widths)) + " |")
    print("| " + " | ".join("--:".ljust(width) for width in widths) + " |")
    for line in cells:
        print("| " + " | ".join(cell.rjust(width) for cell, width in zip(line, widths)) + " |")


def format_cell(key, value):
    if key == "apply_batch":
        return value
    if key == "max_rss_mb":
        return f"{value:.1f}"
    return f"{round(value):,}"


if __name__ == "__main__":
    main()

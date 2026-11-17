#!/usr/bin/env python3
"""Run a rotated apply-delay grid with complete artifacts and captured build inputs.

The default exact counts preserve historical compatibility. Use --seconds 30
for a steady window, --scenario to select rows, and --diagnostics for separate
instrumented companions. Performance runs disable runtime-stats.
"""

import argparse

from benchmark_build import BuildRequest
from benchmark_runner import Plan, Suite, parse_operations
from benchmark_scenarios import Mode, Scenario, add_arguments, selected_features, selected_scenarios, validate

# (read delay µs per batch, I/O delay µs per `apply()` call, delay µs per applied entry).
DELAYS = [
    (0, 0, 0), (500, 0, 0), (0, 500, 0), (500, 500, 0), (1000, 0, 0), (0, 1000, 0),
    (0, 0, 10), (0, 500, 10),
]  # fmt: skip

# (write clients, write operations) of each run.
LOADS = [(256, "29952"), (1, "2k")]


def scenarios():
    return [Scenario(f"clients={clients}/read={read}/io={io}/entry={entry}", clients, parse_operations(operations),
                     read_us=read, io_us=io, entry_us=entry)
            for clients, operations in LOADS for read, io, entry in DELAYS]


def grid_args(clients, operations, read_us, io_us, entry_us):
    """Legacy count arguments, without options absent in historical binaries."""
    flags = ["--latency", "-c", str(clients), "-n", str(operations),
             "--apply-read-delay-us", str(read_us), "--apply-io-delay-us", str(io_us)]
    if entry_us:
        flags += ["--apply-entry-delay-us", str(entry_us)]
    return flags


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    args = parser.parse_args()
    validate(args, parser)
    rows = selected_scenarios(args, scenarios())
    request = BuildRequest("target", features=selected_features(args), lockfile=args.lockfile, toolchain=args.toolchain)
    prebuilt = {"target": args.binary_manifest} if args.binary_manifest else {}
    plan = Plan(rows, [Mode("current")], {"target": request}, prebuilt=prebuilt)
    Suite(args, plan).run()


if __name__ == "__main__":
    main()

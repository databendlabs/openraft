#!/usr/bin/env python3
"""Compare revisions with captured locks, immutable binaries and paired uncertainty.

Performance builds disable runtime-stats. Cache entries live outside the checkout
and include actual source, toolchain, flags/configuration and lockfile identity.
Use --apply-delays for the shared grid, or pass exact bench arguments after --.
Examples:
    compare.py --base HEAD~1 -- --latency -c 256 -n 29952
    compare.py --base HEAD~1 --apply-delays --seconds 30 --warmup-seconds 5
"""

import argparse
import sys
from pathlib import Path

from apply_delays import scenarios
from benchmark_build import BuildRequest
from benchmark_runner import Plan, Suite
from benchmark_scenarios import Mode, Scenario, add_arguments, selected_features, selected_scenarios, validate


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    controls = add_arguments(parser)
    parser.add_argument("--base", required=True)
    parser.add_argument("--target", help="default: current checkout including uncommitted source")
    parser.add_argument("--apply-delays", action="store_true")
    parser.add_argument("--no-default-features", action="store_true", help="compatibility option; now the default")
    for side in ("base", "target"):
        parser.add_argument(f"--{side}-lockfile", type=Path)
        parser.add_argument(f"--{side}-toolchain")
        parser.add_argument(f"--{side}-binary-manifest", type=Path)
    parser.add_argument("bench_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    outer = sys.argv[1:sys.argv.index("--")] if "--" in sys.argv else sys.argv[1:]
    args.outer_workload_controls = sorted({item.split("=", 1)[0] for item in outer} & controls)
    validate(args, parser)
    validate_comparison(args, parser)
    return args


def validate_comparison(args, parser):
    if args.bench_args[:1] == ["--"]:
        args.bench_args = args.bench_args[1:]
    if args.apply_delays and args.bench_args:
        parser.error("--apply-delays takes no bench arguments")
    if not args.apply_delays and "--latency" not in args.bench_args:
        parser.error("bench arguments after -- must include --latency")
    if not args.apply_delays and args.outer_workload_controls:
        parser.error("pass custom workload controls after --: " + ", ".join(args.outer_workload_controls))
    if args.binary_manifest:
        parser.error("use --base-binary-manifest and --target-binary-manifest")


def build_requests(args):
    requests = {}
    for side in ("base", "target"):
        lock = getattr(args, side + "_lockfile") or args.lockfile
        toolchain = getattr(args, side + "_toolchain") or args.toolchain
        requests[side] = BuildRequest(side, revision=getattr(args, side), features=selected_features(args),
                                      lockfile=lock, toolchain=toolchain)
    return requests


def main():
    args = parse_args()
    rows = selected_scenarios(args, scenarios()) if args.apply_delays else [Scenario("custom", 0, 0)]
    modes = [Mode("base", build="base"), Mode("target", build="target")]
    prebuilt = {side: getattr(args, side + "_binary_manifest") for side in ("base", "target")
                if getattr(args, side + "_binary_manifest") is not None}
    custom = None if args.apply_delays else args.bench_args
    plan = Plan(rows, modes, build_requests(args), custom_args=custom, prebuilt=prebuilt)
    Suite(args, plan).run()


if __name__ == "__main__":
    main()

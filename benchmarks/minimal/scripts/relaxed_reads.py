#!/usr/bin/env python3
"""Compare core, local, relaxed and follower reads in paired, rotated rounds.

Use --seconds for steady windows. Ticks and heartbeats renew read-only leases.
Raw commands, complete JSON, failures and per-round paired statistics are saved
before tables. Runtime-statistics companions are separate diagnostic groups.
"""

import argparse

from benchmark_build import BuildRequest
from benchmark_runner import Plan, Suite
from benchmark_scenarios import Mode, Scenario, add_arguments, selected_features, selected_scenarios, validate

MODES = [
    # (name, enable_local_lease_read, enable_relaxed_read, extra bench arguments)
    ("core", False, False, []),
    ("local", True, False, []),
    ("relaxed", True, True, ["--read-relaxed"]),
]
# Standard follower reads: the leader issues a cached LeaseRead linearizer, and follower 1 waits
# for it and runs the query. "bound" only enables the follower apply bound.
FOLLOWER_MODES = [
    ("follower", True, False, ["--read-on-follower", "1"]),
    ("follower+bound", True, True, ["--read-on-follower", "1"]),
]
SCENARIOS = [
    # (label, write clients, writes, read clients, reads, read delay us, I/O delay us)
    ("read-only/1", 0, 0, 1, 20000, 0, 0),
    ("read-only/64", 0, 0, 64, 200000, 0, 0),
    ("mixed/no-delay", 32, 20000, 64, 180000, 0, 0),
    ("mixed/log-read-500us", 32, 20000, 64, 180000, 500, 0),
    ("mixed/apply-io-500us", 32, 20000, 64, 180000, 0, 500),
]
# Followers trail the leader, and the bound is re-sent, only while writes arrive.
MIXED_SCENARIOS = [scenario for scenario in SCENARIOS if scenario[1]]
# "+bound" modes enable the follower apply bound without requesting relaxed reads.
BOUND_MODES = [
    ("core", False, False, []),
    ("core+bound", False, True, []),
    ("local", True, False, []),
    ("local+bound", True, True, []),
    ("relaxed", True, True, ["--read-relaxed"]),
]
# The paced mixed workload at each apply delay; 0 µs is shared by both delay kinds.
SWEEP_SCENARIOS = [("mixed/no-delay", 32, 20000, 64, 180000, 0, 0)] + [
    (f"mixed/{kind}-{delay}us", 32, 20000, 64, 180000, delay if kind == "log-read" else 0,
     delay if kind == "apply-io" else 0)
    for kind in ["log-read", "apply-io"]
    for delay in [100, 500, 2000]
]
# Leadership moves one second into the workload; clients follow it.
TRANSFER = ["--transfer-leader-after-ms", "1000"]
LEADER_CHANGE_MODES = [
    ("local", True, False, TRANSFER),
    ("local+bound", True, True, TRANSFER),
    ("relaxed", True, True, ["--read-relaxed"] + TRANSFER),
]
# 960 writes per writer and 4,320 reads per reader: 3.07 s at the paced rates.
LEADER_CHANGE_SCENARIOS = [
    ("mixed/log-read-500us/3s", 32, 30720, 64, 276480, 500, 0),
    ("mixed/apply-io-500us/3s", 32, 30720, 64, 276480, 0, 500),
]
# ReadIndex reads always run in RaftCore: only lease reads use the cache.
READ_INDEX_MODES = [
    ("read-index", False, False, ["--read-policy", "read-index"]),
    ("read-index+relaxed", False, True, ["--read-policy", "read-index", "--read-relaxed"]),
]
# Unpaced write-only runs; each client count divides its writes evenly.
WRITE_MODES = [("bound-off", False, False, []), ("bound-on", False, True, [])]
WRITE_SCENARIOS = [
    (f"write-only/{clients}{delay}", clients, writes, 0, 0, read_delay, io_delay)
    for clients, writes in [(1, 5000), (32, 20000), (256, 51200)]
    for delay, read_delay, io_delay in [("", 0, 0), ("/log-read-500us", 500, 0), ("/apply-io-500us", 0, 500)]
]
GRIDS = {
    "leader": (MODES, SCENARIOS),
    "follower": (FOLLOWER_MODES, MIXED_SCENARIOS),
    "writes": (WRITE_MODES, WRITE_SCENARIOS),
    "bound": (BOUND_MODES, MIXED_SCENARIOS),
    "read-index": (READ_INDEX_MODES, SCENARIOS),
    "sweep": (MODES, SWEEP_SCENARIOS),
    "leader-change": (LEADER_CHANGE_MODES, LEADER_CHANGE_SCENARIOS),
}


def modes_for(args, modes):
    result = []
    for name, local, relaxed, extra in modes:
        extra = list(extra)
        if args.seconds is not None and "--transfer-leader-after-ms" in extra:
            if args.seconds <= 1:
                raise ValueError("leader-change measurement must exceed one second")
            index = extra.index("--transfer-leader-after-ms") + 1
            extra[index] = str(round((args.warmup_seconds + 1) * 1000))
        config = {"enable_local_lease_read": local, "enable_relaxed_read": relaxed}
        result.append(Mode(name, config, tuple(extra)))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    parser.add_argument("--grid", choices=GRIDS, default="leader")
    parser.add_argument("--skip-build", action="store_true", help="requires --binary-manifest for verified inputs")
    args = parser.parse_args()
    validate(args, parser)
    if args.skip_build and args.binary_manifest is None:
        parser.error("--skip-build requires --binary-manifest; mutable target binaries have no verified inputs")
    modes, rows = GRIDS[args.grid]
    scenarios = selected_scenarios(args, [Scenario(*row) for row in rows])
    request = BuildRequest("target", features=selected_features(args), lockfile=args.lockfile, toolchain=args.toolchain)
    prebuilt = {"target": args.binary_manifest} if args.binary_manifest else {}
    plan = Plan(scenarios, modes_for(args, modes), {"target": request}, prebuilt=prebuilt)
    Suite(args, plan).run()


if __name__ == "__main__":
    main()

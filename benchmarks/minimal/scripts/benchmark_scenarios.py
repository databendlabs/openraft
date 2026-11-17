"""Shared workload arguments; count controls and steady windows use the same grids."""

import dataclasses
import json
import math
from pathlib import Path

from benchmark_common import DEFAULT_CACHE


@dataclasses.dataclass
class Scenario:
    label: str
    writers: int
    writes: int
    readers: int = 0
    reads: int = 0
    read_us: int = 0
    io_us: int = 0
    entry_us: int = 0
    extra: tuple = ()


@dataclasses.dataclass
class Mode:
    name: str
    config: dict = dataclasses.field(default_factory=dict)
    extra: tuple = ()
    build: str = "target"


def artifact_arguments(parser):
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--output", type=Path, help="new artifact directory; default: persistent cache/runs")
    parser.add_argument("--raw", type=Path, help="additional full-record JSONL file; must be new")
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--timeout", type=float, default=420)
    parser.add_argument("--build-timeout", type=float, default=1800)
    parser.add_argument("--diagnostics", action="store_true", help="also run a separate runtime-stats build")
    parser.add_argument("--features", default="", help="extra compiled features; runtime-stats is diagnostic only")
    parser.add_argument("--toolchain", help="explicit compiler override; otherwise resolve each source independently")
    parser.add_argument("--lockfile", type=Path, help="captured Cargo.lock; otherwise capture/generate and freeze one")
    parser.add_argument("--binary-manifest", type=Path, help="reuse a verified immutable cache manifest")


def workload_arguments(parser):
    parser.add_argument("--seconds", type=float, help="fixed measurement window; omit for exact-count controls")
    parser.add_argument("--warmup-seconds", type=float, default=5)
    parser.add_argument("--drain-seconds", type=float, default=5)
    parser.add_argument("--warmup-writes", type=int, default=128)
    parser.add_argument("--operations", type=int)
    parser.add_argument("--read-operations", type=int)
    parser.add_argument("--clients", type=int)
    parser.add_argument("--read-clients", type=int)
    parser.add_argument("--write-rate", type=int)
    parser.add_argument("--read-rate", type=int)
    parser.add_argument("--client-workers", type=int, default=1)
    parser.add_argument("--server-workers", type=int, default=16)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--batch", type=int, default=1)
    parser.add_argument("--members", type=int, choices=(1, 3, 5), default=3)
    parser.add_argument("--raft-config", default="{}")
    parser.add_argument("--scenario", action="append", help="run only the named scenario (repeatable)")
    parser.add_argument("--lease-expiry-control", action="store_true", help="allow explicit disabled heartbeat/ticking")


def arrival_arguments(parser):
    parser.add_argument("--open-loop", action="store_true")
    parser.add_argument("--arrival-pattern", choices=("uniform", "poisson", "burst"))
    parser.add_argument("--queue-capacity", type=int)
    parser.add_argument("--request-timeout-ms", type=int)
    parser.add_argument("--phase-sample-every", type=int)


def profile_arguments(parser):
    parser.add_argument("--profile", action="store_true")
    for name in ("payload-bytes", "profile-keys", "hot-key-percent", "cache-entries", "cache-miss-us", "cpu-rounds",
                 "log-read-bytes-per-second", "apply-bytes-per-second", "link-bytes-per-second", "link-capacity",
                 "network-jitter-us", "follower-delay-us"):
        parser.add_argument("--" + name, type=int)
    parser.add_argument("--concurrent-queries", action="store_true")
    parser.add_argument("--serialized-transport", action="store_true")


def add_arguments(parser):
    artifact_arguments(parser)
    existing = set(parser._option_string_actions)
    workload_arguments(parser)
    profile_arguments(parser)
    arrival_arguments(parser)
    return set(parser._option_string_actions) - existing


def validate(args, parser):
    positive = [args.runs, args.timeout, args.build_timeout, args.client_workers, args.server_workers, args.batch]
    positive += [value for value in [args.seconds] if value is not None]
    if any(not math.isfinite(value) or value <= 0 for value in positive):
        parser.error("rounds, timeouts, threads, batch and measurement duration must be positive")
    optional = [args.clients, args.read_clients, args.operations, args.read_operations, args.write_rate, args.read_rate]
    nonnegative = [args.warmup_seconds, args.drain_seconds, args.warmup_writes, args.seed]
    if any(not math.isfinite(value) or value < 0 for value in nonnegative + [v for v in optional if v is not None]):
        parser.error("counts, rates, warmup, drain and seed must be finite and nonnegative")
    if "runtime-stats" in args.features.split(","):
        parser.error("use --diagnostics for separate runtime-statistics runs")
    if not isinstance(json.loads(args.raft_config), dict):
        parser.error("raft-config must be a JSON object")
    if args.raw and args.raw.exists():
        parser.error("raw output must be new")


PROFILE_VALUES = ("payload_bytes", "profile_keys", "hot_key_percent", "cache_entries", "cache_miss_us", "cpu_rounds",
                  "log_read_bytes_per_second", "apply_bytes_per_second", "link_bytes_per_second", "link_capacity",
                  "network_jitter_us", "follower_delay_us")


def selected_scenarios(args, scenarios):
    if not args.scenario:
        return scenarios
    selected = [scenario for scenario in scenarios if scenario.label in args.scenario]
    missing = set(args.scenario) - {scenario.label for scenario in selected}
    if missing:
        raise ValueError(f"unknown scenarios: {sorted(missing)}")
    return selected


def selected_features(args):
    features = set(filter(None, args.features.split(",")))
    if args.profile or "--profile" in getattr(args, "bench_args", []):
        features.add("realistic-profiles")
    return tuple(sorted(features))


def profile_flags(args):
    selected = {name: getattr(args, name) for name in PROFILE_VALUES if getattr(args, name) is not None}
    switches = [name for name in ("concurrent_queries", "serialized_transport") if getattr(args, name)]
    if not args.profile and (selected or switches):
        raise ValueError("payload/storage/transport controls require --profile")
    flags = ["--profile"] if args.profile else []
    for name, value in selected.items():
        if value < 0:
            raise ValueError(f"{name} must be nonnegative")
        flags += ["--" + name.replace("_", "-"), str(value)]
    return flags + ["--" + name.replace("_", "-") for name in switches]


def effective_counts(args, scenario):
    writers = args.clients if args.clients is not None and scenario.writers else scenario.writers
    readers = args.read_clients if args.read_clients is not None and scenario.readers else scenario.readers
    writes = args.operations if args.operations is not None and scenario.writers else scenario.writes
    reads = args.read_operations if args.read_operations is not None and scenario.readers else scenario.reads
    if (writers == 0 and writes) or (readers == 0 and reads):
        raise ValueError("positive counts require clients; override both count and concurrency")
    if writes % args.batch:
        raise ValueError("write count must be divisible by batch")
    return writers, writes, readers, reads


def raft_config(args, mode, readers):
    config = {"enable_tick": True, "enable_heartbeat": True, "enable_elect": True}
    overrides = json.loads(args.raft_config)
    if set(overrides) & set(mode.config):
        raise ValueError("raft-config cannot override fields selected by a comparison mode")
    config.update(overrides)
    config.update(mode.config)
    if readers and (not config["enable_tick"] or not config["enable_heartbeat"]) and not args.lease_expiry_control:
        raise ValueError("read runs need lease renewal; use --lease-expiry-control for explicit expiry tests")
    return config


def workload_flags(args, scenario, mode):
    writers, writes, readers, reads = effective_counts(args, scenario)
    config = raft_config(args, mode, readers)
    flags = ["--latency", "--members", str(args.members), "--clients", str(writers), "--operations", str(writes), "--batch", str(args.batch),
             "--warmup-writes", str(args.warmup_writes), "--client-workers", str(args.client_workers),
             "--server-workers", str(args.server_workers), "--raft-config", json.dumps(config),
             "--apply-read-delay-us", str(scenario.read_us), "--apply-io-delay-us", str(scenario.io_us)]
    if scenario.entry_us:
        flags += ["--apply-entry-delay-us", str(scenario.entry_us)]
    if readers:
        flags += ["--read-clients", str(readers), "--read-operations", str(reads), "--read-query"]
    flags += rate_flags(args, writers, readers)
    if args.seconds is not None:
        flags += ["--measurement-seconds", str(args.seconds), "--warmup-seconds", str(args.warmup_seconds),
                  "--drain-seconds", str(args.drain_seconds)]
    return flags + profile_flags(args) + arrival_flags(args) + list(scenario.extra) + list(mode.extra)


def arrival_flags(args):
    flags = ["--open-loop"] if args.open_loop else []
    for name in ("arrival_pattern", "queue_capacity", "request_timeout_ms", "phase_sample_every"):
        value = getattr(args, name)
        if value is not None:
            if isinstance(value, int) and value < 0:
                raise ValueError(f"{name} must be nonnegative")
            flags += ["--" + name.replace("_", "-"), str(value)]
    return flags


def rate_flags(args, writers, readers):
    flags = []
    mixed = bool(writers and readers)
    for name, default, clients in (("write_rate", 10000, writers), ("read_rate", 90000, readers)):
        if not clients:
            continue
        value = getattr(args, name)
        if value is None and mixed:
            value = default
        if value is not None:
            flags += ["--" + name.replace("_", "-"), str(value)]
    return flags

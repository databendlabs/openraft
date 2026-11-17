"""Run rotated paired experiments, retaining full inputs and failures before summaries."""

import dataclasses
import json
import math
import platform
import re
import statistics
import sys
from pathlib import Path

from benchmark_build import BuildCache, verified_entry
from benchmark_common import Artifacts, Command, HostMonitor, digest, file_hash, json_bytes, paired_change, require_success, schedule
from benchmark_scenarios import workload_flags

OUTCOMES = ("completed", "failed", "timed_out", "rejected", "pending", "missed")


def parse_operations(value):
    text = str(value).replace("_", "").lower()
    suffixes = {"k": 1000, "m": 1000000, "g": 1000000000}
    if text and text[-1] in suffixes:
        return int(text[:-1]) * suffixes[text[-1]]
    return int(text)


def option(command, names):
    for index, argument in enumerate(command):
        if argument in names:
            return command[index + 1]
        for name in names:
            if argument.startswith(name + "="):
                return argument[len(name) + 1:]
    return None


def validate_timing(timing):
    if not timing:
        return
    counts = [timing[name] for name in OUTCOMES]
    if any(not isinstance(value, int) or value < 0 for value in counts):
        raise ValueError("operation outcomes must be nonnegative integers")
    if sum(counts) != timing["offered"]:
        raise ValueError("offered operations do not equal all outcomes")
    if timing["admitted"] != timing["offered"] - timing["rejected"] - timing["missed"]:
        raise ValueError("admitted operation count does not conserve offered operations")
    if timing["failed"]:
        raise ValueError("benchmark reported failed operations")


def validate_report(report, command, diagnostic):
    if report.get("schema_version", 1) not in (1, 2):
        raise ValueError("unsupported benchmark JSON schema")
    if report.get("runtime_stats", diagnostic) != diagnostic:
        raise ValueError("runtime-statistics mode differs from this experiment group")
    writes = report.get("write", report)
    timing = report.get("write_timing", report.get("timing"))
    validate_timing(timing)
    if timing and writes["operations"] != timing["completed"]:
        raise ValueError("completed write count disagrees with outcome accounting")
    timed = option(command, ("--measurement-seconds",)) is not None
    if not timed:
        requested = option(command, ("-n", "--operations"))
        expected = parse_operations(requested) if requested is not None else report.get("requested_operations")
        accounted = timing["offered"] if timing else writes["operations"]
        if expected is not None and accounted != expected:
            raise ValueError(f"accounted for {accounted} writes; requested {expected}")
    validate_reads(report, command, timed)


def validate_reads(report, command, timed):
    if "read" not in report:
        return
    reads = report["read"]
    validate_timing(reads.get("timing"))
    if reads["failed"]["operations"]:
        raise ValueError(f"read failures: {reads.get('errors')}")
    requested = option(command, ("--read-operations",))
    accounted = reads["timing"]["offered"] if reads.get("timing") else reads["successful"]["operations"]
    if requested is not None and not timed and accounted != parse_operations(requested):
        raise ValueError("benchmark did not account for exactly the requested reads")


def measurements(report):
    writes = report.get("write", report)
    latency = writes.get("latency") or {}
    result = {"writes/s": writes["operations_per_second"],
              "write p50 ns": latency.get("p50_ns", report.get("request_p50_ns")),
              "write p99 ns": latency.get("p99_ns", report.get("request_p99_ns"))}
    if "read" in report:
        reads = report["read"]["successful"]
        read_latency = reads.get("latency") or {}
        result.update({"reads/s": reads["operations_per_second"], "read p50 ns": read_latency.get("p50_ns"),
                       "read p99 ns": read_latency.get("p99_ns")})
    result.update(apply_measurements(report))
    return result


def apply_measurements(report):
    metrics = report.get("apply_metrics") or {}
    initial_leader = metrics.get("0")
    if initial_leader is None:
        return {}
    batches = initial_leader["entries_per_apply"] or {}
    result = {"node 0 apply calls": initial_leader["apply_calls"], "node 0 entries/apply p50": batches.get("p50"),
              "node 0 entries/apply p90": batches.get("p90")}
    commands = report.get("command_counts")
    if commands is not None:
        result["SaveCommittedAndApply commands"] = commands["counts"].get("SaveCommittedAndApply", 0)
    return result


def resource_metrics(stderr):
    mac = re.search(r"(\d+)\s+maximum resident set size", stderr)
    gnu = re.search(r"Maximum resident set size \(kbytes\): (\d+)", stderr)
    rss = int(mac[1]) if mac else int(gnu[1]) * 1024 if gnu else None
    return {"peak_rss_bytes": rss, "scope": "whole process including bootstrap and reporting"}


def final_report(directory):
    lines = (directory / "stdout").read_text().strip().splitlines()
    if not lines:
        raise ValueError("benchmark produced no JSON report")
    report = json.loads(lines[-1])
    if not isinstance(report, dict):
        raise ValueError("benchmark report must be an object")
    return report


def capabilities(build, directory):
    command = Command([build["binary"], "--help"], timeout=30)
    record = command.run(directory)
    require_success(record, directory)
    return set(re.findall(r"--[a-z][a-z0-9-]*", (directory / "stdout").read_text()))


def checked_flags(flags, supported):
    unsupported = {value.split("=", 1)[0] for value in flags if value.startswith("--")} - supported
    if unsupported:
        raise ValueError(f"binary does not support requested controls: {sorted(unsupported)}")
    return flags


@dataclasses.dataclass
class Plan:
    rows: list
    modes: list
    builds: dict
    custom_args: list = None
    prebuilt: dict = dataclasses.field(default_factory=dict)


class Suite:
    def __init__(self, args, plan):
        self.args, self.plan = args, plan
        self.artifacts = Artifacts(args)
        self.builds = {}
        self.supported = {}

    def build_all(self):
        cache = BuildCache(self.args)
        for diagnostic in [False, True] if self.args.diagnostics else [False]:
            for name, request in self.plan.builds.items():
                features = set(request.features)
                if diagnostic:
                    features.add("runtime-stats")
                selected = dataclasses.replace(request, features=tuple(sorted(features)))
                if diagnostic:
                    selected = self.diagnostic_request(selected, name)
                build = self.build_one(cache, selected, name, diagnostic)
                if diagnostic:
                    check_matching_builds(self.builds[(False, name)], build)
                key = (diagnostic, name)
                self.builds[key] = build
                self.artifacts.manifest["builds"] = {str(key): value for key, value in self.builds.items()}
                self.supported[key] = capabilities(build, self.artifacts.path / f"help-{int(diagnostic)}-{name}")
        self.artifacts.manifest["builds"] = {str(key): value for key, value in self.builds.items()}

    def diagnostic_request(self, request, name):
        performance = self.builds[(False, name)]
        inputs = performance["metadata"]["inputs"]
        lock = Path(performance["manifest"]).parent / "source" / inputs["crate"] / "Cargo.lock"
        return dataclasses.replace(request, lockfile=lock, toolchain=inputs["toolchain"]["selected"])

    def build_one(self, cache, request, name, diagnostic):
        prebuilt = self.plan.prebuilt.get(name)
        if prebuilt is None or diagnostic:
            return cache.build(request)
        manifest = json.loads(prebuilt.read_text())
        build = verified_entry(prebuilt.parent, manifest["inputs"])
        if sorted(request.features) != manifest["inputs"]["features"]:
            raise ValueError("prebuilt features differ from the requested experiment")
        check_prebuilt_inputs(request, manifest)
        if request.revision is not None:
            from benchmark_common import git
            revision = git(cache.root, "rev-parse", f"{request.revision}^{{commit}}")
            if manifest["inputs"]["source"]["revision"] != revision or manifest["inputs"]["source"]["dirty"]:
                raise ValueError("prebuilt source differs from the requested reference")
        return build

    def flags(self, row, mode, supported):
        flags = self.plan.custom_args
        if flags is None:
            flags = workload_flags(self.args, row, mode)
            if "--arrival-seed" in supported:
                flags += ["--arrival-seed", str(self.args.seed)]
            elif self.args.seed != 1:
                raise ValueError("binary cannot apply the requested arrival seed")
            if "--measure-apply" in supported:
                flags += ["--measure-apply"]
        return checked_flags(flags, supported)

    def measure(self, round_index, row, mode, diagnostic):
        key = (diagnostic, mode.build)
        build = self.builds[key]
        flags = self.flags(row, mode, self.supported[key])
        if file_hash(build["binary"]) != build["metadata"]["binary_sha256"]:
            raise ValueError("immutable benchmark binary changed")
        command = ["/usr/bin/time", "-l" if platform.system() == "Darwin" else "-v", build["binary"], *flags]
        record = {"schema_version": 1, "round": round_index, "scenario": row.label, "mode": mode.name,
                  "group": "diagnostic" if diagnostic else "performance", "build": build, "command": command,
                  "requested_statistics_seed": self.args.seed, "status": "starting"}
        directory = self.artifacts.path / f"run-{len(self.artifacts.records):05}"
        record["artifact_directory"] = str(directory)
        print(f"{record['group']} round {round_index}/{self.args.runs}: {row.label}: {mode.name}", file=sys.stderr, flush=True)
        return self.execute(record, directory, diagnostic)

    def execute(self, record, directory, diagnostic):
        try:
            with HostMonitor(directory.with_suffix(".host.jsonl")) as monitor:
                result = Command(record["command"], timeout=self.args.timeout).run(directory)
            record.update(execution=result, environment=monitor.report())
            require_success(result, directory)
            report = final_report(directory)
            record["report"] = report
            record["workload_seed"] = workload_seed(report, record["command"])
            validate_report(report, record["command"], diagnostic)
            record["runtime_identity"] = runtime_identity(record)
            record.update(status="success", measurements=measurements(report),
                          resources=resource_metrics((directory / "stderr").read_text()))
        except BaseException as error:
            record.update(status="failed", error={"type": type(error).__name__, "message": str(error)})
            raise
        finally:
            self.artifacts.record(record)
        return record

    def run(self):
        try:
            self.build_all()
            for diagnostic in [False, True] if self.args.diagnostics else [False]:
                for round_index, row, mode in schedule(self.plan.rows, self.plan.modes, self.args.runs):
                    self.measure(round_index, row, mode, diagnostic)
            summary = summarize(self.args, self.plan, self.artifacts.records)
        except BaseException as error:
            self.artifacts.finish({"status": "incomplete", "completed_runs": len(self.artifacts.records)},
                                  {"type": type(error).__name__, "message": str(error)})
            raise
        self.artifacts.finish(summary)
        print_summary(summary)
        print(f"Artifacts: {self.artifacts.path}", file=sys.stderr)
        return self.artifacts.path


def metric_summary(values):
    if not values or any(value is None or not math.isfinite(value) for value in values):
        return {"runs": values, "median": None, "range": None}
    return {"runs": values, "median": statistics.median(values), "range": [min(values), max(values)]}


def check_prebuilt_inputs(request, manifest):
    inputs = manifest["inputs"]
    if request.lockfile is not None and file_hash(request.lockfile) != inputs["lockfile_sha256"]:
        raise ValueError("prebuilt lockfile differs from the requested lockfile")
    if request.toolchain is not None and request.toolchain != inputs["toolchain"]["selected"]:
        raise ValueError("prebuilt toolchain differs from the requested toolchain")


def check_matching_builds(performance, diagnostic):
    left = dict(performance["metadata"]["inputs"])
    right = dict(diagnostic["metadata"]["inputs"])
    left.pop("features")
    right.pop("features")
    if left != right:
        raise ValueError("diagnostic source/toolchain/flags/lock differs from its performance companion")


def summarize(args, plan, records):
    groups = {}
    for record in records:
        key = (record["group"], record["scenario"])
        groups.setdefault(key, {}).setdefault(record["mode"], []).append(record)
    rows = []
    for (group, scenario), modes in groups.items():
        summary = summarize_modes(args, plan, modes)
        summary["decision_eligible"] = summary["decision_eligible"] and group == "performance"
        rows.append({"group": group, "scenario": scenario, **summary})
    return {"schema_version": 1, "rounds": args.runs, "rows": rows,
            "percentile_scope": "medians and paired changes of run-level percentiles; request samples are not pooled",
            "uncertainty": "seeded percentile bootstrap; short or contaminated groups are diagnostic only"}


def summarize_modes(args, plan, records):
    modes = {}
    eligible = args.runs >= 5
    reference = plan.modes[0].name
    for name, runs in records.items():
        ordered = sorted(runs, key=lambda item: item["round"])
        eligible = eligible and len(ordered) == args.runs and all(clean_run(run) for run in ordered)
        identities = {run["runtime_identity"] for run in ordered}
        eligible = eligible and len(identities) == 1
        metrics = {key: metric_summary([run["measurements"][key] for run in ordered]) for key in ordered[0]["measurements"]}
        modes[name] = {"metrics": metrics, "paired_to": reference, "changes": {}, "runtime_identities": sorted(identities)}
    for name, mode in modes.items():
        if name == reference:
            continue
        for key, metric in mode["metrics"].items():
            if key not in modes[reference]["metrics"]:
                continue
            base = modes[reference]["metrics"][key]["runs"]
            mode["changes"][key] = paired_change(base, metric["runs"], args.seed)
    return {"decision_eligible": eligible, "modes": modes}


def runtime_identity(record):
    report = record["report"]
    selected = {"binary_sha256": record["build"]["metadata"]["binary_sha256"],
                "build_inputs": record["build"]["metadata"]["inputs"], "command": record["command"],
                "compiled_features": report.get("compiled_features"), "resolved_raft_config": report.get("resolved_raft_config"),
                "workers": {name: report.get(name, report.get("config", {}).get(name))
                            for name in ("client_workers", "server_workers")}}
    return digest(json_bytes(selected))


def workload_seed(report, command):
    arrival = report.get("arrival", report.get("config", {}).get("arrival", {}))
    configured = option(command, ("--arrival-seed",))
    reported = arrival.get("arrival_seed")
    if configured is not None and reported is not None and int(configured) != reported:
        raise ValueError("reported arrival seed differs from the command")
    return int(configured) if configured is not None else reported


def clean_run(run):
    environment = run["environment"]
    return run["status"] == "success" and not environment["external_build_observed"] and not environment["sampling_errors"]


def print_summary(summary):
    print("| Group / scenario | Mode | Metric | Median | Paired change (95% CI) |")
    print("| --- | --- | --- | ---: | ---: |")
    for row in summary["rows"]:
        label = row["group"] + " / " + row["scenario"]
        if not row["decision_eligible"]:
            label += " (diagnostic only)"
        for name, mode in row["modes"].items():
            for metric, measured in mode["metrics"].items():
                change = format_change(mode["changes"].get(metric))
                median = "n/a" if measured["median"] is None else f"{measured['median']:,.3f}"
                print(f"| {label} | {name} | {metric} | {median} | {change} |")
    print("\nEach percentile is a run-level observation. Ranges describe variation; overlap is not a noise test.")


def format_change(change):
    if not change or change["median_percent"] is None:
        return "n/a"
    lower, upper = change["ci95_percent"]
    return f"{change['median_percent']:+.2f}% [{lower:+.2f}%, {upper:+.2f}%]"

#!/usr/bin/env python3
"""Audit saved CPU studies without running measurements or changing their artifacts.

Usage: cpu_profile_audit.py PLAN.json --output NEW_DIRECTORY

The plan lists phases with name, directory, protocol_sha256 and helpers_directory.
Paths are relative to the plan, or absolute. A phase may require passing results
from an earlier phase or a hashed analysis/audit pair: case and comparison select
the result. An inconclusive requirement skips that phase. Protocol dependency
paths resolve from their protocol directory, including the parent audit helper.
"""

import argparse
import json
from pathlib import Path
import sys
import types

from benchmark_common import file_hash, paired_change
from cpu_calibration import SCOPES, summarize_calibration


def read(path):
    return json.loads(path.read_text())


def save(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def resolve(base, path):
    return (base / path).resolve()


def verified(base, artifact):
    path = resolve(base, artifact["path"])
    require(file_hash(path) == artifact["sha256"], f"artifact hash mismatch: {path}")
    return path


def parent_audit(directory, protocol):
    artifact = {"path": protocol["parent_audit"], "sha256": protocol["parent_audit_sha256"]}
    path = verified(directory, artifact)
    module = types.ModuleType("cpu_parent_audit")
    module.__file__ = str(path)
    code = compile(path.read_bytes(), str(path), "exec", optimize=0)
    exec(code, module.__dict__)
    return module


def verify_dependencies(directory, helpers, protocol):
    for name in ("parent_controller", "source_proof", "preparation"):
        if name in protocol:
            verified(directory, {"path": protocol[name], "sha256": protocol[name + "_sha256"]})
    if "host_helper" in protocol:
        verified(directory, protocol["host_helper"])
    for group in ("execution_helpers", "confirmation_helpers"):
        for name, expected in protocol.get(group, {}).items():
            verified(helpers, {"path": name, "sha256": expected})
    for artifact in protocol.get("inherited_prerequisites", {}).values():
        verified(directory, artifact)


def verify_artifacts(row, directory, modes):
    for artifact in row["artifacts"].values():
        verified(directory, artifact)
    run = resolve(directory, row["directory"])
    accepted = read(run.parent / "accepted.json")
    require(accepted == {"observations": modes, "whole_block_clean": True}, f"unaccepted round: {run.parent}")
    for path in (run / "host.jsonl", run.parent / "readiness.jsonl"):
        samples = [json.loads(line) for line in path.read_text().splitlines()]
        require(bool(samples), f"empty host observations: {path}")
        require(not any(sample["foreign_native"] for sample in samples), f"host interference: {path}")
    return run


def verify_scopes(row, report):
    if row["mode"] == "BASE":
        require(row["scope_deltas"] is None, "BASE contains scope counters")
        return
    sampled = report["monitoring"]["resources"]["sampled_cpu"]
    require(sampled["sample_every"] == row["sample_every"], "sampling interval mismatch")
    require(int(sampled["seed"]) == row["sampling_seed"], "sampling seed mismatch")
    require(sampled["mode"] == row["sample_mode"], "sampling mode mismatch")
    require(set(row["scope_deltas"]) == set(SCOPES), "scope coverage mismatch")
    for scope, delta in row["scope_deltas"].items():
        actual = {name: sampled["after"][scope][name] - sampled["before"][scope][name] for name in delta}
        require(delta == actual, f"scope delta mismatch: {scope}")
        require(delta["calls_or_polls_started"] > delta["samples_completed"] > 0, f"empty scope: {scope}")
        require(delta["pending_samples"] + delta["ready_samples"] == delta["samples_completed"], "sample accounting")
        cpu_mode = row["sample_mode"] in ("cpu", "empty")
        valid_cpu = delta["cpu_ns_sampled"] > 0 if cpu_mode else delta["cpu_ns_sampled"] == 0
        require(valid_cpu, f"clock mode mismatch: {scope}")


def verify_derived(row, report):
    completed = report["completed_in_window"]
    resources = report["monitoring"]["resources"]
    require(completed == row["measurement_completed"], "completed-write denominator mismatch")
    require(row["metrics"]["writes/s"] == report["operations_per_second"], "throughput mismatch")
    for key, value in resources.items():
        require(row["resources"][key] == value, f"resource mismatch: {key}")
    cpu = resources["cpu_ns_per_completed_operation"]
    require(row["resources"]["cpu_ns_per_completed_entry"] == cpu, "CPU per write mismatch")


def verify_rows(directory, protocol, audit):
    rows = read(directory / "completed-runs.json")
    expected = {audit.identity(row): row for row in protocol["commands"]}
    actual = {audit.identity(row) for row in rows}
    require(len(rows) == len(expected) == protocol["maximum_observations"], "observation count mismatch")
    require(actual == set(expected), "observation identities mismatch")
    for row in rows:
        for key, value in expected[audit.identity(row)].items():
            require(row[key] == value, f"command input mismatch: {audit.identity(row)} {key}")
        run = verify_artifacts(row, directory, len(protocol["modes"]))
        resolved = dict(row, directory=str(run))
        audit.verify_accounting(resolved)
        report = json.loads((run / "stdout").read_text().splitlines()[-1])
        verify_derived(row, report)
        verify_scopes(row, report)
    return rows


def verdict(change, metric, assessment):
    lower, upper = change["ci95_percent"]
    if metric == "CPU_ns_per_completed_write":
        budget = assessment["CPU_budget_percent"]
        return "pass" if upper <= budget else "clearly_over_budget" if lower > budget else "inconclusive"
    budget = assessment["throughput_regression_guard_percent"]
    return "pass" if lower >= -budget else "clearly_regressed" if upper < -budget else "inconclusive"


def comparison(rounds, target, base, protocol):
    metrics = {"CPU_ns_per_completed_write": lambda row: row["resources"]["cpu_ns_per_completed_entry"],
               "writes_per_second": lambda row: row["metrics"]["writes/s"]}
    result = {}
    settings = protocol["statistics"]
    for name, value in metrics.items():
        before = [value(group[base]) for group in rounds]
        after = [value(group[target]) for group in rounds]
        change = paired_change(before, after, seed=settings["seed"], samples=settings["bootstrap_samples"])
        change["resampling_unit"] = "complete matched round"
        if base == "BASE":
            change["budget_verdict"] = verdict(change, name, protocol["assessment"])
        result[name] = change
    return result


def summarize(rows, protocol):
    result = {}
    for case in protocol["cases"]:
        rounds = []
        for number in range(1, case["rounds"] + 1):
            group = {row["mode"]: row for row in rows if row["case"] == case["name"] and row["round"] == number}
            require(set(group) == set(protocol["modes"]), f"incomplete round: {case['name']} {number}")
            rounds.append(group)
        result[case["name"]] = {}
        for label in protocol["comparisons"]:
            target, base = label.split("/")
            result[case["name"]][label] = comparison(rounds, target, base, protocol)
    return result


def audit_inputs(definition, base):
    directory = resolve(base, definition["directory"])
    helpers = resolve(base, definition["helpers_directory"])
    protocol_path = verified(directory, {"path": "protocol.json", "sha256": definition["protocol_sha256"]})
    protocol = read(protocol_path)
    require(protocol["phase"] in {"qualification", "calibration"}, f"unknown audit phase: {protocol['phase']}")
    receipt = read(directory / "recording-complete.json")
    require(receipt["runs"] == protocol["maximum_observations"], "recording count mismatch")
    require(receipt["protocol_sha256"] == file_hash(protocol_path), "recording protocol mismatch")
    audit = parent_audit(directory, protocol)
    verify_dependencies(directory, helpers, protocol)
    for binding in protocol["bindings"].values():
        for name in ("binary", "manifest", "dwarf"):
            verified(directory, {"path": binding[name], "sha256": binding[name + "_sha256"]})
    rows = verify_rows(directory, protocol, audit)
    return protocol, rows


def audit_phase(definition, base, destination):
    protocol, rows = audit_inputs(definition, base)
    results = summarize_calibration(rows, protocol) if protocol["phase"] == "calibration" else summarize(rows, protocol)
    analysis = {"status": "complete", "observations": len(rows), "results": results,
                "statistics": protocol["statistics"], "sampled_CPU_shares_calibrated": protocol["phase"] == "calibration",
                "interpretation": "Individual95percent intervals resample matched complete rounds; CPU includes client and Raft work at observed boundaries."}
    save(destination / "analysis.json", analysis)
    proof = {"status": "passed", "accepted_observations": len(rows), "raw_hashes_match": True,
             "coverage_complete": True, "all_accepted_host_samples_clear": True,
             "all_mode_and_counter_semantics_verified": True, "analysis_sha256": file_hash(destination / "analysis.json")}
    save(destination / "audit.json", proof)
    return analysis


def prerequisite(requirement, base, results):
    if "phase" in requirement:
        analysis = results[requirement["phase"]]
    else:
        path = verified(base, requirement["analysis"])
        audit = read(verified(base, requirement["audit"]))
        require(audit["status"] == "passed", "prerequisite audit failed")
        require(audit["analysis_sha256"] == file_hash(path), "prerequisite analysis mismatch")
        analysis = read(path)
    metrics = analysis["results"][requirement["case"]][requirement["comparison"]]
    names = {"CPU_ns_per_completed_write", "writes_per_second"}
    require(set(metrics) == names, "prerequisite metrics mismatch")
    return all(metrics[name]["budget_verdict"] == "pass" for name in names)


def execute(plan, base, output, state):
    results = {}
    for definition in plan["phases"]:
        name = definition["name"]
        state["phase"] = name
        state["phases"][name] = {"status": "running"}
        save(output / "state.json", state)
        ready = all(prerequisite(item, base, results) for item in definition.get("requires", []))
        if not ready:
            state["phases"][name] = {"status": "skipped", "reason": "qualification guard did not pass"}
            continue
        destination = output / name
        destination.mkdir()
        results[name] = audit_phase(definition, base, destination)
        state["phases"][name] = {"status": "complete", "audit": str(destination / "audit.json")}
    return results


def load_plan(plan_path, output):
    plan = read(plan_path)
    names = set()
    for definition in plan["phases"]:
        name = definition["name"]
        require(name.isidentifier(), f"invalid phase name: {name}")
        require(name not in names, f"duplicate phase name: {name}")
        names.add(name)
        source = resolve(plan_path.parent, definition["directory"])
        require(not output.is_relative_to(source), "output must be outside saved measurement directories")
    return plan


def replay(plan_path, output):
    plan_path = plan_path.resolve()
    output = output.resolve()
    plan = load_plan(plan_path, output)
    output.mkdir(parents=True, exist_ok=False)
    state = {"status": "running", "plan_sha256": file_hash(plan_path), "measurements_launched": 0, "phases": {}}
    try:
        results = execute(plan, plan_path.parent, output, state)
    except BaseException as error:
        state.update(status="failed", error=f"{type(error).__name__}: {error}")
        if "phase" in state:
            state["phases"][state["phase"]].update(status="failed")
        raise
    else:
        state.update(status="complete", observations=sum(item["observations"] for item in results.values()))
    finally:
        save(output / "state.json", state)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("plan", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    replay(args.plan, args.output)


if __name__ == "__main__":
    sys.dont_write_bytecode = True
    main()

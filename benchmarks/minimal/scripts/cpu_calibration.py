"""Calibrate saved sampled CPU observations with matched empty-clock controls."""

import random
import statistics

SCOPES = ["raft_core", "apply_worker", "client_session", "client_request", "client_completion"]
TOP_LEVEL = ["raft_core", "apply_worker", "client_session"]


def pair_values(cpu, empty):
    interval = cpu["sample_every"]
    if interval != empty["sample_every"] or interval <= 0:
        raise ValueError("CPU and EMPTY must use the same positive sampling interval")
    work = {}
    for scope in SCOPES:
        measured = cpu["scope_deltas"][scope]
        control = empty["scope_deltas"][scope]
        clock_ns = control["cpu_ns_sampled"] / control["samples_completed"]
        correction = clock_ns * measured["samples_completed"]
        corrected_ns = measured["cpu_ns_sampled"] - correction
        estimated_ns = corrected_ns * interval
        work[scope] = estimated_ns / cpu["measurement_completed"]
    process_ns = cpu["resources"]["cpu_ns_per_completed_entry"]
    return {"work_ns_per_write": work, "process_ns_per_write": process_ns}


def means(pairs):
    result = {}
    process_ns = statistics.fmean(pair["process_ns_per_write"] for pair in pairs)
    for scope in SCOPES:
        work = statistics.fmean(pair["work_ns_per_write"][scope] for pair in pairs)
        result[scope + ":ns_per_write"] = work
        result[scope + ":percent_process_CPU"] = 100 * work / process_ns
    total = sum(result[scope + ":ns_per_write"] for scope in TOP_LEVEL)
    result["top_level_sum:percent_process_CPU"] = 100 * total / process_ns
    return result


def bootstrap(pairs, settings):
    count = settings["bootstrap_samples"]
    if not pairs or count <= 0:
        raise ValueError("calibration requires matched pairs and positive bootstrap_samples")
    randomizer = random.Random(settings["seed"])
    points = means(pairs)
    draws = {name: [] for name in points}
    for _ in range(count):
        sample = randomizer.choices(pairs, k=len(pairs))
        values = means(sample)
        for name, value in values.items():
            draws[name].append(value)
    result = {}
    for name, values in draws.items():
        values.sort()
        interval = [values[int(count * 0.025)], values[min(count - 1, int(count * 0.975))]]
        result[name] = {"estimate": points[name], "ci95": interval}
    return result


def summarize_calibration(rows, protocol):
    result = {}
    for case in protocol["cases"]:
        pairs = []
        for number in range(1, case["rounds"] + 1):
            group = {row["mode"]: row for row in rows if row["case"] == case["name"] and row["round"] == number}
            if set(group) != {"CPU", "EMPTY"}:
                raise ValueError(f"incomplete calibration pair: {case['name']} round {number}")
            pair = pair_values(group["CPU"], group["EMPTY"])
            pairs.append(pair)
        metrics = bootstrap(pairs, protocol["statistics"])
        signals = {scope: "resolved_positive" if metrics[scope + ":ns_per_write"]["ci95"][0] > 0
                   else "inconclusive" for scope in SCOPES}
        result[case["name"]] = {"rounds": len(pairs), "metrics": metrics, "signal_status": signals}
    return result

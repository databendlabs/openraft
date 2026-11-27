import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from benchmark_common import file_hash
import cpu_calibration as calibration
import cpu_profile_audit as audit


PARENT = '''def identity(row):
    return row["case"], row["round"], row["mode"], row["recording"]

def verify_accounting(row):
    assert row["measurement_completed"] == 7
'''


def counters(cpu_ns):
    return {"calls_or_polls_started": 20, "samples_completed": 2, "pending_samples": 2,
            "ready_samples": 0, "cpu_ns_sampled": cpu_ns}


def sampled(mode):
    after = {name: counters(8 if mode == "EMPTY" else 6) for name in calibration.SCOPES}
    before = {name: dict.fromkeys(values, 0) for name, values in after.items()}
    return {"sample_every": 4, "seed": "7", "mode": mode.lower(), "after": after, "before": before}


class WorkflowTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cpu-audit-test-"))
        self.input = self.root / "input"
        self.input.mkdir()
        self.helper = self.input / "parent.py"
        self.helper.write_text(PARENT)

    def tearDown(self):
        result = self._outcome.result
        failures = [test for test, _ in result.failures + result.errors]
        if self in failures:
            print(f"CPU audit fixture retained: {self.root}", file=sys.stderr)
            return
        shutil.rmtree(self.root)

    def record(self, directory, mode):
        directory.mkdir(parents=True)
        row = {"case": "case", "round": 1, "mode": mode, "recording": "plain",
               "sample_every": None if mode == "BASE" else 4, "sampling_seed": 7,
               "sample_mode": None if mode == "BASE" else mode.lower()}
        resources = {"cpu_ns_per_completed_operation": 100}
        if mode != "BASE":
            resources["sampled_cpu"] = sampled(mode)
        report = {"monitoring": {"resources": resources}, "operations_per_second": 10, "completed_in_window": 7}
        (directory / "stdout").write_text(json.dumps(report) + "\n")
        (directory / "host.jsonl").write_text('{"foreign_native": false}\n')
        row.update(directory=str(directory), measurement_completed=7, metrics={"writes/s": 10})
        row["resources"] = dict(resources, cpu_ns_per_completed_entry=100)
        row["scope_deltas"] = None if mode == "BASE" else resources["sampled_cpu"]["after"]
        row["artifacts"] = {name: {"path": str(directory / name), "sha256": file_hash(directory / name)}
                            for name in ("stdout", "host.jsonl")}
        return row

    def phase(self, name, kind="qualification", modes=("BASE", "CPU")):
        directory = self.input / name
        directory.mkdir()
        runs = directory / "runs"
        rows = [self.record(runs / mode, mode) for mode in modes]
        commands = [{key: row[key] for key in ("case", "round", "mode", "recording")} for row in rows]
        protocol = {"phase": kind, "parent_audit": "../parent.py", "parent_audit_sha256": file_hash(self.helper),
                    "maximum_observations": 2, "commands": commands, "modes": list(modes),
                    "bindings": {"BASE": {name + suffix: file_hash(self.helper) if suffix else "../parent.py"
                                          for name in ("binary", "manifest", "dwarf") for suffix in ("", "_sha256")}},
                    "cases": [{"name": "case", "rounds": 1}], "comparisons": ["CPU/BASE"] if kind == "qualification" else [],
                    "statistics": {"seed": 7, "bootstrap_samples": 100},
                    "assessment": {"CPU_budget_percent": 5, "throughput_regression_guard_percent": 5}}
        audit.save(directory / "protocol.json", protocol)
        audit.save(directory / "completed-runs.json", rows)
        audit.save(directory / "recording-complete.json", {"runs": 2, "protocol_sha256": file_hash(directory / "protocol.json")})
        audit.save(runs / "accepted.json", {"observations": 2, "whole_block_clean": True})
        (runs / "readiness.jsonl").write_text('{"foreign_native": false}\n')
        return {"name": name, "directory": "input/" + name, "helpers_directory": "input",
                "protocol_sha256": file_hash(directory / "protocol.json")}

    def plan(self, phases):
        path = self.root / "plan.json"
        audit.save(path, {"phases": phases})
        return path

    def test_replay_both_phases_from_another_directory_preserves_inputs(self):
        first = self.phase("confirmation")
        second = self.phase("clock_control", "calibration", ("CPU", "EMPTY"))
        second["requires"] = [{"phase": "confirmation", "case": "case", "comparison": "CPU/BASE"}]
        plan = self.plan([first, second])
        before = {str(path): file_hash(path) for path in self.input.rglob("*") if path.is_file()}
        output = self.root / "replay"
        script = Path(audit.__file__).resolve()
        result = subprocess.run([sys.executable, "-B", str(script), str(plan), "--output", str(output)],
                                cwd=self.root.parent, capture_output=True, text=True)
        self.assertEqual(0, result.returncode, result.stderr)
        state = audit.read(output / "state.json")
        self.assertEqual(4, state["observations"])
        self.assertEqual(0, state["measurements_launched"])
        self.assertEqual("complete", state["status"])
        statuses = {name: phase["status"] for name, phase in state["phases"].items()}
        self.assertEqual({"confirmation": "complete", "clock_control": "complete"}, statuses)
        after = {str(path): file_hash(path) for path in self.input.rglob("*") if path.is_file()}
        self.assertEqual(before, after)

    def test_raw_corruption_records_terminal_failure(self):
        phase = self.phase("confirmation")
        plan = self.plan([phase])
        raw = self.input / "confirmation/runs/CPU/stdout"
        raw.write_text("corrupted")
        output = self.root / "failed"
        with self.assertRaisesRegex(ValueError, "artifact hash mismatch"):
            audit.replay(plan, output)
        state = audit.read(output / "state.json")
        self.assertEqual("failed", state["status"])
        self.assertEqual({"confirmation": {"status": "failed"}}, state["phases"])
        self.assertEqual(0, state["measurements_launched"])

    def test_inconclusive_guard_skips_calibration_before_reading_its_files(self):
        prior = self.root / "prior.json"
        metrics = {name: {"budget_verdict": "inconclusive"} for name in ("CPU_ns_per_completed_write", "writes_per_second")}
        audit.save(prior, {"results": {"case": {"CPU/BASE": metrics}}})
        proof = self.root / "prior-audit.json"
        audit.save(proof, {"status": "passed", "analysis_sha256": file_hash(prior)})
        phase = {"name": "calibration", "directory": "absent", "requires": [{"case": "case", "comparison": "CPU/BASE",
                 "analysis": {"path": "prior.json", "sha256": file_hash(prior)},
                 "audit": {"path": "prior-audit.json", "sha256": file_hash(proof)}}]}
        output = self.root / "skipped"
        audit.replay(self.plan([phase]), output)
        state = audit.read(output / "state.json")
        self.assertEqual("complete", state["status"])
        self.assertEqual(0, state["observations"])
        expected = {"calibration": {"status": "skipped", "reason": "qualification guard did not pass"}}
        self.assertEqual(expected, state["phases"])

    def test_optimized_python_keeps_the_pinned_validator_checks(self):
        code = """import sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from cpu_profile_audit import parent_audit
protocol = {"parent_audit": "parent.py", "parent_audit_sha256": sys.argv[3]}
helper = parent_audit(Path(sys.argv[2]), protocol)
try:
    helper.verify_accounting({"measurement_completed": 8})
except AssertionError:
    sys.exit(0)
raise RuntimeError("validator assertions were disabled")
"""
        command = [sys.executable, "-O", "-B", "-c", code, str(Path(audit.__file__).parent),
                   str(self.input), file_hash(self.helper)]
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(0, result.returncode, result.stderr)

    def test_parent_helper_hash_is_checked_before_execution(self):
        protocol = {"parent_audit": "parent.py", "parent_audit_sha256": "incorrect"}
        self.helper.write_text("raise RuntimeError('must not execute')")
        with self.assertRaisesRegex(ValueError, "artifact hash mismatch"):
            audit.parent_audit(self.input, protocol)

    def test_unknown_protocol_phase_records_terminal_failure(self):
        phase = self.phase("misspelled", kind="calibrtion")
        plan = self.plan([phase])
        output = self.root / "failed"
        with self.assertRaisesRegex(ValueError, "unknown audit phase: calibrtion"):
            audit.replay(plan, output)
        state = audit.read(output / "state.json")
        self.assertEqual("failed", state["status"])
        self.assertEqual({"misspelled": {"status": "failed"}}, state["phases"])
        self.assertEqual(0, state["measurements_launched"])


class CalibrationTest(unittest.TestCase):
    def test_empty_clock_correction_preserves_negative_estimates_and_nested_scopes(self):
        cpu = {"sample_every": 4, "measurement_completed": 2,
               "resources": {"cpu_ns_per_completed_entry": 20},
               "scope_deltas": {scope: {"cpu_ns_sampled": 6, "samples_completed": 2} for scope in calibration.SCOPES}}
        empty = {"sample_every": 4,
                 "scope_deltas": {scope: {"cpu_ns_sampled": 8, "samples_completed": 2} for scope in calibration.SCOPES}}
        pair = calibration.pair_values(cpu, empty)
        expected = {"work_ns_per_write": dict.fromkeys(calibration.SCOPES, -4), "process_ns_per_write": 20}
        self.assertEqual(expected, pair)
        estimates = calibration.bootstrap([pair], {"seed": 7, "bootstrap_samples": 100})
        self.assertEqual({"estimate": -4, "ci95": [-4, -4]}, estimates["client_completion:ns_per_write"])
        self.assertEqual({"estimate": -60, "ci95": [-60, -60]}, estimates["top_level_sum:percent_process_CPU"])

    def test_different_sampling_intervals_are_not_calibrated(self):
        with self.assertRaisesRegex(ValueError, "same positive sampling interval"):
            calibration.pair_values({"sample_every": 4}, {"sample_every": 8})


if __name__ == "__main__":
    unittest.main()

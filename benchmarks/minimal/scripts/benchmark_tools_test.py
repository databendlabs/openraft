import argparse
import contextlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

import benchmark_build as build
import benchmark_common as common
import benchmark_runner as runner
import benchmark_scenarios as scenarios
import compare
import process_profile


def arguments(*extra):
    parser = argparse.ArgumentParser()
    scenarios.add_arguments(parser)
    return parser.parse_args(extra)


def outcomes(completed=7, timed_out=2, pending=1):
    return {"offered": 10, "admitted": 10, "completed": completed, "failed": 0,
            "timed_out": timed_out, "pending": pending, "rejected": 0, "missed": 0}


class StatisticsTest(unittest.TestCase):
    def test_each_scenario_rotates_modes_independently_of_row_order(self):
        actual = list(common.schedule(["A", "B"], ["base", "target"], 3))
        expected = [(1, "A", "base"), (1, "A", "target"), (1, "B", "target"), (1, "B", "base"),
                    (2, "B", "base"), (2, "B", "target"), (2, "A", "target"), (2, "A", "base"),
                    (3, "A", "base"), (3, "A", "target"), (3, "B", "target"), (3, "B", "base")]
        self.assertEqual(expected, actual)

    def test_overlapping_ranges_do_not_hide_consistent_paired_change(self):
        actual = common.paired_change([10, 20, 40, 80, 160], [20, 40, 80, 160, 320], seed=7, samples=250)
        expected = {"paired_percent": [100.0] * 5, "median_percent": 100.0, "ci95_percent": [100.0, 100.0],
                    "method": "percentile bootstrap of paired round medians", "resampling_unit": "complete paired round",
                    "rounds": 5, "bootstrap_samples": 250, "seed": 7}
        self.assertEqual(expected, actual)

    def test_absent_or_zero_baselines_have_no_fabricated_percent_change(self):
        actual = common.paired_change([0, 10], [2, 20])
        self.assertEqual({"paired_percent": [None, 100.0], "median_percent": None, "ci95_percent": None}, actual)


class WorkloadTest(unittest.TestCase):
    def test_read_only_renews_leases_and_keeps_write_overrides_inactive(self):
        args = arguments("--seconds", "2", "--clients", "32", "--operations", "100", "--write-rate", "10000")
        row = scenarios.Scenario("read-only", 0, 0, 2, 20)
        flags = scenarios.workload_flags(args, row, scenarios.Mode("local", {"enable_local_lease_read": True}))
        config = json.loads(runner.option(flags, ("--raft-config",)))
        self.assertEqual("0", runner.option(flags, ("--clients",)))
        self.assertEqual("0", runner.option(flags, ("--operations",)))
        self.assertIsNone(runner.option(flags, ("--write-rate",)))
        self.assertEqual({"enable_tick": True, "enable_heartbeat": True, "enable_elect": True,
                          "enable_local_lease_read": True}, config)

    def test_profiles_select_the_required_nondefault_feature(self):
        args = arguments("--profile", "--payload-bytes", "4096", "--serialized-transport")
        self.assertEqual(("realistic-profiles",), scenarios.selected_features(args))
        flags = scenarios.profile_flags(args)
        self.assertEqual(["--profile", "--payload-bytes", "4096", "--serialized-transport"], flags)

    def test_seed_is_resolved_from_the_actual_custom_command(self):
        actual = runner.workload_seed({}, ["--arrival-seed", "9"])
        self.assertEqual(9, actual)
        with self.assertRaisesRegex(ValueError, "arrival seed"):
            runner.workload_seed({"arrival": {"arrival_seed": 2}}, ["--arrival-seed", "9"])

    def test_historical_truncated_count_is_rejected(self):
        report = {"operations": 29952, "operations_per_second": 100, "runtime_stats": False}
        with self.assertRaisesRegex(ValueError, "requested 30000"):
            runner.validate_report(report, ["--latency", "-n", "30000"], False)

    def test_conserved_overload_is_valid_for_writes_and_reads(self):
        report = {"write": {"operations": 7}, "write_timing": outcomes(), "runtime_stats": False,
                  "read": {"successful": {"operations": 7}, "failed": {"operations": 0}, "timing": outcomes()}}
        runner.validate_report(report, ["-n", "10", "--read-operations", "10", "--open-loop"], False)
        report["read"]["timing"]["pending"] = 0
        with self.assertRaisesRegex(ValueError, "all outcomes"):
            runner.validate_report(report, ["-n", "10", "--read-operations", "10"], False)

    def test_zero_completions_keep_rates_and_unavailable_histograms(self):
        report = {"write": {"operations_per_second": 0, "latency": None},
                  "read": {"successful": {"operations_per_second": 0, "latency": None}},
                  "apply_metrics": {"0": {"apply_calls": 0, "entries_per_apply": None}}}
        expected = {"writes/s": 0, "write p50 ns": None, "write p99 ns": None,
                    "reads/s": 0, "read p50 ns": None, "read p99 ns": None,
                    "node 0 apply calls": 0, "node 0 entries/apply p50": None, "node 0 entries/apply p90": None}
        self.assertEqual(expected, runner.measurements(report))

    def test_custom_comparisons_reject_outer_workload_overrides(self):
        command = ["compare.py", "--base", "HEAD", "--seed", "7", "--", "--latency", "-n", "10"]
        output = io.StringIO()
        with mock.patch.object(sys, "argv", command), contextlib.redirect_stderr(output):
            with self.assertRaises(SystemExit) as error:
                compare.parse_args()
        self.assertEqual(2, error.exception.code)
        self.assertIn("pass custom workload controls after --", output.getvalue())

    def test_diagnostic_companions_pin_the_performance_lock_and_toolchain(self):
        suite = object.__new__(runner.Suite)
        inputs = {"crate": "benchmarks/minimal", "toolchain": {"selected": "pinned"}}
        suite.builds = {(False, "target"): {"manifest": "/cache/entry/manifest.json", "metadata": {"inputs": inputs}}}
        request = build.BuildRequest("target", features=("runtime-stats",))
        actual = suite.diagnostic_request(request, "target")
        expected = build.BuildRequest("target", features=("runtime-stats",), toolchain="pinned",
                                      lockfile=Path("/cache/entry/source/benchmarks/minimal/Cargo.lock"))
        self.assertEqual(expected, actual)

    def test_diagnostic_companions_reject_a_different_dependency_graph(self):
        performance = {"metadata": {"inputs": {"features": [], "lockfile_sha256": "one"}}}
        diagnostic = {"metadata": {"inputs": {"features": ["runtime-stats"], "lockfile_sha256": "two"}}}
        with self.assertRaisesRegex(ValueError, "lock differs"):
            runner.check_matching_builds(performance, diagnostic)

    def test_read_success_and_failure_counters_must_match_cohort_outcomes(self):
        report = {"write": {"operations": 0}, "runtime_stats": False,
                  "read": {"successful": {"operations": 11}, "failed": {"operations": 0},
                           "timing": outcomes(completed=10, timed_out=0, pending=0)}}
        with self.assertRaisesRegex(ValueError, "read counters"):
            runner.validate_report(report, ["-n", "0", "--read-operations", "10"], False)
        report["read"]["successful"]["operations"] = 10
        report["read"]["failed"]["operations"] = 1
        with self.assertRaisesRegex(ValueError, "read counters"):
            runner.validate_report(report, ["-n", "0", "--read-operations", "10"], False)

    def test_optional_apply_observers_are_symmetric_diagnostics(self):
        suite = object.__new__(runner.Suite)
        suite.args, suite.plan = types.SimpleNamespace(seed=1), types.SimpleNamespace(custom_args=None)
        suite.supported = {(False, "base"): {"--latency"}, (False, "target"): {"--latency", "--measure-apply"},
                           (True, "base"): {"--latency"}, (True, "target"): {"--latency", "--measure-apply"}}
        mode = scenarios.Mode("target")
        with mock.patch.object(runner, "workload_flags", return_value=["--latency"]):
            self.assertEqual(["--latency"], suite.flags(None, mode, False))
            self.assertEqual(["--latency"], suite.flags(None, mode, True))
            suite.supported[(True, "base")].add("--measure-apply")
            self.assertEqual(["--latency", "--measure-apply"], suite.flags(None, mode, True))

    def test_overload_keeps_capacity_statistics_and_conditional_latency(self):
        coverage = runner.completion_coverage({"operations": 7, "timing": outcomes(), "samples": 7}, ["-n", "10"])
        runs = [{"round": index, "status": "success", "environment": {"external_build_observed": False, "sampling_errors": []},
                 "runtime_identity": "unchanged", "coverage": coverage,
                 "measurements": {"writes/s": 7, "write p99 ns": 100}} for index in range(1, 6)]
        plan = runner.Plan([], [scenarios.Mode("base"), scenarios.Mode("target")], {})
        actual = runner.summarize_modes(types.SimpleNamespace(runs=5, seed=1), plan, {"base": runs, "target": runs})
        self.assertTrue(actual["capacity_comparison_eligible"])
        self.assertFalse(actual["latency_comparison_eligible"])
        expected = {"offered": 50, "completed": 35, "failed": 0, "timed_out": 10, "rejected": 0,
                    "pending": 5, "missed": 0, "latency_samples": 35, "complete": False}
        self.assertEqual(expected, actual["modes"]["target"]["coverage"]["write"])
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            runner.print_summary({"rows": [{"group": "performance", "scenario": "overload", **actual}]})
        self.assertIn("(conditional)", output.getvalue())


class ArtifactTest(unittest.TestCase):
    def test_nonzero_exit_retains_complete_streams_and_command(self):
        directory = Path(tempfile.mkdtemp(prefix="bench-failure-"))
        output = directory / "run"
        command = common.Command([sys.executable, "-c", "import sys; print('full stdout'); print('full stderr', file=sys.stderr); sys.exit(3)"])
        result = command.run(output)
        self.assertEqual("failed", result["status"])
        self.assertEqual(3, result["returncode"])
        self.assertEqual("full stdout\n", (output / "stdout").read_text())
        self.assertEqual("full stderr\n", (output / "stderr").read_text())
        self.assertEqual(result, json.loads((output / "command.json").read_text()))
        shutil.rmtree(directory)

    def test_timeout_retains_output_before_killing_the_owned_process(self):
        directory = Path(tempfile.mkdtemp(prefix="bench-timeout-"))
        command = common.Command([sys.executable, "-u", "-c", "import time; print('before timeout'); time.sleep(30)"], timeout=0.1)
        result = command.run(directory / "run")
        self.assertEqual("timeout", result["status"])
        self.assertEqual("before timeout\n", (directory / "run/stdout").read_text())
        self.assertLess(result["returncode"], 0)
        shutil.rmtree(directory)


class CacheTest(unittest.TestCase):
    def test_cache_identity_changes_with_compiler_flags_and_actual_ignored_lock(self):
        directory = Path(tempfile.mkdtemp(prefix="bench-cache-inputs-"))
        source = directory / "source"
        crate = source / "benchmarks/minimal"
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_text('[features]\nrealistic-profiles=[]\n')
        supplied = directory / "captured.lock"
        supplied.write_text("lock-one")
        cache = types.SimpleNamespace(root=source, cache=directory / "cache", args=types.SimpleNamespace(build_timeout=1))
        job = build.BuildJob(cache, build.BuildRequest("test", revision="reference", lockfile=supplied), directory, source, {})
        with mock.patch.object(job, "toolchain", return_value={"rustc": "compiler-one"}), mock.patch.object(job, "compile", side_effect=lambda entry, inputs, lock: entry.name):
            first = job.run()
            supplied.write_text("lock-two")
            second = job.run()
            with mock.patch.dict(job.env, {"RUSTFLAGS": "-C opt-level=1"}):
                third = job.run()
        self.assertNotEqual(first, second)
        self.assertNotEqual(second, third)
        shutil.rmtree(directory)

    def test_toolchain_resolution_uses_the_crate_directory(self):
        job = object.__new__(build.BuildJob)
        job.cache = types.SimpleNamespace(root=Path("/checkout"))
        job.request = build.BuildRequest("current")
        self.assertEqual(Path("/checkout/benchmarks/minimal"), job.root_context())
        job.request.revision = "older"
        job.crate = Path("/snapshot/benchmarks/minimal")
        self.assertEqual(job.crate, job.root_context())

    def test_cache_hits_recheck_checkout_and_configuration_inputs(self):
        directory = Path(tempfile.mkdtemp(prefix="bench-cache-hit-"))
        cache = types.SimpleNamespace(root=directory, cache=directory, args=types.SimpleNamespace(build_timeout=1))
        job = object.__new__(build.BuildJob)
        job.cache, job.source = cache, directory
        with mock.patch.object(job, "inputs", return_value={}), mock.patch.object(job, "resolve_lock", return_value={"sha256": "lock"}):
            with mock.patch.object(Path, "exists", return_value=True), mock.patch.object(job, "verify_inputs", side_effect=RuntimeError("checkout changed")):
                with self.assertRaisesRegex(RuntimeError, "checkout changed"):
                    job.run()
        shutil.rmtree(directory)

    def test_compiler_alias_is_queried_and_reused_wrapper_paths_are_hashed(self):
        directory = Path(tempfile.mkdtemp(prefix="bench-compiler-"))
        wrapper = directory / "wrapper"
        wrapper.write_text("#!/bin/sh\nexec \"$@\"\n")
        wrapper.chmod(0o755)
        job = object.__new__(build.BuildJob)
        job.crate, job.source, job.request = directory, directory, build.BuildRequest("test", toolchain="selected")
        job.env = {"PATH": os.environ["PATH"], "CARGO_BUILD_RUSTC": sys.executable,
                   "CARGO_BUILD_RUSTC_WRAPPER": str(wrapper)}
        with mock.patch.object(build, "build_tools_config", return_value={}), mock.patch.object(job, "command", return_value="identity") as command:
            first = job.toolchain()
            wrapper.write_text("#!/bin/sh\n# changed wrapper\nexec \"$@\"\n")
            second = job.toolchain()
        self.assertEqual([str(Path(sys.executable).absolute()), "-vV"], command.call_args_list[0].args[0])
        self.assertNotEqual(first["executables"]["rustc-wrapper"]["sha256"], second["executables"]["rustc-wrapper"]["sha256"])
        shutil.rmtree(directory)

    def test_overlay_external_symlinks_are_rejected_and_internal_links_are_allowed(self):
        directory = Path(tempfile.mkdtemp(prefix="bench-links-"))
        source = directory / "source"
        source.mkdir()
        (directory / "outside.rs").write_text("external source")
        link = source / "module.rs"
        link.symlink_to("../outside.rs")
        with self.assertRaisesRegex(ValueError, "external symlinks"):
            build.reject_external_links(source)
        link.unlink()
        (source / "inside.rs").write_text("inside source")
        link.symlink_to("inside.rs")
        build.reject_external_links(source)
        self.assertEqual("inside source", link.read_text())
        shutil.rmtree(directory)

    def test_compiler_build_aliases_participate_in_environment_identity(self):
        aliases = {"CARGO_BUILD_RUSTC": "compiler", "CARGO_BUILD_RUSTC_WRAPPER": "wrapper",
                   "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER": "workspace-wrapper", "CARGO_BUILD_INCREMENTAL": "false"}
        with mock.patch.dict(os.environ, aliases, clear=True):
            actual = build.build_environment()
        self.assertEqual(aliases, actual)

    def test_repository_local_wrapper_identity_survives_snapshot_relocation(self):
        directory = Path(tempfile.mkdtemp(prefix="bench-local-wrapper-"))
        identities = []
        for name in ("snapshot-one", "snapshot-two"):
            source = directory / name
            source.mkdir()
            wrapper = source / "wrapper"
            wrapper.write_text("#!/bin/sh\nexec \"$@\"\n")
            wrapper.chmod(0o755)
            job = object.__new__(build.BuildJob)
            job.source, job.env = source, {"RUSTC_WRAPPER": str(wrapper)}
            tool = build.tool_identity(str(wrapper), source, job.env)
            identities.append({"tool": job.normalized_tools({"wrapper": tool}), "env": job.environment_inputs()})
        self.assertEqual(identities[0], identities[1])
        shutil.rmtree(directory)

    def test_multicall_compiler_shims_keep_their_invocation_name(self):
        directory = Path(tempfile.mkdtemp(prefix="bench-multicall-"))
        executable = directory / "multicall"
        executable.write_text("#!/usr/bin/env python3\nimport os, sys\nprint(os.path.basename(sys.argv[0]))\n")
        executable.chmod(0o755)
        alias = directory / "compiler-shim"
        alias.symlink_to(executable.name)
        identity = build.tool_identity(str(alias), directory, os.environ)
        result = subprocess.run([identity["path"]], capture_output=True, text=True, check=True, timeout=1)
        self.assertEqual("compiler-shim\n", result.stdout)
        self.assertEqual(str(executable.resolve()), identity["resolved_path"])
        shutil.rmtree(directory)


class ProcessMetadataTest(unittest.TestCase):
    def test_duplicate_numeric_endpoints_are_rejected_before_connecting(self):
        nodes = [{"api_addr": "127.0.0.1:1001", "raft_addr": "127.0.0.1:2001", "control_addr": "127.0.0.1:3001"},
                 {"api_addr": "http://127.0.0.1:1001", "raft_addr": "127.0.0.1:2002", "control_addr": "127.0.0.1:3002"}]
        with self.assertRaisesRegex(ValueError, "unique"):
            process_profile.validate_endpoints(nodes)

    def test_declared_node_ids_are_verified_before_fault_requests(self):
        cluster = types.SimpleNamespace(read_reports=lambda route: {1: {"id": 2}}, http=mock.Mock())
        with self.assertRaisesRegex(ValueError, "reports ID 2"):
            process_profile.topology_metadata(cluster, {})
        cluster.http.assert_not_called()

    def test_attach_runtime_unknown_is_explicit_and_owned_nodes_require_verification(self):
        cluster = types.SimpleNamespace(args=types.SimpleNamespace(attach=Path("manifest")),
                                        api=mock.Mock(side_effect=process_profile.AvailabilityError("missing", status=404)))
        actual = process_profile.runtime_report(cluster, 1)
        self.assertEqual({"server_worker_threads": None, "verification": "unknown", "reason": "runtime endpoint unsupported"}, actual)
        cluster.args.attach = None
        with self.assertRaisesRegex(ValueError, "rebuild"):
            process_profile.runtime_report(cluster, 1)


if __name__ == "__main__":
    unittest.main()

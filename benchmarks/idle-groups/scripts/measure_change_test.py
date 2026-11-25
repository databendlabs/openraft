import argparse
import contextlib
import io
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

import measure_change as mc


def git(root, *args):
    config = ["-c", "user.name=test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false",
              "-c", "core.hooksPath=/dev/null"]
    subprocess.run(["git", "-C", str(root), *config, *args], check=True, capture_output=True)


class ParseCaseTest(unittest.TestCase):
    def test_extra_args_follow_the_benchmark_args(self):
        case = mc.parse_case("3 1000 local default --count-allocs --settle-secs 5", mc.IDLE_GROUPS)

        self.assertEqual(3, case.runs)
        expected = ["--groups", "1000", "--followers", "local", "--mode", "default",
                    "--count-allocs", "--settle-secs", "5"]
        self.assertEqual(expected, case.args())

    def test_a_grpc_groups_case_runs_its_scenario_with_config_flags_last(self):
        case = mc.parse_case("2 crash --groups 100 -- --enable-pre-vote true", mc.GRPC_GROUPS)

        self.assertEqual(2, case.runs)
        expected = ["run", "--scenario", "crash", "--groups", "100", "--", "--enable-pre-vote", "true"]
        self.assertEqual(expected, case.args())
        self.assertEqual(["--groups", "100", "--", "--enable-pre-vote", "true"], case.extra)

    def test_rejects_a_short_case_and_a_non_numeric_runs(self):
        cases = [(mc.IDLE_GROUPS, "3 1000 stub"), (mc.IDLE_GROUPS, "x 1000 stub default"),
                 (mc.GRPC_GROUPS, "3"), (mc.GRPC_GROUPS, "x crash")]
        for bench, text in cases:
            with self.subTest(bench=bench.name, text=text), self.assertRaises(argparse.ArgumentTypeError):
                mc.parse_case(text, bench)


class ParseArgsTest(unittest.TestCase):
    def test_cases_follow_the_form_of_the_chosen_benchmark(self):
        args = mc.parse_args(["label", "f.rs", "--bench", "grpc-groups", "--case", "3 steady"])
        self.assertEqual(mc.GRPC_GROUPS, args.bench)
        self.assertEqual(["run", "--scenario", "steady"], args.cases[0].args())

        args = mc.parse_args(["label", "f.rs", "--case", "3 1000 stub default"])
        self.assertEqual(mc.IDLE_GROUPS, args.bench)
        self.assertEqual(["--groups", "1000", "--followers", "stub", "--mode", "default"], args.cases[0].args())

    def test_a_case_of_the_wrong_form_stops_before_any_build(self):
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit):
            mc.parse_args(["label", "f.rs", "--case", "3 crash"])
        self.assertIn("RUNS GROUPS FOLLOWERS MODE", stderr.getvalue())


class ArtifactExecutableTest(unittest.TestCase):
    MESSAGES = "\n".join([
        '{"reason":"compiler-artifact","target":{"name":"openraft"},"executable":null}',
        '{"reason":"compiler-artifact","target":{"name":"idle-groups"},"executable":"/t/idle-groups"}',
        '{"reason":"compiler-artifact","target":{"name":"grpc-groups"},"executable":"/t/grpc-groups"}',
        '{"reason":"build-finished","success":true}',
    ])

    def test_finds_the_binary_of_each_benchmark(self):
        self.assertEqual("/t/idle-groups", mc.artifact_executable(self.MESSAGES, "idle-groups"))
        self.assertEqual("/t/grpc-groups", mc.artifact_executable(self.MESSAGES, "grpc-groups"))

    def test_fails_without_the_binary(self):
        with self.assertRaises(mc.Failure):
            mc.artifact_executable(self.MESSAGES, "no-such-bench")


class RunCasesTest(unittest.TestCase):
    """Runs a fake benchmark that prints its arguments in one line per phase, as grpc-groups does."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.binary = Path(self.tmp.name) / "fake-bench"

    def tearDown(self):
        self.tmp.cleanup()

    def fake(self, script):
        self.binary.write_text(f"#!/bin/sh\n{script}\n")
        self.binary.chmod(self.binary.stat().st_mode | stat.S_IXUSR)
        return {"before": self.binary, "after": self.binary}

    def test_writes_each_line_of_a_run_with_its_round_build_and_args(self):
        binaries = self.fake("printf 'phase=baseline\\targv=%s\\n' \"$*\"; printf 'phase=failover\\n'")
        case = mc.parse_case("2 crash --groups 6", mc.GRPC_GROUPS)
        results = io.StringIO()

        lines = mc.run_cases([case], binaries, results)

        self.assertEqual(8, len(lines))
        argv = "run --scenario crash --groups 6"
        self.assertEqual(f"round=1\tbuild=before\targs=--groups 6\tphase=baseline\targv={argv}", lines[0])
        self.assertEqual("round=1\tbuild=before\targs=--groups 6\tphase=failover", lines[1])
        self.assertEqual("round=2\tbuild=after\targs=--groups 6\tphase=failover", lines[7])
        self.assertEqual("".join(f"{line}\n" for line in lines), results.getvalue())

    def test_a_run_that_fails_or_prints_nothing_stops_the_comparison(self):
        case = mc.parse_case("1 steady", mc.GRPC_GROUPS)
        for script in ("echo phase=measure; exit 1", "true"):
            with self.subTest(script=script), self.assertRaises(mc.Failure):
                mc.run_cases([case], self.fake(script), io.StringIO())


class SummarizeTest(unittest.TestCase):
    @staticmethod
    def line(round_, build, args, fields):
        return f"round={round_}\tbuild={build}\targs={args}\tscenario=crash\tnodes=3\tgroups=6\t{fields}"

    def test_medians_of_each_build_for_each_case_and_phase(self):
        lines = []
        for round_, before, after in [(1, "480.0", "470.0"), (2, "500.0", "490.0"), (3, "490.0", "480.5")]:
            for build, elect in [("before", before), ("after", after)]:
                lines.append(self.line(round_, build, "", "phase=baseline\tappends/s=39098\tcpu%=231.3"))
                lines.append(self.line(round_, build, "", f"phase=failover\telect_p50_ms={elect}\tunrecovered=0"
                                                          "\tnote=n/a"))
        lines.append(self.line(1, "before", "-- --enable-pre-vote true", "phase=failover\telect_p50_ms=900"))
        lines.append(self.line(1, "after", "-- --enable-pre-vote true", "phase=failover\telect_p50_ms=n/a"))

        summary = mc.summarize(lines, mc.GRPC_GROUPS)

        sections = summary.split("### ")[1:]
        self.assertEqual(3, len(sections), summary)
        baseline, failover, pre_vote = sections
        self.assertTrue(baseline.startswith("scenario=crash, phase=baseline, nodes=3, groups=6\n"), baseline)
        self.assertIn("The medians of 3 rounds per build:", baseline)
        self.assertIn("| `appends/s` | 39,098 | 39,098 |", baseline)
        self.assertIn("| `cpu%` | 231.3 | 231.3 |", baseline)
        self.assertIn("| `elect_p50_ms` | 490 | 480.5 |", failover)
        self.assertIn("| `unrecovered` | 0 | 0 |", failover)
        self.assertNotIn("note", failover, "a field without a number in any run has no row")
        self.assertTrue(pre_vote.startswith("scenario=crash, phase=failover, nodes=3, groups=6, "
                                            "args: -- --enable-pre-vote true\n"), pre_vote)
        self.assertIn("| `elect_p50_ms` | 900 | n/a |", pre_vote)

    def test_idle_groups_lines_are_summarized_per_case(self):
        lines = [f"round={round_}\tbuild={build}\targs=--count-allocs\tgroups=1000\tfollowers=stub\tmode=default"
                 f"\theap_kib/group={heap}"
                 for round_, build, heap in [(1, "before", 114), (1, "after", 100), (2, "before", 115),
                                             (2, "after", 100)]]

        summary = mc.summarize(lines, mc.IDLE_GROUPS)

        self.assertIn("### groups=1000, followers=stub, mode=default, args: --count-allocs\n", summary)
        self.assertIn("The medians of 2 rounds per build:", summary)
        self.assertIn("| `heap_kib/group` | 114.5 | 100 |", summary)


class CheckoutTestCase(unittest.TestCase):
    """A checkout with a change that modifies `kept.txt`, removes `removed.txt` and adds `added.txt`."""

    FILES = [Path("kept.txt"), Path("removed.txt"), Path("added.txt")]

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "checkout"
        self.saved = Path(self.tmp.name) / "saved"
        self.root.mkdir()
        git(self.root, "init", "-q")
        (self.root / "kept.txt").write_text("base")
        (self.root / "removed.txt").write_text("removed by the change")
        git(self.root, "add", ".")
        git(self.root, "commit", "-q", "-m", "base")

        (self.root / "kept.txt").write_text("changed")
        (self.root / "removed.txt").unlink()
        (self.root / "added.txt").write_text("added by the change")

    def tearDown(self):
        self.tmp.cleanup()

    def read(self, name):
        path = self.root / name
        return path.read_text() if path.exists() else None


class CheckedFilesTest(CheckoutTestCase):
    def test_accepts_modified_removed_and_added_files(self):
        self.assertEqual(self.FILES, mc.checked_files(self.root, "HEAD", self.FILES))

    def test_rejects_outside_paths_directories_and_unknown_files(self):
        (self.root / "dir").mkdir()
        for name in ("/etc/hosts", "../outside.rs", "dir", "typo.txt"):
            with self.subTest(name=name), self.assertRaises(mc.Failure):
                mc.checked_files(self.root, "HEAD", [Path(name)])


class ChangeSwapTest(CheckoutTestCase):
    def test_sets_the_files_to_base_and_puts_the_change_back(self):
        swap = mc.ChangeSwap(self.root, "HEAD", self.FILES, self.saved)
        swap.save()

        swap.set_to_base()
        self.assertEqual(("base", "removed by the change", None),
                         (self.read("kept.txt"), self.read("removed.txt"), self.read("added.txt")))
        self.assertFalse(swap.restored())

        swap.restore()
        self.assertEqual(("changed", None, "added by the change"),
                         (self.read("kept.txt"), self.read("removed.txt"), self.read("added.txt")))
        self.assertTrue(swap.restored())

    def test_a_restored_file_gets_a_new_mtime(self):
        # Cargo rebuilds a crate only if a source file is newer than its last build, so a restored
        # file must not keep the mtime of its saved copy.
        long_ago = 1_000_000_000
        os.utime(self.root / "kept.txt", (long_ago, long_ago))
        swap = mc.ChangeSwap(self.root, "HEAD", [Path("kept.txt")], self.saved)
        swap.save()
        os.utime(self.saved / "kept.txt", (long_ago, long_ago))
        swap.set_to_base()

        swap.restore()

        self.assertGreater((self.root / "kept.txt").stat().st_mtime, long_ago + 86400)


if __name__ == "__main__":
    unittest.main()

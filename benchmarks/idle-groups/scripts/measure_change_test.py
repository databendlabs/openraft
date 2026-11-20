import argparse
import os
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
        case = mc.parse_case("3 1000 local default --count-allocs --settle-secs 5")

        self.assertEqual(3, case.runs)
        expected = ["--groups", "1000", "--followers", "local", "--mode", "default",
                    "--count-allocs", "--settle-secs", "5"]
        self.assertEqual(expected, case.args())

    def test_rejects_a_short_case_and_a_non_numeric_runs(self):
        for text in ("3 1000 stub", "x 1000 stub default"):
            with self.subTest(text=text), self.assertRaises(argparse.ArgumentTypeError):
                mc.parse_case(text)


class ArtifactExecutableTest(unittest.TestCase):
    def test_finds_the_idle_groups_binary(self):
        messages = "\n".join([
            '{"reason":"compiler-artifact","target":{"name":"openraft"},"executable":null}',
            '{"reason":"compiler-artifact","target":{"name":"idle-groups"},"executable":"/t/idle-groups"}',
            '{"reason":"build-finished","success":true}',
        ])

        self.assertEqual("/t/idle-groups", mc.artifact_executable(messages))


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

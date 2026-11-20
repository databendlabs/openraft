#!/usr/bin/env python3
"""Measure a change with idle-groups: build the benchmark without the change and with it, in one
checkout, then run each case with the two builds in turns, so that both see the same machine load.

For the build without the change, each file of the change is set to its version at --base, or
removed if --base has no such file, and then put back. Do not edit those files while it runs. The
results go to OUT_DIR/results/LABEL-<time>.tsv, one line per run, and both binaries stay in
OUT_DIR/bin.

Example, from the root of a checkout:
    benchmarks/idle-groups/scripts/measure_change.py my-change $(git diff --name-only) \\
        --case '3 1000 stub default' --case '1 1000 local default --count-allocs'
"""

import argparse
import filecmp
import json
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path

BENCH = Path("benchmarks/idle-groups")
BINARY = "idle-groups"


class Failure(Exception):
    """A failure to report without a traceback."""


@dataclass
class Case:
    """Run the benchmark `runs` times with each build, with these arguments."""

    runs: int
    groups: str
    followers: str
    mode: str
    extra: list[str] = field(default_factory=list)

    def args(self):
        return ["--groups", self.groups, "--followers", self.followers, "--mode", self.mode, *self.extra]


def parse_case(text):
    """Parse a case, "RUNS GROUPS FOLLOWERS MODE [EXTRA ARGS]"."""
    words = shlex.split(text)
    if len(words) < 4:
        raise argparse.ArgumentTypeError(f"a case is 'RUNS GROUPS FOLLOWERS MODE [EXTRA ARGS]', got: {text!r}")
    runs, groups, followers, mode, *extra = words
    if not re.fullmatch(r"[0-9]+", runs):
        raise argparse.ArgumentTypeError(f"RUNS must be a number, got: {runs!r}")
    return Case(int(runs), groups, followers, mode, extra)


class ChangeSwap:
    """The files of a change in the checkout `root`, set to their versions at `base` and back.

    `save()` copies each file into `saved`, or records that the change removed it.
    """

    def __init__(self, root, base, files, saved):
        self.root = root
        self.base = base
        self.files = files
        self.saved = saved
        self.absent = set()

    def save(self):
        for name in self.files:
            path = self.root / name
            if path.exists():
                copy = self.saved / name
                copy.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy(path, copy)
            else:
                self.absent.add(name)

    def set_to_base(self):
        """Set each file to its version at `base`, or remove it if `base` has none."""
        for name in self.files:
            path = self.root / name
            spec = f"{self.base}:{name.as_posix()}"
            if git(self.root, "cat-file", "-e", spec, check=False).returncode != 0:
                path.unlink(missing_ok=True)
                continue
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(git(self.root, "cat-file", "--filters", spec).stdout)

    def restore(self):
        """Put the change back. A copied file gets a new mtime, which makes cargo rebuild it."""
        for name in self.files:
            path = self.root / name
            if name in self.absent:
                path.unlink(missing_ok=True)
            else:
                shutil.copy(self.saved / name, path)

    def restored(self):
        """Return whether every file is as `save()` found it."""
        for name in self.files:
            path = self.root / name
            if name in self.absent:
                if path.exists():
                    return False
            elif not (path.is_file() and filecmp.cmp(path, self.saved / name, shallow=False)):
                return False
        return True


def git(root, *args, check=True):
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, check=check)


def checked_files(root, files):
    """Return `files` if each is a relative path to a file, or to nothing, in the checkout `root`.

    The script rewrites and removes these files, so it must not reach outside the checkout.
    """
    for name in files:
        if name.is_absolute() or ".." in name.parts:
            raise Failure(f"a file of the change must be relative to the checkout root, got: {name}")
        if (root / name).is_dir():
            raise Failure(f"a file of the change is a directory: {name}")
    return files


def artifact_executable(messages):
    """Return the path of the idle-groups binary, from the JSON messages of `cargo build`."""
    for line in messages.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact" or not message.get("executable"):
            continue
        if message.get("target", {}).get("name") == BINARY:
            return message["executable"]
    raise Failure(f"cargo reported no executable named {BINARY}")


def build(root, binary):
    """Build the benchmark in `root`, with the toolchain of `root`, and copy it to `binary`."""
    manifest = root / BENCH / "Cargo.toml"
    command = ["cargo", "build", "--release", "--quiet", "--message-format=json-render-diagnostics",
               "--manifest-path", str(manifest)]
    result = subprocess.run(command, cwd=root, stdout=subprocess.PIPE, text=True)
    if result.returncode != 0:
        raise Failure(f"build failed: {binary.name}")
    shutil.copy(artifact_executable(result.stdout), binary)


def run_cases(cases, binaries, results):
    """Run each case with each build in turns, and write one line per run to `results`."""
    for case in cases:
        for round_ in range(1, case.runs + 1):
            for build_name, binary in binaries.items():
                command = [str(binary), *case.args()]
                result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, text=True)
                lines = result.stdout.splitlines()
                if result.returncode != 0 or len(lines) != 1:
                    raise Failure(f"run failed, round {round_}: {shlex.join(command)}")
                results.write(f"round={round_}\tbuild={build_name}\targs={shlex.join(case.extra)}\t{lines[0]}\n")
                results.flush()


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("label", help="names the two binaries and the results file")
    parser.add_argument("files", nargs="+", type=Path, help="the files of the change, relative to the checkout root")
    parser.add_argument("--case", dest="cases", action="append", type=parse_case, required=True,
                        metavar="'RUNS GROUPS FOLLOWERS MODE [EXTRA ARGS]'",
                        help="run the benchmark RUNS times with each build; repeat for more cases")
    parser.add_argument("--base", default="HEAD", help="the revision to compare with (default: HEAD)")
    parser.add_argument("--checkout", type=Path,
                        help="the root of the checkout that holds the change (default: the current one)")
    parser.add_argument("--out-dir", type=Path,
                        help="where to keep the binaries and results (default: CHECKOUT/benchmarks/idle-groups/"
                             "target/measure-change)")
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9._-]+", args.label):
        parser.error(f"LABEL may hold only letters, digits, '.', '_' and '-', got: {args.label!r}")
    return args


def now():
    return time.strftime("%H:%M:%S", time.gmtime())


def main():
    args = parse_args()
    if args.checkout is None:
        top = git(Path.cwd(), "rev-parse", "--show-toplevel", check=False)
        if top.returncode != 0:
            raise Failure("the current directory is not in a git checkout; pass --checkout")
        args.checkout = Path(top.stdout.decode().strip())
    root = args.checkout.resolve()
    files = checked_files(root, args.files)
    if git(root, "rev-parse", "-q", "--verify", f"{args.base}^{{commit}}", check=False).returncode != 0:
        raise Failure(f"not a revision: {args.base}")
    out_dir = (args.out_dir or root / BENCH / "target" / "measure-change").resolve()
    (out_dir / "bin").mkdir(parents=True, exist_ok=True)
    (out_dir / "results").mkdir(parents=True, exist_ok=True)
    results_path = out_dir / "results" / f"{args.label}-{time.strftime('%Y%m%dT%H%M%S')}.tsv"
    binaries = {name: out_dir / "bin" / f"{BINARY}-{args.label}-{name}" for name in ("before", "after")}

    print(f"start: {now()}", flush=True)
    saved = Path(tempfile.mkdtemp(prefix="measure-change-"))
    swap = ChangeSwap(root, args.base, files, saved)
    swap.save()
    try:
        swap.set_to_base()
        build(root, binaries["before"])
    finally:
        try:
            swap.restore()
            restored = swap.restored()
        except OSError:
            restored = False
        if not restored:
            raise Failure(f"restore failed: the files of the change are saved in {saved}")
    shutil.rmtree(saved)
    build(root, binaries["after"])
    print(f"built both: {now()}", flush=True)

    base = git(root, "rev-parse", "--short", args.base).stdout.decode().strip()
    head = git(root, "rev-parse", "--short", "HEAD").stdout.decode().strip()
    with results_path.open("w") as results:
        results.write(f"# {args.label}: before = the files at {base}; after = {head} with the change\n")
        results.write("# release builds of benchmarks/idle-groups; its README describes the fields\n")
        run_cases(args.cases, binaries, results)
    print(f"wrote {results_path}")
    print(f"done: {now()}")


def exit_on_sigterm(_signum, _frame):
    # Exit through the `finally` of main(), which puts the change back.
    sys.exit(128 + signal.SIGTERM)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, exit_on_sigterm)
    try:
        main()
    except Failure as e:
        sys.exit(f"{Path(sys.argv[0]).name}: {e}")

#!/usr/bin/env python3
"""Measure a change with idle-groups or grpc-groups: build the benchmark without the change and with
it, in one checkout, then run each case with the two builds in turns, so that both see the same
machine load.

For the build without the change, each file of the change is set to its version at --base, or
removed if --base has no such file, and then put back. Do not edit those files while it runs. The
results go to OUT_DIR/results/LABEL-<time>.tsv, one line for each line that a run prints: one per
run with idle-groups, one per phase with grpc-groups. Both binaries stay in OUT_DIR/bin. At the
end, the script prints the median of each field for each build.

Examples, from the root of a checkout:
    benchmarks/idle-groups/scripts/measure_change.py my-change $(git diff --name-only) \\
        --case '3 1000 stub default' --case '1 1000 local default --count-allocs'
    benchmarks/idle-groups/scripts/measure_change.py my-change $(git diff --name-only) \\
        --bench grpc-groups --case '3 steady' --case '3 crash -- --enable-pre-vote true'
"""

import argparse
import filecmp
import json
import math
import re
import shlex
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path


class Failure(Exception):
    """A failure to report without a traceback."""


@dataclass(frozen=True)
class Bench:
    """A benchmark crate under `benchmarks/`, whose binary has the name of its directory."""

    name: str

    # The words of a case between RUNS and its extra args.
    case_words: tuple[str, ...]

    # The arguments of the binary that those words stand for.
    command: Callable[[list[str]], list[str]]

    # The fields that tell what a line of results measured. The summary takes the median over the
    # lines of one case that have the same values of these fields.
    key_fields: tuple[str, ...]

    @property
    def path(self):
        return Path("benchmarks") / self.name

    @property
    def case_form(self):
        return " ".join(("RUNS", *self.case_words, "[EXTRA ARGS]"))


IDLE_GROUPS = Bench(
    name="idle-groups",
    case_words=("GROUPS", "FOLLOWERS", "MODE"),
    command=lambda words: ["--groups", words[0], "--followers", words[1], "--mode", words[2]],
    key_fields=("groups", "followers", "mode"),
)

# The extra args of a case follow `--scenario`, so that openraft `Config` flags can come last, after
# `--`.
GRPC_GROUPS = Bench(
    name="grpc-groups",
    case_words=("SCENARIO",),
    command=lambda words: ["run", "--scenario", words[0]],
    key_fields=("scenario", "phase", "nodes", "groups"),
)

BENCHES = {bench.name: bench for bench in (IDLE_GROUPS, GRPC_GROUPS)}


@dataclass
class Case:
    """Run `bench` `runs` times with each build, with the arguments of `words`, then `extra`."""

    bench: Bench
    runs: int
    words: list[str]
    extra: list[str] = field(default_factory=list)

    def args(self):
        return [*self.bench.command(self.words), *self.extra]


def parse_case(text, bench):
    """Parse a case of `bench`, such as "RUNS GROUPS FOLLOWERS MODE [EXTRA ARGS]" for idle-groups."""
    words = shlex.split(text)
    if len(words) < 1 + len(bench.case_words):
        raise argparse.ArgumentTypeError(f"a case of {bench.name} is '{bench.case_form}', got: {text!r}")
    runs, *rest = words
    if not re.fullmatch(r"[0-9]+", runs):
        raise argparse.ArgumentTypeError(f"RUNS must be a number, got: {runs!r}")
    count = len(bench.case_words)
    return Case(bench, int(runs), rest[:count], rest[count:])


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
            if not in_revision(self.root, self.base, name):
                path.unlink(missing_ok=True)
                continue
            path.parent.mkdir(parents=True, exist_ok=True)
            spec = f"{self.base}:{name.as_posix()}"
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


def in_revision(root, revision, name):
    return git(root, "cat-file", "-e", f"{revision}:{name.as_posix()}", check=False).returncode == 0


def checked_files(root, base, files):
    """Return `files` if each is a relative path to a file in the checkout `root` or at `base`.

    The script rewrites and removes these files, so none may lie outside the checkout. A name in
    neither place, such as a typo, would leave the change of the intended file in both builds.
    """
    for name in files:
        if name.is_absolute() or ".." in name.parts:
            raise Failure(f"a file of the change must be relative to the checkout root, got: {name}")
        path = root / name
        if path.is_dir():
            raise Failure(f"a file of the change is a directory: {name}")
        if not path.exists() and not in_revision(root, base, name):
            raise Failure(f"a file of the change is neither in the checkout nor at {base}: {name}")
    return files


def artifact_executable(messages, binary):
    """Return the path of the executable named `binary`, from the JSON messages of `cargo build`."""
    for line in messages.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact" or not message.get("executable"):
            continue
        if message.get("target", {}).get("name") == binary:
            return message["executable"]
    raise Failure(f"cargo reported no executable named {binary}")


def build(root, bench, binary):
    """Build `bench` in `root`, with the toolchain of `root`, and copy its executable to `binary`."""
    manifest = root / bench.path / "Cargo.toml"
    command = ["cargo", "build", "--release", "--quiet", "--message-format=json-render-diagnostics",
               "--manifest-path", str(manifest)]
    result = subprocess.run(command, cwd=root, stdout=subprocess.PIPE, text=True)
    if result.returncode != 0:
        raise Failure(f"build failed: {binary.name}")
    shutil.copy(artifact_executable(result.stdout, bench.name), binary)


def run_cases(cases, binaries, results):
    """Run each case with each build in turns, and write each line a run prints to `results`.

    Return the lines that were written.
    """
    written = []
    for case in cases:
        for round_ in range(1, case.runs + 1):
            for build_name, binary in binaries.items():
                command = [str(binary), *case.args()]
                result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, text=True)
                lines = result.stdout.splitlines()
                if result.returncode != 0 or not lines:
                    raise Failure(f"run failed, round {round_}: {shlex.join(command)}")
                for line in lines:
                    written.append(f"round={round_}\tbuild={build_name}\targs={shlex.join(case.extra)}\t{line}")
                    results.write(f"{written[-1]}\n")
                results.flush()
    return written


def parse_fields(line):
    """Return the `name=value` fields of a line of results, in their order."""
    fields = {}
    for item in line.split("\t"):
        name, sep, value = item.partition("=")
        if sep:
            fields[name] = value
    return fields


def number(value):
    """Return `value` as a finite number, or None if it is not one, such as `n/a`."""
    try:
        parsed = float(value)
    except ValueError:
        return None
    return parsed if math.isfinite(parsed) else None


def format_number(value):
    """Format `value` with thousands separators and at most two decimals."""
    return f"{value:,.2f}".rstrip("0").rstrip(".")


def summarize(lines, bench, builds=("before", "after")):
    """Return a Markdown summary of `lines` of results: for each case, and for each value of the
    key fields of `bench` in it, a table of the median of each numeric field for each build.
    """
    groups = {}
    for line in lines:
        fields = parse_fields(line)
        key = (fields.get("args", ""), *(fields.get(name, "") for name in bench.key_fields))
        groups.setdefault(key, []).append(fields)

    skipped = {"round", "build", "args", *bench.key_fields}
    sections = []
    for (args, *values), group in groups.items():
        rounds = len({fields.get("round") for fields in group})
        heading = ", ".join(f"{name}={value}" for name, value in zip(bench.key_fields, values) if value)
        if args:
            heading += f", args: {args}"

        names = [name for name in dict.fromkeys(name for fields in group for name in fields) if name not in skipped]
        rows = []
        for name in names:
            medians = []
            for build_name in builds:
                samples = [number(fields.get(name, "")) for fields in group if fields.get("build") == build_name]
                samples = [sample for sample in samples if sample is not None]
                medians.append(format_number(statistics.median(samples)) if samples else "n/a")
            if any(median != "n/a" for median in medians):
                rows.append(f"| `{name}` | " + " | ".join(medians) + " |")

        header = "| field | " + " | ".join(builds) + " |"
        separator = "|---|" + "--:|" * len(builds)
        table = "\n".join([header, separator, *rows])
        sections.append(f"### {heading}\n\nThe medians of {rounds} rounds per build:\n\n{table}\n")
    return "\n".join(sections)


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("label", help="names the two binaries and the results file")
    parser.add_argument("files", nargs="+", type=Path, help="the files of the change, relative to the checkout root")
    parser.add_argument("--bench", choices=sorted(BENCHES), default=IDLE_GROUPS.name,
                        help=f"the benchmark under benchmarks/ to build and run (default: {IDLE_GROUPS.name})")
    parser.add_argument("--case", dest="cases", action="append", required=True,
                        metavar="'RUNS ... [EXTRA ARGS]'",
                        help=f"run the benchmark RUNS times with each build; repeat for more cases. A case is "
                             f"'{IDLE_GROUPS.case_form}' for {IDLE_GROUPS.name}, and "
                             f"'{GRPC_GROUPS.case_form}' for {GRPC_GROUPS.name}")
    parser.add_argument("--base", default="HEAD", help="the revision to compare with (default: HEAD)")
    parser.add_argument("--checkout", type=Path,
                        help="the root of the checkout that holds the change (default: the current one)")
    parser.add_argument("--out-dir", type=Path,
                        help="where to keep the binaries and results (default: CHECKOUT/benchmarks/BENCH/"
                             "target/measure-change)")
    args = parser.parse_args(argv)
    if not re.fullmatch(r"[A-Za-z0-9._-]+", args.label):
        parser.error(f"LABEL may hold only letters, digits, '.', '_' and '-', got: {args.label!r}")
    args.bench = BENCHES[args.bench]
    try:
        args.cases = [parse_case(text, args.bench) for text in args.cases]
    except argparse.ArgumentTypeError as e:
        parser.error(str(e))
    return args


def now():
    return time.strftime("%H:%M:%S", time.gmtime())


def main():
    args = parse_args()
    bench = args.bench
    if args.checkout is None:
        top = git(Path.cwd(), "rev-parse", "--show-toplevel", check=False)
        if top.returncode != 0:
            raise Failure("the current directory is not in a git checkout; pass --checkout")
        args.checkout = Path(top.stdout.decode().strip())
    root = args.checkout.resolve()
    if git(root, "rev-parse", "-q", "--verify", f"{args.base}^{{commit}}", check=False).returncode != 0:
        raise Failure(f"not a revision: {args.base}")
    files = checked_files(root, args.base, args.files)
    out_dir = (args.out_dir or root / bench.path / "target" / "measure-change").resolve()
    (out_dir / "bin").mkdir(parents=True, exist_ok=True)
    (out_dir / "results").mkdir(parents=True, exist_ok=True)
    results_path = out_dir / "results" / f"{args.label}-{time.strftime('%Y%m%dT%H%M%S')}.tsv"
    binaries = {name: out_dir / "bin" / f"{bench.name}-{args.label}-{name}" for name in ("before", "after")}

    print(f"start: {now()}", flush=True)
    saved = Path(tempfile.mkdtemp(prefix="measure-change-"))
    swap = ChangeSwap(root, args.base, files, saved)
    swap.save()
    try:
        swap.set_to_base()
        build(root, bench, binaries["before"])
    finally:
        try:
            swap.restore()
            restored = swap.restored()
        except OSError:
            restored = False
        if not restored:
            raise Failure(f"restore failed: the files of the change are saved in {saved}")
    shutil.rmtree(saved)
    build(root, bench, binaries["after"])
    print(f"built both: {now()}", flush=True)

    base = git(root, "rev-parse", "--short", args.base).stdout.decode().strip()
    head = git(root, "rev-parse", "--short", "HEAD").stdout.decode().strip()
    with results_path.open("w") as results:
        results.write(f"# {args.label}: before = the files at {base}; after = {head} with the change\n")
        results.write(f"# release builds of {bench.path.as_posix()}; its README describes the fields\n")
        lines = run_cases(args.cases, binaries, results)
    print(f"wrote {results_path}")
    print(f"done: {now()}")
    print()
    print(f"before = the files at {base}; after = {head} with the change\n")
    print(summarize(lines, bench), end="")


def exit_on_sigterm(_signum, _frame):
    # Exit through the `finally` of main(), which puts the change back.
    sys.exit(128 + signal.SIGTERM)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, exit_on_sigterm)
    try:
        main()
    except (Failure, OSError) as e:
        sys.exit(f"{Path(sys.argv[0]).name}: {e}")

#!/usr/bin/env python3
"""Check that markdown links pointing into this repository resolve to a file.

Two link forms are checked:

- relative paths, such as `[client](../app-http/src/client.rs)`, resolved
  against the directory holding the file that contains them;
- GitHub URLs pinned to `main`, such as
  `https://github.com/databendlabs/openraft/blob/main/examples/...`, translated
  back into a path in the working tree.

Everything else is left alone:

- URLs to other hosts, and URLs to this repository pinned to a commit or a tag,
  since those name a historical snapshot rather than the current tree;
- pure `#anchor` links;
- Rustdoc intra-doc targets such as `crate::docs::getting_started`, which are
  item paths rather than file paths, and would otherwise be reported as missing
  files;
- markdown files that are symlinks, since the file they point at is checked at
  its own path, where its relative links resolve.

Anchors are stripped before the check, so `README.md#two-servers-per-node`
passes when `README.md` exists; the anchor itself is not validated.

Usage:

    ./scripts/check-doc-links.py             # every tracked markdown file
    ./scripts/check-doc-links.py openraft/   # only files under a path

Exits 1 if any link is broken, so it can gate CI. It writes nothing.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

# A link into this repository at its current state. Only `main` is translated:
# a URL pinned to a commit or tag names a snapshot, not the working tree.
REPO_URL = re.compile(r"^https://github\.com/databendlabs/openraft/(?:blob|tree)/main/(.+)$")

# `[text](destination)`, with an optional `"title"` and optional `<>` around
# the destination.
INLINE_LINK = re.compile(r"\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+[\"'][^\"']*[\"'])?\s*\)")

# `[label]: destination`, the reference-style definition.
REFERENCE_DEF = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*<?([^\s>]+)>?\s*(?:[\"'].*[\"'])?\s*$")


def repo_root() -> Path:
    out = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True
    )
    return Path(out.stdout.strip())


def markdown_files(root: Path, paths: list[str]) -> list[Path]:
    """Tracked markdown files under `paths`, symlinks excluded."""
    cmd = ["git", "ls-files", "-z", "--"] + [p + "*.md" if p.endswith("/") else p for p in paths]
    out = subprocess.run(cmd, cwd=root, capture_output=True, text=True, check=True)
    files = []
    for name in out.stdout.split("\0"):
        if not name.endswith(".md"):
            continue
        if (root / name).is_symlink():
            continue
        files.append(Path(name))
    return files


def destinations(line: str) -> list[str]:
    found = INLINE_LINK.findall(line)
    ref = REFERENCE_DEF.match(line)
    if ref:
        found.append(ref.group(1))
    return found


def resolve(root: Path, md_file: Path, dest: str) -> Path | None:
    """The working-tree path `dest` refers to, or None if it refers elsewhere."""
    repo_url = REPO_URL.match(dest)
    if repo_url:
        target = repo_url.group(1)
        base = root
    elif "://" in dest or dest.startswith(("#", "mailto:", "`")) or "::" in dest:
        return None
    elif dest.startswith("/"):
        target = dest.lstrip("/")
        base = root
    else:
        target = dest
        base = (root / md_file).parent

    target = target.split("#")[0].split("?")[0]
    if not target:
        return None
    return base / target


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "paths", nargs="*", default=["*.md"], help="limit the scan to these paths (default: all)"
    )
    args = parser.parse_args()

    root = repo_root()
    checked = 0
    broken = []

    for md_file in markdown_files(root, args.paths):
        lines = (root / md_file).read_text(encoding="utf-8").splitlines()
        for lineno, line in enumerate(lines, 1):
            for dest in destinations(line):
                path = resolve(root, md_file, dest)
                if path is None:
                    continue
                checked += 1
                if not path.exists():
                    broken.append(f"{md_file}:{lineno}: {dest}")

    if broken:
        print(f"broken repository links: {len(broken)}", file=sys.stderr)
        for item in broken:
            print(f"  {item}", file=sys.stderr)
        return 1

    print(f"checked {checked} repository links, all resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())

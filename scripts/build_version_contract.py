#!/usr/bin/env python3
"""Generate the version contract at the top of the getting-started guide.

The guide opens with the release line an application should depend on and the
exact dependency declaration to copy. Both are derived rather than typed, from
two sources that cannot drift from the code:

- the version comes from `[workspace.package] version` in the root `Cargo.toml`;
- the dependency name and feature list come from `tests-consumer/Cargo.toml`,
  the fixture that compiles that declaration outside the root workspace.

Generating from the fixture is what keeps the documented declaration buildable:
the guide can only claim a feature set that something in CI compiles.

Usage:

    ./scripts/build_version_contract.py            # rewrite the guide
    ./scripts/build_version_contract.py --check     # fail if the guide is stale

`make doc` regenerates; `make docs-check` and the CI lint job check. The check
writes nothing.
"""

import argparse
import sys
import tomllib
from pathlib import Path

GUIDE = Path("openraft/src/docs/getting_started/getting-started.md")
FIXTURE = Path("tests-consumer/Cargo.toml")
ROOT_MANIFEST = Path("Cargo.toml")

BEGIN = "<!-- BEGIN GENERATED VERSION CONTRACT: scripts/build_version_contract.py -->"
END = "<!-- END GENERATED VERSION CONTRACT -->"

TEMPLATE = """\
{begin}
This chapter describes Openraft `{version}`, the {line} line,
developed on branch `main`. An application depends on it with:

```toml
[dependencies]
openraft = {{ version = "{version}", features = [{features}] }}
```

`{fixture}` compiles this declaration on its own, outside this
repository's workspace, so the feature list above is enough by itself.
{end}\
"""


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def release_line(version: str) -> str:
    """`0.10.0-alpha.34` -> `0.10 prerelease`, `0.9.0` -> `0.9 stable`."""
    major, minor = version.split(".")[:2]
    stage = "prerelease" if "-" in version else "stable"
    return f"{major}.{minor} {stage}"


def render(root: Path) -> str:
    workspace = tomllib.loads((root / ROOT_MANIFEST).read_text(encoding="utf-8"))
    version = workspace["workspace"]["package"]["version"]

    fixture = tomllib.loads((root / FIXTURE).read_text(encoding="utf-8"))
    dependency = fixture["dependencies"]["openraft"]

    if dependency["version"] != version:
        raise SystemExit(
            f"{FIXTURE} depends on openraft {dependency['version']}, "
            f"but the workspace is at {version}; update the fixture."
        )

    features = ", ".join(f'"{f}"' for f in dependency["features"])
    return TEMPLATE.format(
        begin=BEGIN,
        end=END,
        version=version,
        line=release_line(version),
        features=features,
        fixture=FIXTURE,
    )


def splice(guide_text: str, block: str) -> str:
    start = guide_text.index(BEGIN)
    stop = guide_text.index(END) + len(END)
    return guide_text[:start] + block + guide_text[stop:]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check", action="store_true", help="report a stale guide instead of rewriting it"
    )
    args = parser.parse_args()

    root = repo_root()
    guide_path = root / GUIDE
    current = guide_path.read_text(encoding="utf-8")
    updated = splice(current, render(root))

    if current == updated:
        print(f"{GUIDE}: version contract is current")
        return 0

    if args.check:
        print(
            f"{GUIDE}: version contract is stale; run `./{Path(__file__).name}` "
            f"from the repository root, or `make doc`",
            file=sys.stderr,
        )
        return 1

    guide_path.write_text(updated, encoding="utf-8")
    print(f"{GUIDE}: version contract regenerated")
    return 0


if __name__ == "__main__":
    sys.exit(main())

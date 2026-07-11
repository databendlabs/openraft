#!/usr/bin/env python3
"""
List every OpenRaft contributor and build README-ready Markdown.

Collects two groups from GitHub — all-time commit authors and the authors of
issues / pull requests within a time window — and unions them into the full
contributor set (bots excluded). Writes two files: the sorted login list (which
generate_contributor_svgs.py renders into avatar SVGs) and a Markdown block that
places each contributor's avatar SVG as an image linking to their GitHub
profile, ready to copy-paste under the README Contributors section.

Usage:
    scripts/contributors/list_contributors.py [--repo OWNER/NAME] [--since YYYY-MM-DD]
        [--dir DIR] [--txt NAME] [--markdown NAME]

Defaults: --repo databendlabs/openraft, --since one year ago,
          --dir assets/contributors, --txt contributors.txt, --markdown contributors.md.
The list and Markdown are written into --dir; the SVG path prefix used in the
Markdown is derived as <dir>/svg (where generate_contributor_svgs.py writes).
Requires: gh (authenticated).
"""

import argparse
import datetime
import re
import subprocess
import sys
from pathlib import Path

BOT_RE = re.compile(r"\[bot\]$|^Copilot$", re.IGNORECASE)


def gh_lines(*args):
    """Run a `gh` command and return stdout as a list of non-empty lines."""
    result = subprocess.run(["gh", *args], capture_output=True, text=True, check=True)
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]


def commit_authors(repo):
    """All-time commit contributors: everyone with a merged commit."""
    return set(gh_lines(
        "api", "--paginate", f"repos/{repo}/contributors?per_page=100",
        "--jq", ".[].login",
    ))


def issue_pr_authors(repo, since):
    """Authors of issues and pull requests created since `since`."""
    authors = set()
    for kind in ("prs", "issues"):
        authors.update(gh_lines(
            "search", kind, "--repo", repo, "--created", f">={since}",
            "--limit", "1000", "--json", "author", "--jq", ".[].author.login",
        ))
    return authors


def all_contributors(repo, since):
    """Every contributor: commit authors ∪ issue/PR authors, bots excluded, sorted."""
    everyone = commit_authors(repo) | issue_pr_authors(repo, since)
    return sorted((a for a in everyone if not BOT_RE.search(a)), key=str.lower)


def render_markdown(names, svg_dir):
    """Linked-image Markdown: each contributor's avatar SVG links to their GitHub profile."""
    return "\n".join(
        f'[<img src="{svg_dir}/{name}.svg" alt="@{name}"/>](https://github.com/{name})'
        for name in names
    )


def main():
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--repo", default="databendlabs/openraft", help="GitHub owner/name")
    parser.add_argument("--since", help="ISO date lower bound for issues/PRs (default: one year ago)")
    parser.add_argument("--dir", default="assets/contributors", help="output directory for the list, Markdown, and svg/ subdir")
    parser.add_argument("--txt", default="contributors.txt", help="login-list filename within --dir")
    parser.add_argument("--markdown", default="contributors.md", help="Markdown filename within --dir")
    args = parser.parse_args()

    since = args.since or (datetime.date.today() - datetime.timedelta(days=365)).isoformat()
    names = all_contributors(args.repo, since)

    directory = Path(args.dir)
    directory.mkdir(parents=True, exist_ok=True)
    txt_path = directory / args.txt
    markdown_path = directory / args.markdown
    svg_dir = f"{directory.as_posix()}/svg"
    txt_path.write_text("\n".join(names) + "\n")
    markdown_path.write_text(render_markdown(names, svg_dir) + "\n")
    print(f"wrote {len(names)} logins to {txt_path} and Markdown to {markdown_path}", file=sys.stderr)


if __name__ == "__main__":
    main()

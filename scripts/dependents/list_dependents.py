#!/usr/bin/env python3
"""List the GitHub repositories that depend on a repository.

GitHub publishes its dependency graph only as HTML at `/network/dependents`;
neither the REST nor the GraphQL API serves this data. This script walks those
pages through the `gh` CLI, so the requests reuse the user's authenticated
session/token.

A repository that publishes several packages (openraft, openraft-memstore, ...)
gets one dependents list per package. The script walks every package and reports
the union, keeping each repository once.

Star counts come from the dependents pages themselves; descriptions are not
listed there and are fetched separately, one GraphQL query per 100 repositories.

Output is one `stars<TAB>owner/repo<TAB>description` row per repository on
stdout, sorted by descending star count, so it can be saved for later use.
Progress goes to stderr and stays out of the saved file.

Examples:

    # Every dependent of databendlabs/openraft
    python scripts/dependents/list_dependents.py > dependents.txt

    # Only the dependents with at least 100 stars
    python scripts/dependents/list_dependents.py --min-stars 100 > star-100.txt

    # Another repository
    python scripts/dependents/list_dependents.py tokio-rs/tokio > tokio.txt

A saved file can be filtered again without crawling GitHub a second time, which
is what `make -C scripts/dependents stars` does:

    awk -F'\\t' '$1 >= 100' dependents.txt
"""

import argparse
import html
import json
import re
import subprocess
import sys
import time
from collections.abc import Iterable

DEFAULT_REPO = "databendlabs/openraft"

# Repositories per GraphQL query when fetching descriptions.
DESCRIPTION_BATCH_SIZE = 100

# A page holds 30 rows; this cap is far above any real dependent list and only
# guards against a pagination loop.
MAX_PAGES_PER_PACKAGE = 200

# Pause between page requests, to stay friendly to github.com.
REQUEST_DELAY_SEC = 0.3

# The dependents page repeats one such block per listed repository.
DEPENDENT_BLOCK_MARKER = 'data-test-id="dg-repo-pkg-dependent"'

# Every package of the repository, as `package_id` plus the name shown in the
# package selector.
PACKAGE_PATTERN = re.compile(
    r'package_id=([A-Za-z0-9_%-]+)".*?select-menu-item-text">\s*([^<\n]+)',
    re.DOTALL,
)
DEPENDENT_REPO_PATTERN = re.compile(
    r'class="text-bold" data-hovercard-type="repository"[^>]*href="/([^"]+)"'
)
# The count sits below the star icon, after the inlined SVG path.
STAR_COUNT_PATTERN = re.compile(r"octicon-star.*?</svg>\s*([\d,.]+[kKmM]?)", re.DOTALL)
NEXT_PAGE_PATTERN = re.compile(r'href="([^"]+)"[^>]*>Next<')
REPO_SLUG_PATTERN = re.compile(r"^[^/\s]+/[^/\s]+$")


def log(message: str) -> None:
    """Report progress on stderr, leaving stdout to the two lists."""

    print(message, file=sys.stderr)


def fetch_page(url: str) -> str:
    """Return the HTML of `url`, fetched with `gh` so it carries the gh token."""

    cmd = ["gh", "api", url]
    result = subprocess.run(cmd, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise subprocess.CalledProcessError(result.returncode, cmd, result.stdout, result.stderr)
    return result.stdout


def to_star_count(text: str) -> int:
    """Convert a rendered star count ("1,234", "8.9k") to an integer."""

    digits = text.replace(",", "")
    suffix = digits[-1]
    if suffix in "kK":
        return int(float(digits[:-1]) * 1_000)
    if suffix in "mM":
        return int(float(digits[:-1]) * 1_000_000)
    return int(digits)


def parse_packages(page: str) -> list[tuple[str, str]]:
    """Return the (package_id, package_name) of every package of the repository.

    A repository publishing a single package has no package selector, and the
    returned list is then empty.
    """

    packages: list[tuple[str, str]] = []
    seen: set[str] = set()
    for package_id, package_name in PACKAGE_PATTERN.findall(page):
        if package_id in seen:
            continue
        seen.add(package_id)
        packages.append((package_id, package_name.strip()))
    return packages


def parse_dependents(page: str) -> list[tuple[str, int]]:
    """Return the (owner/repo, stars) of every dependent listed on `page`."""

    dependents: list[tuple[str, int]] = []
    for block in page.split(DEPENDENT_BLOCK_MARKER)[1:]:
        repo_match = DEPENDENT_REPO_PATTERN.search(block)
        star_match = STAR_COUNT_PATTERN.search(block)
        if repo_match is None or star_match is None:
            continue
        dependents.append((repo_match.group(1), to_star_count(star_match.group(1))))
    return dependents


def parse_next_page_url(page: str) -> str | None:
    """Return the URL behind the "Next" button, or None on the last page."""

    match = NEXT_PAGE_PATTERN.search(page)
    if match is None:
        return None
    return html.unescape(match.group(1))


def crawl_package(url: str) -> list[tuple[str, int]]:
    """Return the dependents of one package, following the pagination links."""

    dependents: list[tuple[str, int]] = []
    next_url: str | None = url
    page_number = 1

    while next_url is not None:
        if page_number > MAX_PAGES_PER_PACKAGE:
            log(f"  warning: stopped at the {MAX_PAGES_PER_PACKAGE} page cap; later pages are missing")
            break

        page = fetch_page(next_url)
        page_dependents = parse_dependents(page)
        dependents.extend(page_dependents)
        log(f"  page {page_number}: {len(page_dependents)} repositories")

        next_url = parse_next_page_url(page)
        page_number += 1
        time.sleep(REQUEST_DELAY_SEC)

    return dependents


def collect_dependents(repo: str) -> dict[str, int]:
    """Return {owner/repo: stars} for every package of `repo`."""

    base_url = f"https://github.com/{repo}/network/dependents"

    log(f"Fetching packages of {repo}")
    packages = parse_packages(fetch_page(base_url))

    crawled: list[tuple[str, int]] = []
    if not packages:
        log(f"  {repo} publishes a single package")
        crawled = crawl_package(base_url)
    else:
        log(f"  {repo} publishes {len(packages)} packages")
        for package_id, package_name in packages:
            log(f"package {package_name}")
            crawled.extend(crawl_package(f"{base_url}?package_id={package_id}"))

    # A repository depending on several packages is listed once per package.
    dependents: dict[str, int] = {}
    for dependent, stars in crawled:
        if stars > dependents.get(dependent, -1):
            dependents[dependent] = stars
    return dependents


def build_description_query(repos: Iterable[str]) -> str:
    """Build a GraphQL query asking for the description of every repository."""

    fields = []
    for index, repo in enumerate(repos):
        owner, name = repo.split("/", 1)
        alias = f"r{index}"
        selector = f"owner: {json.dumps(owner)}, name: {json.dumps(name)}"
        fields.append(f"{alias}: repository({selector}) {{ description }}")
    return "query { " + " ".join(fields) + " }"


def run_graphql(query: str) -> dict:
    """Run one GraphQL query through `gh` and return its `data` object.

    A repository that was deleted or renamed away resolves to null and makes
    `gh` exit non-zero, while the response body still carries all the others,
    so the body is parsed before the exit code is considered.
    """

    cmd = ["gh", "api", "graphql", "-f", f"query={query}"]
    result = subprocess.run(cmd, text=True, capture_output=True, check=False)
    if not result.stdout.strip():
        raise subprocess.CalledProcessError(result.returncode, cmd, result.stdout, result.stderr)

    data = json.loads(result.stdout).get("data")
    if data is None:
        raise subprocess.CalledProcessError(result.returncode, cmd, result.stdout, result.stderr)
    return data


def fetch_descriptions(repos: list[str]) -> dict[str, str]:
    """Return {owner/repo: description} for `repos`, batching the queries.

    A repository that can no longer be resolved gets an empty description.
    """

    descriptions: dict[str, str] = {}
    for start in range(0, len(repos), DESCRIPTION_BATCH_SIZE):
        batch = repos[start : start + DESCRIPTION_BATCH_SIZE]
        log(f"  descriptions {start + 1}-{start + len(batch)} of {len(repos)}")

        data = run_graphql(build_description_query(batch))
        for index, repo in enumerate(batch):
            repository = data.get(f"r{index}") or {}
            text = repository.get("description") or ""
            # Keep one row per repository: no tab, no newline.
            descriptions[repo] = " ".join(text.split())
    return descriptions


def print_rows(dependents: Iterable[tuple[str, int]], descriptions: dict[str, str]) -> None:
    """Print one `stars<TAB>owner/repo<TAB>description` row per repository."""

    for dependent, stars in dependents:
        print(f"{stars}\t{dependent}\t{descriptions[dependent]}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "repo",
        nargs="?",
        default=DEFAULT_REPO,
        help=f"repository whose dependents to list, as OWNER/REPO (default: {DEFAULT_REPO})",
    )
    parser.add_argument(
        "-s",
        "--min-stars",
        type=int,
        default=None,
        help="keep only the repositories with at least MIN_STARS stars (default: keep all)",
    )
    args = parser.parse_args()

    if REPO_SLUG_PATTERN.match(args.repo) is None:
        parser.error(f"repository must be given as OWNER/REPO, got {args.repo!r}")
    if args.min_stars is not None and args.min_stars < 0:
        parser.error(f"--min-stars takes a non-negative integer, got {args.min_stars}")

    dependents = collect_dependents(args.repo)
    ranked = sorted(dependents.items(), key=lambda item: (-item[1], item[0]))

    selected = ranked
    if args.min_stars is not None:
        selected = [item for item in ranked if item[1] >= args.min_stars]
        log(f"Selected {len(selected)} of {len(ranked)} repositories with at least {args.min_stars} stars")

    log(f"Fetching descriptions of {len(selected)} repositories")
    descriptions = fetch_descriptions([dependent for dependent, _stars in selected])

    print_rows(selected, descriptions)

    log(f"Done: {len(selected)} repositories")


if __name__ == "__main__":
    main()

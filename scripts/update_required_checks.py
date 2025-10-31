#!/usr/bin/env python3
"""Synchronise required branch status checks with those observed on a pull request.

This helper is intended for maintainers who want to keep the "Require status
checks to pass before merging" list (see the OpenRaft rule
`https://github.com/databendlabs/openraft/settings/branch_protection_rules/28100923`
for an example) aligned with the CI jobs that actually ran on a recent PR.
It shells out to the `gh` CLI so that it automatically reuses the user's
authenticated session/token. Run it from inside the repository you want to
update (or provide `--repo` explicitly).

The workflow is:

1. Inspect a pull request and record every status/check that completed on its
   latest commit.
2. Fetch the current branch protection rule for the PR's base branch, printing
   the set of currently required checks for context.
3. Replace the rule's required status checks with the observed list (dry-run by
   default; pass `--apply` to persist the change).

Examples:

    # Review the changes that would be applied
    python scripts/update_required_checks.py --pr 1479

    # Apply the update to branch protection
    python scripts/update_required_checks.py --pr 1479 --apply

    # Explicitly target another repo / branch
    python scripts/update_required_checks.py \
        --repo databendlabs/openraft \
        --branch release/0.10 \
        --pr 2001 \
        --apply
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from typing import Dict, Iterable, List, Optional, Sequence, Set


def run_gh(args: Sequence[str], *, input_text: Optional[str] = None) -> subprocess.CompletedProcess:
    """Run a `gh` command and return the completed process, raising on failure."""

    cmd = ["gh", *args]
    result = subprocess.run(
        cmd,
        input=input_text,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        sys.stderr.write(f"Command failed: {' '.join(cmd)}\n{result.stderr}\n")
        raise subprocess.CalledProcessError(result.returncode, cmd, result.stdout, result.stderr)
    return result


def gh_json(args: Sequence[str]) -> Dict:
    """Invoke `gh ...` and parse stdout as JSON."""

    cp = run_gh(args)
    if not cp.stdout.strip():
        return {}
    return json.loads(cp.stdout)


def detect_repo_slug() -> str:
    """Return the canonical owner/name for the current repository."""

    data = gh_json(["repo", "view", "--json", "nameWithOwner,parent"])
    parent = data.get("parent") or {}
    if parent:
        owner = parent.get("owner", {}).get("login")
        name = parent.get("name")
        if owner and name:
            return f"{owner}/{name}"

    slug = data.get("nameWithOwner")
    if not slug:
        raise RuntimeError("Failed to determine repository slug; provide --repo explicitly")
    return slug


def pr_details(repo: Optional[str], number: int) -> Dict:
    args = ["pr", "view", str(number), "--json", "baseRefName,headRefOid"]
    if repo:
        args.extend(["--repo", repo])
    cp = run_gh(args)
    return json.loads(cp.stdout)


def collect_check_run_names(repo: str, sha: str) -> Set[str]:
    endpoint = f"repos/{repo}/commits/{sha}/check-runs"
    args = ["api", endpoint, "--paginate", "--jq", ".check_runs[].name"]
    cp = run_gh(args)
    names = {line.strip() for line in cp.stdout.splitlines() if line.strip()}
    return names


def collect_status_contexts(repo: str, sha: str) -> Set[str]:
    endpoint = f"repos/{repo}/commits/{sha}/status"
    args = ["api", endpoint, "--jq", ".statuses[].context"]
    cp = run_gh(args)
    contexts = {line.strip() for line in cp.stdout.splitlines() if line.strip()}
    return contexts


def collect_required_contexts(repo: str, sha: str) -> List[str]:
    contexts: Set[str] = set()
    contexts.update(collect_check_run_names(repo, sha))
    contexts.update(collect_status_contexts(repo, sha))
    return sorted(contexts)


def normalize_subject(value) -> Optional[str]:
    if isinstance(value, str):
        return value
    if isinstance(value, dict):
        return value.get("slug") or value.get("login") or value.get("name")
    return None


def extract_allowances(data: Optional[Dict]) -> Optional[Dict[str, List[str]]]:
    if not data:
        return None

    result: Dict[str, List[str]] = {}
    for key in ("users", "teams", "apps"):
        entries = [normalize_subject(v) for v in data.get(key, [])]
        entries = [v for v in entries if v]
        if entries:
            result[key] = entries

    return result or None


def build_protection_payload(current: Dict, contexts: List[str]) -> Dict:
    payload: Dict = {}

    status = current.get("required_status_checks") or {}
    payload["required_status_checks"] = {
        "strict": status.get("strict", True),
        "contexts": contexts,
    }

    payload["enforce_admins"] = (current.get("enforce_admins") or {}).get("enabled", False)

    reviews = current.get("required_pull_request_reviews")
    if reviews:
        allowed_keys = {
            "dismiss_stale_reviews",
            "require_code_owner_reviews",
            "required_approving_review_count",
            "require_last_push_approval",
        }
        filtered = {k: reviews[k] for k in allowed_keys if k in reviews}
        allowances = extract_allowances(reviews.get("bypass_pull_request_allowances"))
        if allowances is not None:
            filtered["bypass_pull_request_allowances"] = allowances
        payload["required_pull_request_reviews"] = filtered
    else:
        payload["required_pull_request_reviews"] = None

    restrictions = current.get("restrictions")
    if restrictions:
        payload["restrictions"] = {
            "users": [s for s in map(normalize_subject, restrictions.get("users", [])) if s],
            "teams": [s for s in map(normalize_subject, restrictions.get("teams", [])) if s],
            "apps": [s for s in map(normalize_subject, restrictions.get("apps", [])) if s],
        }
    else:
        payload["restrictions"] = None

    toggle_keys = [
        "required_linear_history",
        "allow_force_pushes",
        "allow_deletions",
        "block_creations",
        "required_conversation_resolution",
        "lock_branch",
        "allow_fork_syncing",
    ]
    for key in toggle_keys:
        value = current.get(key)
        if isinstance(value, dict):
            payload[key] = value.get("enabled", False)
        elif isinstance(value, bool):
            payload[key] = value

    return payload


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Replace branch required status checks with those observed on a PR",
    )
    parser.add_argument("--pr", type=int, required=True, help="Pull request number")
    parser.add_argument(
        "--repo",
        help="Repository in owner/name form (defaults to the current directory's repo)",
    )
    parser.add_argument(
        "--branch",
        help="Branch to update (defaults to the PR's base branch)",
    )
    parser.add_argument(
        "--apply",
        action="store_true",
        help="Persist the changes (omit for a dry run)",
    )

    args = parser.parse_args()

    repo = args.repo or detect_repo_slug()
    details = pr_details(repo, args.pr)
    base_branch = args.branch or details["baseRefName"]
    head_sha = details["headRefOid"]

    print(f"PR #{args.pr} ({repo}) head SHA: {head_sha}")
    print(f"Base branch: {base_branch}")

    protection = gh_json(["api", f"repos/{repo}/branches/{base_branch}/protection"])
    current_required = protection.get("required_status_checks", {}).get("contexts", [])
    if current_required:
        print("\nCurrently required status checks:")
        for ctx in current_required:
            print(f"  - {ctx}")
    else:
        print("\nCurrently required status checks: (none)")

    contexts = collect_required_contexts(repo, head_sha)
    if not contexts:
        sys.stderr.write("No status checks found on the latest commit; aborting.\n")
        sys.exit(1)

    print("\nStatus checks detected:")
    for ctx in contexts:
        print(f"  - {ctx}")

    payload = build_protection_payload(protection, contexts)

    if args.apply:
        payload_json = json.dumps(payload)
        run_gh(
            [
                "api",
                f"repos/{repo}/branches/{base_branch}/protection",
                "--method",
                "PUT",
                "--input",
                "-",
            ],
            input_text=payload_json,
        )
        print("\nBranch protection updated.")
    else:
        print("\nDry run (no changes applied). JSON payload:")
        print(json.dumps(payload, indent=2))


if __name__ == "__main__":
    main()

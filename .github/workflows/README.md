## Fast and daily CI

Two workflows separate required checks from delayed coverage:

- **`ci.yaml`:** runs on pull requests, merge queue `checks_requested` events,
  and pushes to `main`. It contains core/macros/membership tests,
  integration and feature tests, store tests, Tokio tests, simulated-runtime
  tests, both deterministic simulation suites, and lint/documentation/security
  checks. A lightweight decision job allows merge queue runs to reuse verified
  PR results for an identical tree. PR and main-push runs always run the tests.
  `workflow_call` also lets the full workflow reuse these definitions.
- **`ci-reuse.yaml`:** a local reusable workflow that makes the reuse decision.
  GitHub records its immutable commit SHA in the caller's `referenced_workflows`
  metadata, providing the actual synthetic merge commit used by PR CI.
- **`ci-full.yaml`:** runs daily at 01:40 UTC. It calls `ci.yaml` from the same
  commit and adds all examples on stable and nightly (including demo scripts),
  release and benchmark builds, Monoio/Compio runtimes, WASM, metrics integration,
  and the Jepsen app check. Only this workflow has the history-check plan.
- **Manual (`workflow_dispatch` on `ci-full.yaml`):** always run the full suite
  on the selected branch, even if that commit has already passed. Use the
  Actions UI or `gh workflow run ci-full.yaml --ref <branch>`.

The daily run uses the latest default-branch commit (`main`), not every commit
merged that day. It skips expensive jobs if a scheduled or manual full run on
that exact branch and SHA has already succeeded. A successful fast run does not
count. Commits without a successful full run are retried the next day after a
failure or cancellation. Manual dispatch bypasses the history check. If the
history API fails, the plan fails visibly rather than silently skipping tests.
The workflows use distinct literal concurrency prefixes, because a reusable
workflow inherits the caller's `github.workflow` value. The fast workflow also
includes the event in its group, separating nested checks from main-push checks.
Thus full runs cannot deadlock on their own nested checks or have those checks
replaced by a pending main-push run. Detection is normally within one schedule
interval plus CI runtime; GitHub schedules may be delayed.

Routine CI runs 21 test/check jobs plus one lightweight reuse decision job.
Full runs retain all 65 existing jobs plus the daily plan and reuse decision.
Other workflows are unchanged:
`jepsen.yml` still runs its full scenarios on pushes to `main`, and checks
Jepsen-specific PR changes before merging.

### Required checks

The merge queue tests a temporary merge-group commit, not the PR head commit.
The `merge_group` trigger reports the same fast checks on that commit so the
queue can receive its required statuses. Passing PR checks alone does not
satisfy the queue's checks.

On a merge queue event, the decision job considers the 30 latest successful PR
runs of `ci.yaml`, accepting only results updated within the last 24 hours. It
rechecks the latest attempt's success, resolves the tested merge commit from
GitHub's immutable reusable-workflow metadata, and compares its entire Git tree
with the queue commit. The tested merge commit must also have that run's PR
head as its second parent. The caller, reusable workflow, and decision script
at the PR head must match those in the queue tree, preventing a modified caller
from presenting an unrelated workflow reference as evidence.

When the evidence matches, each existing required job (including every matrix
variant) runs a small guard, logs the source run URL, and skips checkout,
toolchain setup, and expensive checks. The jobs themselves are not skipped:
GitHub still receives all required check names on the queue SHA. A failed
planner or an invalid decision fails those jobs rather than skipping them.
Different trees, missing or ambiguous metadata, old results, and API errors
fall back to normal testing. Runs from before this policy was introduced lack
the provenance and cannot be reused. Main pushes and daily/manual full runs
always retest, including dependencies and security advisories that can change
without a source change.

Keep only the fast suite's checks required for merging. The daily-only jobs
(`test-release-build` / `Build`, `openraft-test-bench`, `bench-minimal`,
`rt-monoio`, `rt-compio`, `wasm`, `metrics-otel`, `examples`, and `jepsen-app` /
`jepsen app`, including matrix variants) must not be required by branch
protection or rulesets. They do not run on PRs and are not pre-merge coverage.
Neither `Full CI plan`, the former `CI plan`, nor the new `CI reuse decision`
should be required: the existing fast jobs enforce the reuse decision and
fail closed if it fails. The fast job names are unchanged for PRs; full-run
checks called through `fast` have a caller prefix and should not be required.
Repository settings are managed separately; this workflow change does not
update them.

Run `make ci-check` to test triggers, tree reuse, fail-closed guards, and daily
history gating locally.
It uses Node.js's built-in test runner, with no package installation. The lint
job runs it too. GitHub's normal workflow failure notifications apply to daily
failures; configure Actions notifications for the maintainers responsible for
triage.

## Action and App used for CI

- **slash-commands**: Trigger some command when a slash-commond, e.g. `/help` presents in a issue comment:
  https://github.com/marketplace/actions/slash-commands

- **github-script**: Call github API in action:
  https://github.com/actions/github-script

- **peter-evans/create-or-update-comment**:
  https://github.com/marketplace/actions/create-or-update-comment

- **actions-ecosystem/action-add-assignees**: add assignee:
  https://github.com/marketplace/actions/actions-ecosystem-action-add-assignees

## Action hints

### Trigger command when `/xxx` is commented:

```yaml
    steps:
      - name: Check for Command
        id: command
        uses: xt0rted/slash-command-action@v1
        with:
          repo-token: ${{ secrets.GITHUB_TOKEN }}
          command: test
          reaction: "true"
          reaction-type: "eyes"
          allow-edits: "false"
          permission-level: admin
      - name: Act on the command
        run: echo "The command was '${{ steps.command.outputs.command-name }}' with arguments '${{ steps.command.outputs.command-arguments }}'"
```

### Assign an issue to an assignee:

With `github-script`:

```
- uses: actions/github-script@v5
  with:
    script: |
      github.rest.issues.addAssignees({
        issue_number: context.issue.number,
        owner: context.repo.owner,
        repo: context.repo.repo,
        assignees: [context.payload.sender.login]
      })
```

With

### Create Comment:

With `github-script`:

```yaml
- uses: actions/github-script@v5
  with:
    script: |
      github.rest.issues.createComment({
        issue_number: context.issue.number,
        owner: context.repo.owner,
        repo: context.repo.repo,
        body: [
                'Get help or engage by:',
                '',
                '- `/help` : to print help messages.',
                '- `/assignme` : to assign this issue to you.',
              ].join('\n')
      })
```

### Trigger command when PR/issue is created


```yaml
jobs:
  pr_commented:
    # This job only runs for pull request comments
    name: PR comment
    if: ${{ github.event.issue.pull_request }}
    runs-on: ubuntu-latest
    steps:
      - run: |
          echo "Comment on PR #${{ github.event.issue.number }}"

  issue_commented:
    # This job only runs for issue comments
    name: Issue comment
    if: ${{ !github.event.issue.pull_request }}
    runs-on: ubuntu-latest
    steps:
      - run: |
          echo "Comment on issue #${{ github.event.issue.number }}"
```

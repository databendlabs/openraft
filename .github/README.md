## Action and App used for CI

- **Mergify**: automate PR workflow:
  https://docs.mergify.com/conditions/#about-status-checks

- **slash-commands**: Trigger some command when a slash-commond, e.g. `/help` presents in a issue comment:
  https://github.com/marketplace/actions/slash-commands

- **github-script**: Call github API in action:
  https://github.com/actions/github-script

- **peter-evans/create-or-update-comment**:
  https://github.com/marketplace/actions/create-or-update-comment

- **actions-ecosystem/action-add-assignees**: add assignee:
  https://github.com/marketplace/actions/actions-ecosystem-action-add-assignees

## Action hints

### Inform a user when there is a conflict with merge base:

```yaml
# if there is a conflict in a approved PR, ping the author.
- name: ping author on conflicts
  conditions:
    - conflict
    - "#approved-reviews-by >= 2"
  actions:
    comment:
      message: |
        This pull request has merge conflicts that must be resolved before it can be merged. @{{author}} please update it 🙏.

        Try `@mergify update` or update manually.
```

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

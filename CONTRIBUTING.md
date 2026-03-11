# CONTRIBUTING

This is a Rust project, so [rustup](https://rustup.rs/) is the best place to start.

## The guide

The guide for this project is built using [mdBook](https://rust-lang-nursery.github.io/mdBook/index.html).
Review their guide for more details on how to work with mdBook. Here are a few of the pertinents:

```
# Install the CLI.
cargo install mdbook

# Build the guide.
mdbook build

# Watch the FS for changes & rebuild.
mdbook watch
```

## Working with git

- **Write the commit message like writing an email to your friends**. There is a great [git commit format guide](https://cbea.ms/git-commit/).

- Do **rebase** and **squash** the branch onto the latest `main` branch before publishing a PR.

- Do **NOT** **rebase** after publishing PR. Only merge.


## Issue triage

When reviewing issues, sort them from high to low priority:

- **P0 - critical**: correctness, safety, data loss, security, panic/hang on a supported path, or anything blocking a release. Track it with an assignee and target branch or release immediately.

- **P1 - high**: a common workflow is broken or blocked and there is no practical workaround. Track it with an assignee and a target release.

- **P2 - normal**: a limited-scope bug, feature gap, or documentation issue with a reasonable workaround. Track it in the backlog or a milestone once it is scheduled.

- **P3 - low**: cleanup, ergonomics, research, or other nice-to-have work. Track it in the backlog until there is bandwidth.

Use these criteria when assigning and tracking issue priority:

- impact on correctness, safety, data loss, or security;
- how many users or workflows are affected;
- whether a workaround exists;
- whether the issue blocks a release, upgrade, or supported branch;
- which version or branch is affected; and
- who owns it and which milestone or release it targets once scheduled.


## Release checklist

- `make`: pass the unit test, format and clippy check.

- Any documentation updates should also be reflected in the guide.

- Ensure the Cargo.toml version for openraft or memstore has been updated, depending on which is being released.

- Once the release CI has been finished, navigate to the release page, update the release info and publish the release.

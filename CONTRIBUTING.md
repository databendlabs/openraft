# Contributing to OpenRaft

## Validate your changes

Run `make basic_check` before opening a pull request. It runs the project's formatting, lint, test, and documentation checks.

## Code style

Beyond `rustfmt`, OpenRaft follows the conventions in [`CLAUDE.md`](CLAUDE.md). The essentials are:

- Trait bounds use `where` clauses, never inline bounds: `fn f<T>() where T: RaftLeaderId`, not `fn f<T: RaftLeaderId>()`. This applies to functions, structs, enums, `impl` blocks, and traits.
- Each file has one main trait or type and its implementations. Do not reorganize existing files unless the change requires it.
- Public API changes include a `#[since(version = "...", change = "...")]` attribute that describes the change.

## Documentation

Public documentation lives in Rustdoc comments under `openraft/src/docs/` and is published on [docs.rs](https://docs.rs/openraft/0.10.0-alpha.30/openraft/docs/index.html). Update it when a public API or user-facing behavior changes.

## Working with git

- Write commit messages as concise notes to collaborators. The [git commit format guide](https://cbea.ms/git-commit/) is a useful reference.
- Before opening a pull request, rebase and squash your branch onto the latest `main` into one or two logical commits.
- Do not rebase a published pull request. Merge `main` into it instead.

## AI-assisted contributions

AI tools can help draft a change, but you own every submitted line.

- Understand each change well enough to defend it in review.
- Simplify generated output. Remove redundant wrappers, unnecessary abstractions, and comments that repeat the code or claim behavior it does not have.
- Match the surrounding code and this guide.

PRs that read as unreviewed AI output will be sent back for a cleanup pass before review.

## Release checklist

- `make basic_check` passes.
- The `[workspace.package]` version in the root `Cargo.toml` and every sibling `path` dependency that pins it are updated. Use a prior `chore: bump version` commit as the complete reference.
- After release CI finishes, open the release page, update the release notes, and publish.

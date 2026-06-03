# Contributing to OpenRaft

## Code style

Beyond `rustfmt`, OpenRaft follows a few conventions. The authoritative list lives in [`CLAUDE.md`](CLAUDE.md); the essentials:

- **Trait bounds** always use `where` clauses, never inline bounds — `fn f<T>() where T: RaftLeaderId`, not `fn f<T: RaftLeaderId>()`. This applies to functions, structs, enums, impls, and traits.

- **One main type per file.** Each file holds a single main trait or type with its `impl`s. Don't reorganize existing files unless asked.

- **Public API changes** carry a `#[since(version = "...", change = "...")]` attribute describing the change.

## Documentation

The docs are rustdoc comments under `openraft/src/docs/`, published at [docs.rs/openraft](https://docs.rs/openraft/latest/openraft/docs/index.html).

## Working with git

- **Write the commit message like an email to your friends.** This [git commit format guide](https://cbea.ms/git-commit/) is a great reference.

- **Rebase and squash** your branch onto the latest `main`, into one or two logical commits, before publishing a PR.

- Do **not** rebase a PR after publishing it; merge `main` in instead.

## AI-assisted contributions

AI tools are welcome as a drafting aid, but **you own every line you submit**.

- **Understand** each change well enough to defend it in review, as if you had written it by hand.

- **Simplify** generated output: drop redundant wrappers, needless abstractions, and comments that restate the code or describe behavior it does not have.

- **Match** the surrounding style and the rest of this guide.

PRs that read as unreviewed AI output will be sent back for a cleanup pass before review.

## Release checklist

- `make basic_check` passes: unit tests, format, and clippy.

- The workspace version is bumped: the `[workspace.package]` version in the root `Cargo.toml` plus every sibling `path` dependency that pins it. See a previous `chore: bump version` commit for the full set.

- After the release CI finishes, open the release page, update the release notes, and publish.

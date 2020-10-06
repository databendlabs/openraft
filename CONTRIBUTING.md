CONTRIBUTING
============
This is a Rust project, so [rustup](https://rustup.rs/) is the best place to start.

### the guide
The guide for this project is built using [mdBook](https://rust-lang-nursery.github.io/mdBook/index.html). Review their guide for more details on how to work with mdBook. Here are a few of the pertinents:

```
# Install the CLI.
cargo install mdbook

# Build the guide.
mdbook build

# Watch the FS for changes & rebuild.
mdbook watch
```

### release checklist
- Any documentation updates should also be reflected in the guide.
- Ensure the changelog is up-to-date.
- Ensure the Cargo.toml version for async-raft or memstore has been updated, depending on which is being released.
- Once the repo is in the desired state, push a tag matching the following pattern: `(async-raft|memstore)-v.+`.
- Once the release CI has been finished, navigate to the release page, update the release info and publish the release.

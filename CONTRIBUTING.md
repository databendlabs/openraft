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

## Release checklist

- Any documentation updates should also be reflected in the guide.
- Ensure the Cargo.toml version for openraft or memstore has been updated, depending on which is being released.
- Once the release CI has been finished, navigate to the release page, update the release info and publish the release.

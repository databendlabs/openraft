CONTRIBUTING
============
This is a Rust project, so [rustup](https://rustup.rs/) is the best place to start.

Check out the `.travis.yml` file to get an idea on how to run tests and the like.

### clippy
Haven't added clippy integration yet, but I am definitely planning on doing so. Don't run rustfmt ...

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

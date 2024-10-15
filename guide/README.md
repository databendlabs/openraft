

## Backup of mdbook building

`mdbook` based document is disabled and is moved to https://docs.rs/openraft/0.8.3/openraft/docs ;

To build mdbook in github workflow, copy the following snippet:


```
on:
  push:
    branches:
      - main
    paths:
      - 'guide/**'
      - 'book.toml'
      - 'README.md'

jobs:
  deploy-guide:
    runs-on: ubuntu-18.04
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Install mdbook
        uses: drmingdrmer/mdbook-full@main

      - name: Build mdbook
        run: mdbook build

      - name: Deploy to github page
        uses: peaceiris/actions-gh-pages@v3
        with:
          github_token: ${{ secrets.GITHUB_TOKEN }}
          publish_dir: ./guide/book
```

Github action `drmingdrmer/mdbook-full` is maintained in https://github.com/drmingdrmer/mdbook-full
with several plugins enabled:

Preprocessors:

- [mdbook-svgbob](https://github.com/drmingdrmer/mdbook-svgbob): SvgBob mdbook preprocessor which swaps code-blocks with neat SVG.
- [mdbook-katex](https://github.com/drmingdrmer/mdbook-katex): A preprocessor for mdBook, rendering LaTex equations to HTML at build time.

Backends:

- [mdbook-linkcheck](https://github.com/drmingdrmer/mdbook-linkcheck): A backend for `mdbook` which will check your links for you.



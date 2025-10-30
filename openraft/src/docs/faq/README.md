# FAQ Structure

Individual markdown files organized by numbered category directories.

## Structure

```
NN-category-name/
├── README.md          # Section name (e.g., "Getting Started")
└── NN-*.md           # FAQ entries
```

Categories auto-discovered by `build_faq.py` (sorted by directory name).

## Build

```bash
make
```

Generates `faq.md` and `faq-toc.md` from individual files.

## Add FAQ Entry

1. Create `NN-my-faq.md` in appropriate category directory
2. Write FAQ with `###` header
3. Add link definitions at bottom:
   ```markdown
   ### My FAQ Title

   Content with [`SomeType`][] references.

   [`SomeType`]: `crate::path::SomeType`
   ```
4. Run `make`

## Add Category

1. Create `NN-category-name/` directory
2. Add `README.md` with section name
3. Add FAQ markdown files
4. Run `make`

## Notes

- Each FAQ file is self-contained with its own link definitions
- Never edit `faq.md` or `faq-toc.md` directly
- Number prefix controls ordering

### `build_change_log.py`

Build change log since a previous git-tag

- Usage: `./scripts/build_change_log.py`: build change log since last tag.
- Usage: `./scripts/build_change_log.py <commit>`: build since specified commit.
- Install: `pip install -r requirements.txt`.

This script builds change log from git commit messages, outputs a versioned
change log to `./change-log/<version>.md` and concatenates all versioned change log
into `./change-log.md`.

If `./change-log/<version>.md` exists, it skips re-building it from git-log.
Thus you can generate semi-automatic change-logs by editing `./change-log/<version>.md` and running the script again, which will just only the `./change-log.md`.


### `check.kdl`

Run daily tests in parallel:

- Usage: `zellij --layout ./scripts/check.kdl`.
- Install: `cargo install zellij`.

It opens 3 panes to run:
- `cargo test --libs`: run only lib tests
- `cargo test --test *`: run only integration tests.
- `cargo clippy`: lint


### `contributors/`

Generate the contributor avatars shown in the top-level `README.md`.

- Usage: `make -C scripts/contributors` — update the login list, Markdown, and SVGs (reusing cached avatars).
- Usage: `make -C scripts/contributors refresh` — rebuild the SVGs, re-downloading every avatar.
- Install: standard library only; requires `gh` (authenticated) and `curl`.

Two scripts drive the `Makefile`:

- `list_contributors.py` unions all commit authors and issue/PR authors from
  GitHub, writing `assets/contributors/contributors.txt` (login list) and
  `assets/contributors/contributors.md` (the README-ready Markdown block).
- `generate_contributor_svgs.py` renders `assets/contributors/svg/<login>.svg`
  for each login — a circular avatar plus the `@handle` in GitHub's username
  font, with the avatar embedded so it renders inline. Downloaded avatars are
  cached under `assets/contributors/avatars/` so refreshes don't re-download.

Paste the contents of `assets/contributors/contributors.md` into the
`## Contributors` section of the top-level `README.md`.

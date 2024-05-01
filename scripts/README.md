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

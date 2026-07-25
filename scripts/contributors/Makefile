# Update the contributor resources under assets/contributors/:
#   contributors.txt  — login list (all commit + issue/PR authors)
#   contributors.md   — README-ready Markdown block (paste under ## Contributors)
#   svg/<login>.svg   — circular-avatar images referenced by the top-level README
#   avatars/          — cached raw avatars, so refreshes don't re-download
#
# Usage (from anywhere in the repo):
#   make -C scripts/contributors           # full update, reusing cached avatars
#   make -C scripts/contributors refresh   # rebuild SVGs, re-downloading avatars
#
# The scripts resolve their paths relative to the repo root, so every recipe
# runs from there. Requires: gh (authenticated) and curl.

REPO_ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))../..)

.PHONY: all list svgs refresh
.NOTPARALLEL:

# Full update: refresh the login list + Markdown, then rebuild the SVGs.
all: list svgs

# Query GitHub → contributors.txt + contributors.md.
list:
	cd "$(REPO_ROOT)" && python3 scripts/contributors/list_contributors.py

# Rebuild the avatar SVGs from contributors.txt, reusing cached avatars.
svgs:
	cd "$(REPO_ROOT)" && python3 scripts/contributors/generate_contributor_svgs.py

# Rebuild the SVGs, re-downloading every avatar (bypass the cache).
refresh:
	cd "$(REPO_ROOT)" && python3 scripts/contributors/generate_contributor_svgs.py --refresh

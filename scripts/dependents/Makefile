# Update the dependent lists under assets/dependents/:
#   all.txt       — every repository GitHub lists as depending on openraft
#   star-100.txt  — the subset with at least 100 stars
#
# Both files hold one `stars<TAB>owner/repo<TAB>description` row per repository,
# sorted by descending star count.
#
# Usage (from anywhere in the repo):
#   make -C scripts/dependents                      # crawl GitHub, rebuild both files
#   make -C scripts/dependents stars                # re-select from all.txt, no crawl
#   make -C scripts/dependents stars MIN_STARS=500  # ... into star-500.txt
#
# The crawl walks the HTML pages under
# https://github.com/databendlabs/openraft/network/dependents, one page per
# request, and takes about a minute. Requires: gh (authenticated).

REPO_ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))../..)
ASSET_DIR := assets/dependents
MIN_STARS := 100

.PHONY: all list stars
.NOTPARALLEL:

# Full update: crawl the dependents, then select the popular ones.
all: list stars

# Crawl GitHub → all.txt. The crawl writes a temporary file first, so a failed
# run leaves the previous list in place instead of a truncated one.
list:
	cd "$(REPO_ROOT)" && mkdir -p "$(ASSET_DIR)" \
		&& python3 scripts/dependents/list_dependents.py > "$(ASSET_DIR)/all.txt.tmp" \
		&& mv "$(ASSET_DIR)/all.txt.tmp" "$(ASSET_DIR)/all.txt"

# Select the repositories with at least MIN_STARS stars from all.txt.
stars:
	cd "$(REPO_ROOT)" \
		&& awk -F'\t' '$$1 >= $(MIN_STARS)' "$(ASSET_DIR)/all.txt" > "$(ASSET_DIR)/star-$(MIN_STARS).txt"

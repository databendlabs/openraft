#!/usr/bin/env bash
# Measure a change with idle-groups: build the benchmark without the change and with it, in one
# checkout, then run each case of PLAN with the two builds in turns.
#
# Usage: PLAN=CASES [BASE_REV=REV] [OUT_DIR=DIR] measure-change.sh WORKTREE LABEL FILE...
#
#   WORKTREE  The root of the checkout that holds the change.
#   LABEL     Names the two binaries and the results file.
#   FILE...   The files of the change, relative to WORKTREE. For the build without the change,
#             each is set to its BASE_REV version, or removed if BASE_REV has no such file, and
#             then restored.
#   PLAN      One case per line: "RUNS GROUPS FOLLOWERS MODE [EXTRA ARGS]". Each case runs RUNS
#             times with each build. Blank lines and lines that start with # are skipped.
#   BASE_REV  The revision to compare with, HEAD by default.
#   OUT_DIR   Where to keep the binaries and the results, by default
#             WORKTREE/benchmarks/idle-groups/target/measure-change.
#
# Writes OUT_DIR/results/LABEL-<time>.tsv, one line per run, and keeps both binaries in
# OUT_DIR/bin. Do not edit the files of the change while it runs.

set -uo pipefail

usage() {
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0" >&2
    exit 2
}

[ $# -ge 3 ] || usage
: "${PLAN:?set PLAN to the cases to run, one per line: RUNS GROUPS FOLLOWERS MODE [EXTRA ARGS]}"

wt=$1 label=$2
shift 2
files=("$@")
base_rev=${BASE_REV:-HEAD}

# Resolve OUT_DIR before leaving the caller's directory.
out_dir=
if [ -n "${OUT_DIR:-}" ]; then
    mkdir -p "$OUT_DIR" && out_dir=$(cd "$OUT_DIR" && pwd) || exit 1
fi
cd "$wt" || exit 1
out_dir=${out_dir:-$PWD/benchmarks/idle-groups/target/measure-change}
if ! git rev-parse -q --verify "$base_rev^{commit}" > /dev/null; then
    echo "not a revision: $base_rev" >&2
    exit 1
fi
mkdir -p "$out_dir/bin" "$out_dir/results" || exit 1
out="$out_dir/results/$label-$(date +%Y%m%dT%H%M%S).tsv"
target_dir=${CARGO_TARGET_DIR:-benchmarks/idle-groups/target}
saved=$(mktemp -d) || exit 1

# Copy each file of the change into $saved, or record that the change removed it.
save() {
    local f
    for f in "${files[@]}"; do
        if [ -e "$f" ]; then
            mkdir -p "$saved/files/$(dirname "$f")" && cp "$f" "$saved/files/$f" || return 1
        else
            mkdir -p "$saved/absent/$(dirname "$f")" && touch "$saved/absent/$f" || return 1
        fi
    done
}

# Set each file of the change to its version at $base_rev, or remove it if $base_rev has none.
set_to_base() {
    local f
    for f in "${files[@]}"; do
        if git cat-file -e "$base_rev:$f" 2> /dev/null; then
            mkdir -p "$(dirname "$f")" && git show "$base_rev:$f" > "$f" || return 1
        else
            rm -f "$f"
        fi
    done
}

# Put the change back. `cp` gives each file a new mtime, which makes cargo rebuild it.
restore() {
    local f
    for f in "${files[@]}"; do
        if [ -e "$saved/files/$f" ]; then
            cp "$saved/files/$f" "$f"
        elif [ -e "$saved/absent/$f" ]; then
            rm -f "$f"
        fi
    done
}

# Succeed if every file of the change is as `save` found it.
restored() {
    local f
    for f in "${files[@]}"; do
        if [ -e "$saved/files/$f" ]; then
            cmp -s "$f" "$saved/files/$f" || return 1
        elif [ -e "$f" ]; then
            return 1
        fi
    done
}

on_exit() {
    restore
    restored || echo "restore failed: the files of the change are saved in $saved" >&2
}

# Build the benchmark and keep the binary as idle-groups-LABEL-$1.
build() {
    cargo build -q --release --manifest-path benchmarks/idle-groups/Cargo.toml || return 1
    cp "$target_dir/release/idle-groups" "$out_dir/bin/idle-groups-$label-$1"
}

echo "start: $(date -u +%H:%M:%S)"
if ! save; then
    echo "cannot save the files of the change in $saved" >&2
    exit 1
fi
trap on_exit EXIT
trap 'exit 130' INT TERM

if ! set_to_base; then
    echo "cannot set the files of the change to $base_rev" >&2
    exit 1
fi
if ! build before; then
    echo "build failed: before" >&2
    exit 1
fi

restore
trap - EXIT INT TERM
if ! restored; then
    echo "restore failed: the files of the change are saved in $saved" >&2
    exit 1
fi
rm -r "$saved"

if ! build after; then
    echo "build failed: after" >&2
    exit 1
fi
echo "built both: $(date -u +%H:%M:%S)"

{
    echo "# $label: before = the files at $(git rev-parse --short "$base_rev"); after = $(git rev-parse --short HEAD) with the change"
    echo "# release builds of benchmarks/idle-groups; its README describes the fields"
} > "$out"

while read -r runs groups followers mode extra; do
    case "${runs:-}" in
        '' | '#'*) continue ;;
        *[!0-9]*)
            echo "RUNS must be a number: $runs" >&2
            exit 2
            ;;
    esac
    round=1
    while [ "$round" -le "$runs" ]; do
        for build in before after; do
            # EXTRA ARGS is split into words on purpose.
            # shellcheck disable=SC2086
            if ! line=$("$out_dir/bin/idle-groups-$label-$build" --groups "$groups" \
                --followers "$followers" --mode "$mode" $extra < /dev/null); then
                echo "run failed: '$runs $groups $followers $mode $extra', round $round, $build" >&2
                exit 1
            fi
            printf 'round=%s\tbuild=%s\targs=%s\t%s\n' "$round" "$build" "$extra" "$line" >> "$out"
        done
        round=$((round + 1))
    done
done <<< "$PLAN"
echo "wrote $out"
echo "done: $(date -u +%H:%M:%S)"

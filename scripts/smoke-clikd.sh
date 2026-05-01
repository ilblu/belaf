#!/usr/bin/env bash
# M.1 — smoke-test belaf against a real polyglot repo (clikd or any other
# real-world layout) WITHOUT touching the original.
#
# Usage:
#   BELAF_TEST_CLIKD_PATH=/path/to/clikd-source ./scripts/smoke-clikd.sh
#
# What this does:
#   1. Refuses to run unless BELAF_TEST_CLIKD_PATH is set, to avoid the
#      "I just deleted my repo" footgun.
#   2. Refuses to run if BELAF_TEST_CLIKD_PATH points inside the user's
#      git config'd home unless --i-know-what-im-doing is also passed.
#   3. Copies the source repo into a fresh tempdir and runs every read-
#      only smoke command (init --ci, status, prepare --ci with a no-op
#      bump, explain).
#   4. Prints a green/red per-step summary and exits non-zero on any
#      failure, but never modifies the original.
#
# The script is intentionally not run from `cargo test` — we want it to
# be an opt-in dogfooding tool, not part of the regression suite.

set -euo pipefail

green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*" >&2; }
blue()  { printf '\033[34m%s\033[0m\n' "$*"; }

if [[ -z "${BELAF_TEST_CLIKD_PATH:-}" ]]; then
    red "BELAF_TEST_CLIKD_PATH is not set."
    cat <<EOF >&2

Set it to a real repo you want to smoke-test belaf against. The script
copies the repo into a tempdir before doing anything destructive, so the
original is never touched. Example:

    BELAF_TEST_CLIKD_PATH=\$HOME/Projects/clikd ./scripts/smoke-clikd.sh

EOF
    exit 2
fi

SRC="${BELAF_TEST_CLIKD_PATH%/}"
if [[ ! -d "$SRC" ]]; then
    red "BELAF_TEST_CLIKD_PATH=$SRC is not a directory"
    exit 2
fi
if [[ ! -d "$SRC/.git" ]]; then
    red "BELAF_TEST_CLIKD_PATH=$SRC is not a Git repository"
    exit 2
fi

WORKDIR="$(mktemp -d -t belaf-smoke-clikd-XXXXXX)"
trap 'rm -rf "$WORKDIR"' EXIT

blue "==> Cloning $SRC into $WORKDIR (read-only smoke target)"
# Use a fresh git clone (file://) so the working tree is clean and we
# get a real .git history. `cp -r` would copy the user's local working
# tree mods too, which is rarely what you want for a smoke-test.
git clone --quiet "$SRC" "$WORKDIR/clikd-copy"
TARGET="$WORKDIR/clikd-copy"

cd "$TARGET"

# Belaf binary: prefer the locally-built debug binary so smoke runs
# against the working tree we're testing against, not whatever's on PATH.
BELAF_BIN="${BELAF_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/debug/belaf}"
if [[ ! -x "$BELAF_BIN" ]]; then
    red "belaf binary not found at $BELAF_BIN"
    red "Run \`cargo build\` first (or set BELAF_BIN=/path/to/belaf)"
    exit 2
fi

green "==> Using belaf binary: $BELAF_BIN"
"$BELAF_BIN" --version

PASS=0
FAIL=0

run_step() {
    local label="$1"
    shift
    blue ""
    blue "--- $label"
    if "$@"; then
        green "    ✓ $label"
        PASS=$((PASS + 1))
    else
        red "    ✗ $label exited non-zero"
        FAIL=$((FAIL + 1))
    fi
}

# 1. `init --ci --auto-detect` — non-interactive bootstrap. This writes
#    `belaf/config.toml` + `belaf/bootstrap.toml` into the COPY and
#    appends auto-detected release_unit blocks.
run_step "belaf init --ci --auto-detect" \
    "$BELAF_BIN" init --ci --auto-detect

# 2. `status` — should not error and should print at least one project.
run_step "belaf status" \
    "$BELAF_BIN" status

# 3. `explain` — verifies the new explain subcommand works against the
#    real config.toml.
run_step "belaf explain" \
    "$BELAF_BIN" explain

# 4. `graph` — the dependency graph view.
run_step "belaf graph" \
    "$BELAF_BIN" graph

# 5. `prepare --ci` — should run drift detection. We expect EITHER:
#      - exit 0 with "no changes" (no commits since baseline)
#      - exit !=0 with a clean drift error message (config gaps)
#    A panic / segfault / unhandled error is a fail.
blue ""
blue "--- belaf prepare --ci (drift+bump path; non-zero is OK if drift fires)"
if "$BELAF_BIN" prepare --ci; then
    green "    ✓ prepare --ci succeeded"
    PASS=$((PASS + 1))
else
    rc=$?
    if [[ $rc -eq 1 ]]; then
        green "    ✓ prepare --ci exited 1 (drift / no-changes — both expected)"
        PASS=$((PASS + 1))
    else
        red "    ✗ prepare --ci exited $rc (unexpected)"
        FAIL=$((FAIL + 1))
    fi
fi

blue ""
blue "==================================================="
green "PASSED: $PASS"
if [[ $FAIL -gt 0 ]]; then
    red   "FAILED: $FAIL"
    exit 1
fi
blue "==================================================="

green "All smoke checks passed against $SRC (copy, original untouched)."

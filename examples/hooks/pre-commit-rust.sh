#!/usr/bin/env bash
# pre-commit hook for Rust projects.
# Use in gd.yml:
#   hooks:
#     pre_commit: "bash examples/hooks/pre-commit-rust.sh"
#
# Aborts the gd auto-commit if:
#   - code is not formatted (cargo fmt --check)
#   - there are clippy warnings at the deny level

set -euo pipefail

echo "[gd pre-commit] checking format..."
if ! cargo fmt --check --quiet 2>&1; then
    echo "[gd pre-commit] FAIL: run 'cargo fmt' to fix formatting"
    exit 1
fi

echo "[gd pre-commit] running clippy..."
if ! cargo clippy --quiet --all-targets -- -D warnings 2>&1; then
    echo "[gd pre-commit] FAIL: fix clippy warnings before committing"
    exit 1
fi

echo "[gd pre-commit] OK"
exit 0

#!/usr/bin/env bash
# pre-commit hook for Node.js / TypeScript projects.
# Use in gd.yml:
#   hooks:
#     pre_commit: "bash examples/hooks/pre-commit-node.sh"

set -euo pipefail

echo "[gd pre-commit] running ESLint..."
if ! npx eslint --quiet . 2>&1; then
    echo "[gd pre-commit] FAIL: fix ESLint errors before committing"
    exit 1
fi

echo "[gd pre-commit] running TypeScript type check..."
if ! npx tsc --noEmit --pretty false 2>&1; then
    echo "[gd pre-commit] FAIL: fix TypeScript errors before committing"
    exit 1
fi

echo "[gd pre-commit] OK"
exit 0

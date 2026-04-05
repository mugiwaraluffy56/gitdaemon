# justfile — task runner for gitdaemon
# Install: cargo install just
# Usage:   just <recipe>

default:
    @just --list

# ── Build ──────────────────────────────────────────────────────────────────────

build:
    cargo build

release:
    cargo build --release

# ── Quality ────────────────────────────────────────────────────────────────────

check:
    cargo check --all-targets

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --all-targets --all-features -- -D warnings

lint: fmt-check clippy

# ── Testing ────────────────────────────────────────────────────────────────────

test:
    cargo test

test-verbose:
    cargo test -- --nocapture

test-integration:
    cargo test --test integration

bench:
    cargo bench

# ── Full CI gate ───────────────────────────────────────────────────────────────

ci: fmt-check clippy test
    @echo "CI gate passed"

# ── Installation ───────────────────────────────────────────────────────────────

install:
    cargo install --path . --locked

install-dev:
    cargo build && cp target/debug/gd ~/.local/bin/gd

# ── Documentation ──────────────────────────────────────────────────────────────

docs:
    cargo doc --no-deps --open

# ── Misc ───────────────────────────────────────────────────────────────────────

clean:
    cargo clean

deny:
    cargo deny check

outdated:
    cargo outdated -R

update:
    cargo update

# ── Release ────────────────────────────────────────────────────────────────────

tag version:
    git tag -s "v{{version}}" -m "Release v{{version}}"
    git push origin "v{{version}}"

# justfile — project task runner (https://github.com/casey/just)
# Usage: just <recipe>

default: check

# Fast type + borrow check (no linking)
check:
    cargo check --all-targets

# Build debug binary
build:
    cargo build

# Build optimised release binary
release:
    cargo build --release

# Run all tests
test:
    cargo test

# Run tests with output visible
test-verbose:
    cargo test -- --nocapture

# Run a single test by name pattern
test-one name:
    cargo test {{name}} -- --nocapture

# Lint with Clippy
lint:
    cargo clippy --all-targets -- -D warnings

# Format all source files
fmt:
    cargo fmt

# Check formatting without modifying files
fmt-check:
    cargo fmt -- --check

# Run benchmarks
bench:
    cargo bench

# Run security audit
audit:
    cargo audit

# Run cargo-deny (license + advisory checks)
deny:
    cargo deny check

# Full CI gate: format check + lint + test
ci: fmt-check lint test

# Install the release binary to ~/.local/bin/
install: release
    install -Dm755 target/release/gd ~/.local/bin/gd
    @echo "installed gd to ~/.local/bin/gd"

# Uninstall
uninstall:
    rm -f ~/.local/bin/gd

# Show binary size breakdown
size:
    cargo bloat --release --crates

# Watch for changes and re-run check
watch:
    cargo watch -x check

# Clean build artefacts
clean:
    cargo clean

# Print current gd version
version:
    cargo run --quiet -- --version

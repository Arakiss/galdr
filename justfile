# galdr task runner (just).
#
# Install: https://just.systems
#
# Recipes mirror the CI jobs in .github/workflows/ci.yml. Running `just check`
# locally should produce the same verdict as CI on a clean PR.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Default recipe — list what's available.
_default:
    @just --list --unsorted

# Run the same checks CI runs on every PR.
check: fmt clippy test deny
    @echo "--- local check: ok ---"

# Check formatting without modifying files.
fmt:
    cargo fmt --all --check

# Apply formatting fixes in place.
fmt-fix:
    cargo fmt --all

# Clippy with -D warnings, both feature configurations.
clippy:
    cargo clippy --all-targets -- -D warnings
    cargo clippy --all-targets --features mlx -- -D warnings

# Test suite (unit + integration), both feature configurations.
test:
    cargo test
    cargo test --features mlx

# Build both feature configurations (catches feature-gated compile errors).
build:
    cargo build
    cargo build --features mlx

# cargo-deny: advisories, bans, licenses, sources. Requires `cargo install cargo-deny`.
deny:
    cargo deny check advisories bans licenses sources

# Release-profile build.
release-build:
    cargo build --release

# Build the rendered docs, denying rustdoc warnings.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --features mlx

# Remove all build artefacts.
clean:
    cargo clean

# User-approved release cut for a merged release-please PR (tags + verifies)
release-cut:
    scripts/release-cut.sh

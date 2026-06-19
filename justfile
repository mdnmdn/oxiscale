# Oxiscale development commands — run `just --list` for all recipes.

default:
    @just check

# --- Build -------------------------------------------------------------------

# Type-check the full workspace (fast iteration).
check:
    cargo check --workspace

# Build all workspace members (debug).
build:
    cargo build --workspace

# Build release binaries.
build-release:
    cargo build --workspace --release

# Run the standalone control-server binary (skeleton).
run *ARGS:
    RUST_LOG={{env_var_or_default("RUST_LOG", "info")}} cargo run -p tailcontrold -- {{ARGS}}

# --- Test --------------------------------------------------------------------

# Run all workspace tests.
test:
    cargo test --workspace

# Run tests with output visible (useful while debugging).
test-verbose:
    cargo test --workspace -- --nocapture

# --- Format ------------------------------------------------------------------

# Format all Rust code.
fmt:
    cargo fmt --all

# Check formatting without writing (CI / pre-commit).
fmt-check:
    cargo fmt --all -- --check

# --- Lint --------------------------------------------------------------------

# Clippy with warnings denied.
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Format check + clippy — run before committing.
lint: fmt-check clippy

# Full quality gate: lint + tests. Run at the end of every task.
ci: lint test

# --- Documentation -----------------------------------------------------------

# Build rustdoc for the workspace.
doc:
    cargo doc --workspace --no-deps

# Open local rustdoc in the browser (macOS).
doc-open:
    cargo doc --workspace --no-deps --open

# --- References --------------------------------------------------------------

# Initialise git submodules (tailscale-rs).
submodule:
    git submodule update --init --recursive

# --- Cleanup -----------------------------------------------------------------

# Remove build artefacts.
clean:
    cargo clean
# Task runner. Install tools once with `just setup`.
# Every recipe is plain cargo underneath; read it if you need the raw command.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Show available recipes.
default:
    @just --list

# Install the cargo tools used by the recipes below.
setup:
    cargo install --locked cargo-nextest cargo-insta cargo-llvm-cov cargo-deny

# Everything CI runs. Definition of done for any change.
ci: fmt-check lint test snap-check cov deny

# Run all tests (nextest for unit/integration/fixtures, cargo test for doctests).
test *ARGS:
    cargo nextest run --workspace {{ARGS}}
    cargo test --workspace --doc

# Run tests, then interactively review new/changed snapshots.
snap:
    cargo insta test --workspace --review

# Fail if any snapshot is pending, changed or unreferenced (used by CI).
snap-check:
    cargo insta test --workspace --check --unreferenced=reject

# Line coverage of the core crate, with a floor.
cov:
    cargo llvm-cov nextest -p mydiff-core --fail-under-lines 85

# HTML coverage report at target/llvm-cov/html/index.html.
cov-html:
    cargo llvm-cov nextest --workspace --html

lint:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

deny:
    cargo deny check

# Usage: just new-case <mode>/<name> [ext]
#   text/<name> [txt]   binary/<name> [bin]   structured/<name> <json|yaml|csv|tsv>
#   folder/<name>       (creates left/ and right/ directories)

# Scaffold a golden fixture case (text, binary, structured or folder).
new-case path ext="":
    #!/usr/bin/env bash
    set -euo pipefail
    dir="crates/mydiff-core/tests/fixtures/{{path}}"
    mode="$(echo '{{path}}' | cut -d/ -f1)"
    if [ -e "$dir" ]; then echo "$dir already exists" >&2; exit 1; fi
    ext="{{ext}}"
    case "$mode" in
      text) ext="${ext:-txt}" ;;
      binary) ext="${ext:-bin}" ;;
      structured) [ -n "$ext" ] || { echo "structured cases need an extension: json, yaml, csv or tsv" >&2; exit 1; } ;;
      folder) ;;
      *) echo "unknown mode '$mode' (text, binary, structured, folder)" >&2; exit 1 ;;
    esac
    mkdir -p "$dir"
    if [ "$mode" = folder ]; then
      mkdir -p "$dir/left" "$dir/right"
      printf '# Required marker for folder cases; add DiffOptions keys as needed.\n' > "$dir/options.toml"
    else
      touch "$dir/left.$ext" "$dir/right.$ext"
      printf '# Optional DiffOptions for this case (same keys as a config [defaults] table).\n' > "$dir/options.toml"
    fi
    echo "Created $dir — fill in the inputs, then run: just snap"

# Run the app.
run *ARGS:
    cargo run -p mydiff -- {{ARGS}}

_default:
  @just --list --list-heading $'BDK Kyoto\n'

check:
  cargo fmt -- --check
  cargo clippy --all-targets -- -D warnings
  cargo check --all-features

# Run a test suite: unit, integration, features, msrv, min-versions.
test suite="unit":
  just _test-{{suite}}

# Unit test suite.
_test-unit:
  cargo test --lib
  cargo test --doc
  cargo test --examples

# Run integration tests
_test-integration: 
  cargo test --tests -- --test-threads 1 --nocapture 

# Test that minimum versions of dependency contraints are still valid.
_test-min-versions:
  just _delete-lockfile
  cargo +nightly check --all-features -Z direct-minimal-versions

# Check code with MSRV compiler.
_test-msrv:
  cargo install cargo-msrv@0.18.4
  cargo msrv verify --all-features

# Run the example: example.
example name="example":
  cargo run --example {{name}} --release

# Delete unused files or branches: data, lockfile, branches
delete item="data":
  just _delete-{{item}}

_delete-data:
  rm -rf light_client_data
  rm -rf data

_delete-lockfile:
  rm -f Cargo.lock

_delete-branches:
  git branch --merged | grep -v \* | xargs git branch -d

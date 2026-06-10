#!/bin/bash

run_format_checks() {
  local target_log=$1
  shift
  local cargo_opts=("$@")

  log "Running code formatting check for $target_log..."
  cargo fmt --all -- --check

  log "Running static analysis (clippy) for $target_log..."
  cargo clippy "${cargo_opts[@]}" --all-targets --all-features -- -D warnings
}


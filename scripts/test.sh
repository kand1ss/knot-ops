#!/bin/bash

run_tests() {
  local target_log=$1
  shift
  local cargo_opts=("$@")

  log "Running tests for $target_log..."
  cargo test "${cargo_opts[@]}" --all-features
}


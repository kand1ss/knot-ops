#!/bin/bash

run_tests() {
    local target_log=$1
    local cargo_opts=$2

    log "Running tests for $target_log..."
    cargo test $cargo_opts --all-features
}
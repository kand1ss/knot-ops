#!/bin/bash

source "$(dirname "$0")/common.sh"

log "Generating documentation..."
cargo doc --no-deps --open --document-private-items --all-features
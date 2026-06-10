#!/bin/bash

SCRIPT_DIR="$(dirname "$0")"
source "$SCRIPT_DIR/common.sh"
source "$SCRIPT_DIR/fmt.sh"
source "$SCRIPT_DIR/test.sh"

set -e

GREEN=$(tput setaf 2)
RED=$(tput setaf 1)
BLUE=$(tput setaf 4)
BOLD=$(tput bold)
NC=$(tput sgr0)

TARGET_DIR=$1
CARGO_OPTS=""

if [ -n "$TARGET_DIR" ]; then
    log_target="package [$TARGET_DIR]"
    CARGO_OPTS="-p $TARGET_DIR"
else
    log_target="the entire workspace"
    CARGO_OPTS="--workspace"
fi

log() {
    echo -e "${BLUE}${BOLD}==>${NC} ${BOLD}$1${NC}"
}

error_handler() {
    echo -e "\n${RED}${BOLD}[!] Check failed for $log_target. Please fix the errors above.${NC}"
}

trap error_handler ERR

run_format_checks "$log_target" "$CARGO_OPTS"
run_tests "$log_target" "$CARGO_OPTS"

echo -e "\n${GREEN}${BOLD}[✓] All checks for $log_target passed successfully!${NC}"
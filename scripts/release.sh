#!/usr/bin/env bash
set -euo pipefail

version="$1"

git add .
git commit -S -m "chore: release v${version}"
git tag -s "v${version}" -m "Release Knot v${version}"
git push origin HEAD
git push origin "v${version}"
#!/usr/bin/env bash
set -euo pipefail

VERSION_FILE="VERSION"
CHANGELOG_FILE="CHANGELOG.md"

if [ -z "${NEW_VERSION:-}" ]; then
  echo "Error: NEW_VERSION environment variable is not set" >&2
  exit 1
fi

if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "Error: NEW_VERSION must be semver (X.Y.Z or X.Y.Z-prerelease), got: $NEW_VERSION" >&2
  exit 1
fi

DATE=$(date +%Y-%m-%d)

if [[ "$OSTYPE" == "darwin"* ]]; then
  SED_INPLACE=(sed -i '')
else
  SED_INPLACE=(sed -i)
fi

# VERSION file: bootstrap vs update
if [ ! -f "$VERSION_FILE" ]; then
  echo "No $VERSION_FILE found — treating this as the first release."
  printf '%s\n' "$NEW_VERSION" > "$VERSION_FILE"
  echo "VERSION created: $NEW_VERSION"
else
  CURRENT_VERSION=$(tr -d '[:space:]' < "$VERSION_FILE")

  if [ -z "$CURRENT_VERSION" ]; then
    echo "$VERSION_FILE is empty — treating this as the first release."
    printf '%s\n' "$NEW_VERSION" > "$VERSION_FILE"
    echo "VERSION set: $NEW_VERSION"
  elif [ "$CURRENT_VERSION" == "$NEW_VERSION" ]; then
    echo "VERSION already at $NEW_VERSION, skipping version bump."
  else
    if printf '%s\n%s\n' "$NEW_VERSION" "$CURRENT_VERSION" | sort -C -V 2>/dev/null; then
      echo "Error: NEW_VERSION ($NEW_VERSION) is not greater than CURRENT_VERSION ($CURRENT_VERSION)" >&2
      exit 1
    fi
    printf '%s\n' "$NEW_VERSION" > "$VERSION_FILE"
    echo "VERSION updated: $CURRENT_VERSION -> $NEW_VERSION"
  fi
fi

# CHANGELOG.md: bootstrap vs update
echo "Preparing CHANGELOG.md for version $NEW_VERSION..."

if [ ! -f "$CHANGELOG_FILE" ]; then
  echo "No $CHANGELOG_FILE found — creating with initial structure."
  cat > "$CHANGELOG_FILE" <<EOF
# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [$NEW_VERSION] - $DATE
EOF
  echo "CHANGELOG.md created with version $NEW_VERSION."
elif ! grep -q "## \[Unreleased\]" "$CHANGELOG_FILE"; then
  echo "Error: $CHANGELOG_FILE exists but has no '## [Unreleased]' section" >&2
  echo "       Add one manually or delete the file to bootstrap fresh." >&2
  exit 1
elif ! grep -q "## \[$NEW_VERSION\]" "$CHANGELOG_FILE"; then
  "${SED_INPLACE[@]}" "s/## \[Unreleased\]/## [Unreleased]\n\n## [$NEW_VERSION] - $DATE/" "$CHANGELOG_FILE"
  echo "CHANGELOG.md successfully updated."
else
  echo "CHANGELOG.md already contains version $NEW_VERSION, skipping."
fi
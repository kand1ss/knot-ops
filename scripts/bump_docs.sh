#!/usr/bin/env bash

set -e

if [ -z "$NEW_VERSION" ]; then
  echo "Error: NEW_VERSION environment variable is not set"
  exit 1
fi

if [ -z "$PREV_VERSION" ]; then
  echo "Error: PREV_VERSION environment variable is not set"
  exit 1
fi

if [[ "$OSTYPE" == "darwin"* ]]; then
  SED_INPLACE="sed -i ''"
else
  SED_INPLACE="sed -i"
fi

DATE=$(date +%Y-%m-%d)

echo "Preparing documentation for version $NEW_VERSION..."

if ! grep -q "## \[$NEW_VERSION\]" CHANGELOG.md; then
  $SED_INPLACE "s/## \[Unreleased\]/## [Unreleased]\n\n## [$NEW_VERSION] - $DATE/" CHANGELOG.md
  echo "CHANGELOG.md successfully updated."
else
  echo "CHANGELOG.md already contains version $NEW_VERSION, skipping."
fi

if ! grep -q "knot = \"$NEW_VERSION\"" README.md; then
  $SED_INPLACE "s/knot = \"$PREV_VERSION\"/knot = \"$NEW_VERSION\"/g" README.md
  echo "README.md successfully updated."
else
  echo "README.md already contains version $NEW_VERSION, skipping."
fi

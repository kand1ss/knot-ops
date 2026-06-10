#!/usr/bin/env bash

set -e

DATE=$(date +%Y-%m-%d)

echo "Preparing documentation for version $NEW_VERSION..."

if ! grep -q "## \[$NEW_VERSION\]" CHANGELOG.md; then
  sed -i "s/## \[Unreleased\]/## [Unreleased]\n\n## [$NEW_VERSION] - $DATE/" CHANGELOG.md
  echo "CHANGELOG.md successfully updated."
else
  echo "CHANGELOG.md already contains version $NEW_VERSION, skipping."
fi

if ! grep -q "knot = \"$NEW_VERSION\"" README.md; then
  sed -i "s/knot = \"$PREV_VERSION\"/knot = \"$NEW_VERSION\"/g" README.md
  echo "README.md successfully updated."
else
  echo "README.md already contains version $NEW_VERSION, skipping."
fi

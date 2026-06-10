#!/usr/bin/env bash

set -e

echo "Starting the installation of Rust project tools..."

if ! command -v cargo &> /dev/null; then
    echo "Cargo not found. Please install Rust first: https://rustup.rs/"
    exit 1
fi

tools=("cargo-release" "cargo-insta" "cargo-update")

for tool in "${tools[@]}"; do
    echo "Installing $tool..."
    cargo install "$tool"
done

CARGO_BIN_PATH="$HOME/.cargo/bin"
if [[ ":$PATH:" != *":$CARGO_BIN_PATH:"* ]]; then
    echo "Warning: $CARGO_BIN_PATH is not in your PATH."
    echo "Please add it to your .bashrc or .zshrc:"
    echo 'export PATH="$HOME/.cargo/bin:$PATH"'
fi

echo "Installation complete!"
echo "You can now use:"
echo "  - cargo release (for releases)"
echo "  - cargo insta (for snapshot tests)"
echo "  - cargo install-update --all (to update your tools)"

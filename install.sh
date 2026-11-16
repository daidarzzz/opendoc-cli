#!/bin/sh
# Unattended OpenDoc CLI installer for Linux and macOS
# One-line installation:
#   curl -fsSL https://raw.githubusercontent.com/daidarzzz/opendoc-cli/master/install.sh | sh

set -eu

REPO="daidarzzz/opendoc-cli"
INSTALL_DIR="$HOME/.local/bin"

echo "Installing OpenDoc CLI (odc)..."

# 1. Detect the OS and architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
  linux)
    case "$ARCH" in
      x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
      *) echo "Error: unsupported Linux architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  darwin)
    case "$ARCH" in
      arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
      x86_64) TARGET="x86_64-apple-darwin" ;;
      *) echo "Error: unsupported macOS architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Error: unsupported operating system: $OS" >&2
    exit 1
    ;;
esac

# 2. Find the release archive
LATEST_JSON="$(curl -fsSL -H "User-Agent: opendoc-cli-installer" "https://api.github.com/repos/${REPO}/releases/latest")"
DOWNLOAD_URL="$(echo "$LATEST_JSON" | grep -o 'https://[^"]*' | grep "${TARGET}\.tar\.gz$" | head -n 1)"

if [ -z "$DOWNLOAD_URL" ]; then
  echo "Error: no archive for ${TARGET} in the latest ${REPO} release." >&2
  exit 1
fi

# 3. Download and unpack in a temporary directory
stage_dir="$(mktemp -d)"
trap 'rm -rf "$stage_dir"' EXIT

echo "Downloading binaries (${TARGET})..."
curl -fsSL "$DOWNLOAD_URL" -o "$stage_dir/release.tar.gz"
tar -xzf "$stage_dir/release.tar.gz" -C "$stage_dir"

mkdir -p "$INSTALL_DIR"

for bin in opendoc-cli odc; do
  if [ ! -f "$stage_dir/$bin" ]; then
    echo "Error: release archive is missing $bin." >&2
    exit 1
  fi
done

for bin in opendoc-cli odc; do
  install -m 0755 "$stage_dir/$bin" "$INSTALL_DIR/$bin"
done

if [ -f "$stage_dir/opendoc-cli.release" ]; then
  install -m 0644 "$stage_dir/opendoc-cli.release" "$INSTALL_DIR/opendoc-cli.release"
fi

echo ""
echo "Installed OpenDoc CLI in $INSTALL_DIR"

# 4. Check whether INSTALL_DIR is on PATH
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo ""
    echo "Note: $INSTALL_DIR is missing from your \$current PATH."
    echo "To use 'odc' and 'opendoc-cli', add this line to ~/.bashrc or ~/.zshrc:"
    echo '  export PATH="$HOME/.local/bin:$PATH"'
    ;;
esac

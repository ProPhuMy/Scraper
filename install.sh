#!/bin/sh
set -eu

REPO="ProPhuMy/scratch"
INSTALL_DIR="${SCRAPER_INSTALL_DIR:-$HOME/.local/bin}"

# 1. Detect OS
raw_os="${SCRAPER_UNAME_S:-$(uname -s)}"
case "$raw_os" in
  Linux)
    OS="linux"
    ;;
  Darwin)
    OS="macos"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    OS="windows"
    ;;
  *)
    echo "scraper-install: unsupported OS: $raw_os" >&2
    exit 1
    ;;
esac

# 2. Detect Architecture
raw_arch="${SCRAPER_UNAME_M:-$(uname -m)}"
case "$raw_arch" in
  x86_64|amd64)
    ARCH="x86_64"
    ;;
  arm64|aarch64)
    ARCH="arm64"
    ;;
  *)
    echo "scraper-install: unsupported architecture: $raw_arch" >&2
    exit 1
    ;;
esac

# Handle macOS Rosetta translation
if [ "$OS" = "macos" ] && [ "$ARCH" = "x86_64" ]; then
  is_proc_translated="${SCRAPER_PROC_TRANSLATED:-$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)}"
  if [ "$is_proc_translated" = "1" ]; then
    ARCH="arm64"
  fi
fi

# Handle Windows binary naming and architecture
if [ "$OS" = "windows" ]; then
  if [ "$ARCH" = "arm64" ]; then
    ARCH="x86_64"
  fi
  EXT=".exe"
  BIN_NAME="scraper.exe"
else
  EXT=""
  BIN_NAME="scraper"
fi

ASSET="scraper-${OS}-${ARCH}${EXT}"

# Construct download URLs
if [ -n "${SCRAPER_VERSION:-}" ]; then
  URL="https://github.com/$REPO/releases/download/$SCRAPER_VERSION/$ASSET"
  CHECKSUM_URL="https://github.com/$REPO/releases/download/$SCRAPER_VERSION/${ASSET}.sha256"
else
  URL="https://github.com/$REPO/releases/latest/download/$ASSET"
  CHECKSUM_URL="https://github.com/$REPO/releases/latest/download/${ASSET}.sha256"
fi

# Dry run mode
if [ "${SCRAPER_DRY_RUN:-0}" = "1" ]; then
  echo "asset: $ASSET"
  echo "url: $URL"
  exit 0
fi

# Detect downloader
if command -v curl >/dev/null 2>&1; then
  DOWNLOADER="curl"
elif command -v wget >/dev/null 2>&1; then
  DOWNLOADER="wget"
else
  echo "scraper-install: neither curl nor wget found in PATH" >&2
  exit 1
fi

TMP_DIR=$(mktemp -d 2>/dev/null || mktemp -d -t 'scraper-install')
trap 'rm -rf "$TMP_DIR"' EXIT
trap 'exit 1' INT TERM

download_file() {
  _url="$1"
  _dest="$2"
  _label="$3"

  if [ "$DOWNLOADER" = "curl" ]; then
    if ! curl -fsSL -o "$_dest" "$_url"; then
      echo "scraper-install: failed to download $_label from $_url" >&2
      exit 1
    fi
  elif [ "$DOWNLOADER" = "wget" ]; then
    if ! wget -qO "$_dest" "$_url"; then
      echo "scraper-install: failed to download $_label from $_url" >&2
      exit 1
    fi
  fi
}

# Download asset and checksum
download_file "$URL" "$TMP_DIR/$ASSET" "asset '$ASSET'"
download_file "$CHECKSUM_URL" "$TMP_DIR/${ASSET}.sha256" "checksum for asset '$ASSET'"

# Verify checksum
verify_status=0
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$TMP_DIR" && sha256sum -c "${ASSET}.sha256" >/dev/null 2>&1) || verify_status=$?
elif command -v shasum >/dev/null 2>&1; then
  (cd "$TMP_DIR" && shasum -a 256 -c "${ASSET}.sha256" >/dev/null 2>&1) || verify_status=$?
else
  echo "scraper-install: WARNING: no sha256sum or shasum found; skipping checksum verification" >&2
fi

if [ "$verify_status" -ne 0 ]; then
  echo "scraper-install: checksum verification failed for $ASSET" >&2
  exit 1
fi

# Install binary
mkdir -p "$INSTALL_DIR"
mv -f "$TMP_DIR/$ASSET" "$INSTALL_DIR/$BIN_NAME"

if [ "$OS" != "windows" ]; then
  chmod +x "$INSTALL_DIR/$BIN_NAME"
fi

# Check PATH
case ":$PATH:" in
  *":$INSTALL_DIR:"*)
    ;;
  *)
    echo ""
    echo "Note: $INSTALL_DIR is not in your PATH."
    echo "Add the following line to your ~/.bashrc or ~/.zshrc:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    echo ""
    ;;
esac

# Smoke test
if ! "$INSTALL_DIR/$BIN_NAME" --help >/dev/null 2>&1; then
  echo "scraper-install: WARNING: '$INSTALL_DIR/$BIN_NAME --help' failed. The binary may not run on this system." >&2
fi

echo "scraper installed successfully to $INSTALL_DIR/$BIN_NAME"
echo ""
echo "Next steps:"
echo "  1. Download the config template to ./scraper.toml:"
echo "     curl -fsSL https://raw.githubusercontent.com/$REPO/main/scraper.example.toml -o ./scraper.toml"
echo "  2. Edit [profile] in ./scraper.toml"
echo "  3. Run 'scraper sync'"
echo "  4. Run 'scraper scan'"
echo "  5. Run 'scraper list'"

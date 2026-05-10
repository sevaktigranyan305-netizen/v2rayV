#!/bin/bash
# Downloads the xray-core sidecar binary for the current platform.
# Usage: ./scripts/download-xray.sh [version]
#
# Source: https://github.com/sevaktigranyan305-netizen/Xray-core/releases (our fork)
# The fork bundles wintun.dll alongside xray.exe in the windows-amd64 zip
# (the desktop l3client device backend uses wintun for the TUN adapter on Windows).
#
# Output (Tauri sidecar naming convention):
#   src-tauri/binaries/xray-<target-triple>[.exe]
#   src-tauri/binaries/wintun.dll       (Windows only, bundled as a Tauri resource)

set -euo pipefail

XRAY_VERSION="${1:-v0.0.14-test}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARIES_DIR="${SCRIPT_DIR}/../src-tauri/binaries"
mkdir -p "${BINARIES_DIR}"
BINARIES_DIR="$(cd "${BINARIES_DIR}" && pwd)"
BASE_URL="https://github.com/sevaktigranyan305-netizen/Xray-core/releases/download/${XRAY_VERSION}"

# Detect OS
case "$(uname -s)" in
    Linux)   OS="linux" ;;
    Darwin)  OS="darwin" ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
             OS="windows" ;;
    *)
        echo "Error: Unsupported OS: $(uname -s)" >&2
        exit 1
        ;;
esac

# Detect architecture
case "$(uname -m)" in
    x86_64|amd64)  ARCH="amd64" ;;
    aarch64|arm64) ARCH="arm64" ;;
    *)
        echo "Error: Unsupported architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

# Map to fork release archive name and Tauri sidecar target triple
case "${OS}-${ARCH}" in
    linux-amd64)
        ARCHIVE="xray-linux-amd64.zip"
        TARGET_TRIPLE="x86_64-unknown-linux-gnu"
        BINARY_NAME="xray-${TARGET_TRIPLE}"
        ;;
    linux-arm64)
        ARCHIVE="xray-linux-arm64.zip"
        TARGET_TRIPLE="aarch64-unknown-linux-gnu"
        BINARY_NAME="xray-${TARGET_TRIPLE}"
        ;;
    darwin-arm64)
        ARCHIVE="xray-darwin-arm64.zip"
        TARGET_TRIPLE="aarch64-apple-darwin"
        BINARY_NAME="xray-${TARGET_TRIPLE}"
        ;;
    darwin-amd64)
        ARCHIVE="xray-darwin-amd64.zip"
        TARGET_TRIPLE="x86_64-apple-darwin"
        BINARY_NAME="xray-${TARGET_TRIPLE}"
        ;;
    windows-amd64)
        ARCHIVE="xray-windows-amd64.zip"
        TARGET_TRIPLE="x86_64-pc-windows-msvc"
        BINARY_NAME="xray-${TARGET_TRIPLE}.exe"
        ;;
    *)
        echo "Error: No xray binary available for ${OS}-${ARCH}" >&2
        exit 1
        ;;
esac

DOWNLOAD_URL="${BASE_URL}/${ARCHIVE}"
TEMP_DIR="$(mktemp -d)"

cleanup() {
    rm -rf "${TEMP_DIR}"
}
trap cleanup EXIT

echo "Downloading xray-core ${XRAY_VERSION} for ${OS}/${ARCH}..."
echo "  URL: ${DOWNLOAD_URL}"

curl -fsSL "${DOWNLOAD_URL}" -o "${TEMP_DIR}/xray.zip"

mkdir -p "${BINARIES_DIR}"

echo "Extracting..."
if [ "${OS}" = "windows" ]; then
    unzip -o "${TEMP_DIR}/xray.zip" -d "${TEMP_DIR}/extracted"
    mv "${TEMP_DIR}/extracted/xray.exe" "${BINARIES_DIR}/${BINARY_NAME}"
    # Bundle wintun.dll alongside xray.exe — the l3client device backend dlopens it.
    if [ -f "${TEMP_DIR}/extracted/wintun.dll" ]; then
        mv "${TEMP_DIR}/extracted/wintun.dll" "${BINARIES_DIR}/wintun.dll"
        echo "  Extracted wintun.dll for L3 TUN backend"
    else
        echo "Warning: wintun.dll not found in archive (older fork tag?). L3 TUN mode will fall back to system proxy on Windows." >&2
    fi
else
    unzip -o "${TEMP_DIR}/xray.zip" "xray" -d "${TEMP_DIR}/extracted"
    mv "${TEMP_DIR}/extracted/xray" "${BINARIES_DIR}/${BINARY_NAME}"
    chmod +x "${BINARIES_DIR}/${BINARY_NAME}"
fi

echo "Done. Binary saved to: ${BINARIES_DIR}/${BINARY_NAME}"

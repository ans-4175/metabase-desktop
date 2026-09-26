#!/usr/bin/env bash
# Download Eclipse Temurin JRE + extract ke src-tauri/resources/runtime/
# Usage:
#   bash scripts/fetch-jre.sh               # JRE 21 (default, LTS)
#   JAVA_VERSION=25 bash scripts/fetch-jre.sh
set -euo pipefail
cd "$(dirname "$0")/.."

JAVA_VERSION="${JAVA_VERSION:-21}"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64|aarch64) AARCH="aarch64" ;;
  *) AARCH="x64" ;;
esac
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
[[ "$OS" == "darwin" ]] && OS="mac"

URL="https://api.adoptium.net/v3/binary/latest/${JAVA_VERSION}/ga/${OS}/${AARCH}/jre/hotspot/normal/eclipse"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo ">> Downloading Temurin JRE ${JAVA_VERSION} (${OS}/${AARCH})"
curl -L --fail --progress-bar "$URL" -o "$TMP/jre.tar.gz"

echo ">> Extracting..."
tar -xzf "$TMP/jre.tar.gz" -C "$TMP"

SRC="$(find "$TMP" -type d -path '*/Contents/Home' | head -n1)"
if [[ -z "$SRC" ]]; then
  # macOS layout not found — Linux/Windows tarballs put bin/ at the top level
  JAVA_EXE="$(find "$TMP" -type f \( -path '*/bin/java' -o -path '*/bin/java.exe' \) | head -n1)"
  if [[ -n "$JAVA_EXE" ]]; then
    SRC="$(dirname "$(dirname "$JAVA_EXE")")"
  fi
fi
if [[ -z "$SRC" ]]; then
  echo "!! Could not find the JRE Home directory inside the tarball"
  exit 1
fi

rm -rf src-tauri/resources/runtime
mkdir -p src-tauri/resources/runtime
cp -R "$SRC/." src-tauri/resources/runtime/
chmod -R u+w src-tauri/resources/runtime
chmod +x src-tauri/resources/runtime/bin/*
echo ">> JRE installed: $(src-tauri/resources/runtime/bin/java -version 2>&1 | head -n1)"

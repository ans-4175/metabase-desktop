#!/usr/bin/env bash
# Download Metabase OSS JAR ke src-tauri/resources/metabase.jar
# Usage:
#   bash scripts/fetch-jar.sh                          # pinned version (default)
#   METABASE_VERSION=latest bash scripts/fetch-jar.sh  # versi terbaru
#   METABASE_VERSION=0.64.0 bash scripts/fetch-jar.sh  # versi spesifik
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${METABASE_VERSION:-0.63.18}"
if [[ "$VERSION" == "latest" ]]; then
  URL="https://downloads.metabase.com/latest/metabase.jar"
else
  URL="https://downloads.metabase.com/v${VERSION}/metabase.jar"
fi

DEST="src-tauri/resources/metabase.jar"
mkdir -p src-tauri/resources
echo ">> Downloading Metabase ${VERSION} -> ${DEST}"
curl -L --fail --progress-bar "$URL" -o "$DEST"
echo ">> Done: $(du -h "$DEST" | cut -f1)"

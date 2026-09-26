#!/usr/bin/env bash
# Bersihin semua jejak Metabase Desktop: data folder + app bundle.
set -euo pipefail

DATA="$HOME/Library/Application Support/com.anshori.metabase-desktop"

# safety guard: jangan rm -rf sembarangan
case "$DATA" in
  *"com.anshori.metabase-desktop") : ;;
  *) echo "!! Unexpected path, aborted"; exit 1 ;;
esac

if [[ -d "$DATA" ]]; then
  echo ">> Removing data: $DATA"
  rm -rf "$DATA"
else
  echo ">> Data folder not found (already clean)."
fi

for APP in "/Applications/Metabase Desktop.app" "$HOME/Applications/Metabase Desktop.app"; do
  if [[ -d "$APP" ]]; then
    echo ">> Removing app: $APP"
    rm -rf "$APP"
  fi
done

echo ">> Done. All Metabase Desktop traces removed."

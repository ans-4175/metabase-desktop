#!/usr/bin/env bash
# Bersihin semua jejak Metabase Desktop: data folder + app bundle.
set -euo pipefail

DATA="$HOME/Library/Application Support/com.anshori.metabase-desktop"

# safety guard: jangan rm -rf sembarangan
case "$DATA" in
  *"com.anshori.metabase-desktop") : ;;
  *) echo "!! Path tidak terduga, dibatalkan"; exit 1 ;;
esac

if [[ -d "$DATA" ]]; then
  echo ">> Menghapus data: $DATA"
  rm -rf "$DATA"
else
  echo ">> Data folder tidak ada (sudah bersih)."
fi

for APP in "/Applications/Metabase Desktop.app" "$HOME/Applications/Metabase Desktop.app"; do
  if [[ -d "$APP" ]]; then
    echo ">> Menghapus app: $APP"
    rm -rf "$APP"
  fi
done

echo ">> Beres. Semua jejak Metabase Desktop udah kehapus."

default:
    @just --list

# Download metabase.jar + JRE ke src-tauri/resources/ (wajib sebelum build)
setup: fetch-jar fetch-jre
    @echo ">> Setup lengkap. Lanjut: just dev (coba jalan) atau just build (.app)"

# Download metabase.jar pinned (override: METABASE_VERSION=0.64.0 just fetch-jar)
fetch-jar:
    bash scripts/fetch-jar.sh

# Download Temurin JRE 21 (override: JAVA_VERSION=25 just fetch-jre)
fetch-jre:
    bash scripts/fetch-jre.sh

# Update metabase.jar ke versi terbaru lalu rebuild
update-jar:
    METABASE_VERSION=latest bash scripts/fetch-jar.sh

# Jalankan app mode dev
dev:
    npx --yes @tauri-apps/cli@^2 dev

# Build .app + .dmg release
build:
    npx --yes @tauri-apps/cli@^2 build

# Regenerate icon set dari src-tauri/icons/app-icon.png
icon:
    npx --yes @tauri-apps/cli@^2 icon src-tauri/icons/app-icon.png

# Hapus semua data + app Metabase Desktop dari mesin ini
uninstall:
    bash scripts/uninstall.sh

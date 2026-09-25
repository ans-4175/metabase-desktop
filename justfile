default:
    @just --list

# Download the JRE into src-tauri/resources/ (the only required pre-build step)
setup: fetch-jre
    @echo ">> JRE setup done. metabase.jar is auto-downloaded on first app launch."

# (Optional) manually download metabase.jar — for offline pre-seed / dev fallback
fetch-jar:
    bash scripts/fetch-jar.sh

# Download Temurin JRE 21 (override: JAVA_VERSION=25 just fetch-jre)
fetch-jre:
    bash scripts/fetch-jre.sh

# (Opsional) refresh jar di resources buat dev fallback
update-jar:
    METABASE_VERSION=latest bash scripts/fetch-jar.sh

# Run the app in dev mode
dev:
    npx --yes @tauri-apps/cli@^2 dev

# Build .app + .dmg release
build:
    npx --yes @tauri-apps/cli@^2 build

# Regenerate icon set from src-tauri/icons/app-icon.png
icon:
    npx --yes @tauri-apps/cli@^2 icon src-tauri/icons/app-icon.png

# Remove all Metabase Desktop data + app from this machine
uninstall:
    bash scripts/uninstall.sh

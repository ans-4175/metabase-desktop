# Metabase Desktop

> Metabase, but as a real desktop app. Setup? Already handled — everything it needs
> rides in the box, and it fetches the latest Metabase all by itself.
> You bring the questions, it brings the dashboards.

Built with Tauri 2. The app is ~160MB; `metabase.jar` (~640MB) is **not** baked in —
it's auto-downloaded once on first run, straight into the data folder.

## Architecture

```
Metabase Desktop.app  (~160MB)
├── Tauri shell (Rust)   → spawns Java, health-checks, splash, native menus
├── resources/runtime/   → bundled Temurin JRE 21 (no Java install needed)
└── WebView → http://127.0.0.1:9010  (Metabase UI)

First run → downloads the latest metabase.jar (~640MB) once, into the data folder.
Updates   → menu "Check for Metabase Updates…" downloads the new jar, backend restarts.
```

All state (H2 database, logs, the downloaded jar) lives in **one folder**:
`~/Library/Application Support/com.anshori.metabase-desktop/`

## Setup

Requires: Rust (`rustup`), Node (for the Tauri CLI). Optional: [`just`](https://github.com/casey/just) (`brew install just`) — a `make`-style command runner.

```bash
just setup      # download Temurin JRE (~130MB) — the only required step
just dev        # run in dev mode
just build      # build release .app + .dmg
```

Without `just`: run `bash scripts/fetch-jre.sh` then `npx --yes @tauri-apps/cli@^2 build`.

> Offline first run? Drop a `metabase.jar` into the data folder manually
> (`just fetch-jar` then copy) — the app detects it and skips the download.

## Updating Metabase

- **In-app**: menu → *Check for Metabase Updates…* — downloads the latest jar
  (with a progress window), then restarts the backend onto it. The download is
  atomic: if it fails halfway, the old version keeps running untouched.
- **Rollback**: delete `metabase.jar` in the data folder → re-downloaded next start.

## Settings

Menu → *Settings…* — a proper little form (no config-file wrangling):

| Key | Meaning | Default |
|---|---|---|
| Local port | Metabase serves on 127.0.0.1, fails fast if taken | `9010` |
| Max memory | `-Xmx` ceiling (idle Metabase only uses ~300–500MB) | `1g` |

Hit **Save & Restart** and the backend restarts with the new values.
Under the hood it's still a plain `config.env` in the data folder — edit and save
that file directly and the watcher picks it up too.

## Menu (macOS)

- **Check for Metabase Updates…** — download latest jar + restart backend
- **Restart Backend** — apply config manually
- **Settings…** — port & memory form
- **Reveal Data Folder** — open the data folder in Finder
- **Uninstall Metabase Desktop…** — confirmation dialog → wipes the data folder and removes the app itself

## Uninstall

In-app: menu → *Uninstall Metabase Desktop…* → confirm. Done.

Or from the CLI: `just uninstall`, or manually: drag the `.app` to Trash +
`rm -rf ~/Library/Application\ Support/com.anshori.metabase-desktop`

## Windows / Linux

The Tauri shell is already cross-platform, but four spots are macOS-specific today:

1. Java path: `bin/java` → needs `bin/java.exe` on Windows (`cfg!(windows)`)
2. Backend shutdown: SIGTERM (unix) → Windows needs `child.kill()` / `taskkill`
3. Bundle targets: add `nsis` (Windows) / `deb`, `appimage` (Linux) in `tauri.conf.json`
4. Per-platform JRE: `fetch-jre.sh` already detects OS/arch (mac/linux); Windows wants a PowerShell variant
5. Builds must run on each OS → GitHub Actions matrix (macos/windows/ubuntu runners)

## Notes

- Metabase OSS is **AGPL**-licensed — fine for personal/internal use.
  Distributing a public product? You must open-source your integration, can't
  white-label, and can't use the Metabase name/logo.
- First run needs internet (jar download). After that it's fully offline.
- The Metabase first-run wizard (admin + DB connection) appears once, in-window.
- Metabase logs: `<data folder>/logs/metabase.log`

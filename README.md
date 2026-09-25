# Metabase Desktop

<p align="center"><em>All the dashboards. None of the setup ritual.</em></p>

**Metabase OSS as a real native desktop app** — the kind you double-click.
It carries its own Java runtime, fetches the latest Metabase for you, and cleans
up after itself. **No JDK. No terminal. No environment puzzles.**

---

## Why this exists

The official way to try Metabase from a JAR goes something like this:

1. Install the *right* Java — version 21+, and hope your system Java isn't older
2. Download a **632MB** JAR
3. Open a terminal and type a command that includes `--add-opens` incantations
4. Keep that terminal open for as long as you use Metabase
5. Remember that the thing lives on `localhost:3000`
6. To update? Congrats, repeat from step 2

Each step is small. Together, they're the reason *"let me just try Metabase"*
quietly becomes *"maybe this weekend."*

**Metabase Desktop collapses all of it into one installer:**

| The old way | Metabase Desktop |
|---|---|
| Install JDK 21, pray about version conflicts | JRE 21 rides **inside** the app — right version, guaranteed |
| Terminal incantation with `--add-opens` | Double-click |
| Terminal window must stay open forever | Real app window; quit = clean, data-safe shutdown |
| Hunt down a new JAR by hand to update | Menu → *Check for Updates* (it even checks versions first) |
| Data scattered wherever the terminal was standing | Everything in **one folder**, removable from a menu |

So: yes, you can absolutely run the JAR yourself. This repo exists because the
*first five minutes* of evaluating a BI tool shouldn't be a Java homework assignment.

## Quick start

**Option 1 — installers (easiest):** push a `v*` tag or run the *Release* workflow
(see [Cross-platform](#cross-platform--releases)) and grab the draft GitHub Release:
`.dmg` for macOS, `.msi`/`.exe` for Windows, `.deb`/`.AppImage` for Linux.

**Option 2 — build from source (~10 min, one-time):**

```bash
git clone <this-repo> && cd metabase-tauri
just setup      # or: bash scripts/fetch-jre.sh
just build      # or: npx --yes @tauri-apps/cli@^2 build
```

Requires Rust + Node; `just` is optional sugar (`brew install just`).

### What first launch looks like

1. A splash appears: *"First run: downloading Metabase v0.63.x (~640MB) — one time only"* with a real progress bar
2. When it's ready, the window opens straight into Metabase's setup wizard
3. That's it. Every next launch boots in ~20–30 seconds, fully offline

The app itself is only **~160MB** — the big JAR isn't baked in, it's fetched once
into the data folder and version-checked on every update.

## How it works

```
Metabase Desktop.app  (~160MB)
├── Tauri shell (Rust)   → spawns Java, health-checks, splash, native menus
├── resources/runtime/   → bundled Temurin JRE 21 (you install nothing)
└── WebView → http://127.0.0.1:9010  (Metabase UI)

First run → downloads the latest metabase.jar once, into the data folder
Updates   → GitHub releases API checks the version first; downloads only if newer
```

All state — H2 database, logs, the downloaded JAR — lives in **one folder**
(`~/Library/Application Support/com.anshori.metabase-desktop/` on macOS),
so backing up or nuking your Metabase is a single `rm`.

## Features

- 🖥️ **Native window** with splash screen — closes cleanly, SIGTERM-graded shutdown keeps the H2 database safe
- 📦 **Zero-setup runtime** — bundled Temurin JRE 21, per-OS fetched (mac/win/linux scripts)
- 🔄 **Smart updates** — version check before any download; atomic jar swap; failed download = old version keeps running
- 🎛️ **Settings form** — local port + memory (1g–8g dropdown), saved = backend auto-restarts
- 🧹 **One-folder data + in-app uninstall** — the app can delete itself, cleanly
- 🔍 **Fail-fast port handling** — no silent port hopping; if 9010 is taken, it says so

## Settings

Menu → *Settings…*

| Key | Meaning | Default |
|---|---|---|
| Local port | Metabase serves on 127.0.0.1; fails fast if taken | `9010` |
| Max memory | JVM heap ceiling (idle Metabase only uses ~300–500MB) | `1g` |

Under the hood it's a plain `config.env` in the data folder — edit and save that
file directly and the file watcher applies it too.

## Cross-platform & releases

The shell is fully cross-platform. `.github/workflows/release.yml` builds and
attaches installers for **macOS (Apple Silicon)**, **Windows (NSIS/MSI)** and
**Linux (deb/AppImage)** on every `v*` tag:

```bash
git tag v0.1.0 && git push origin v0.1.0
# → Actions tab → draft release with all installers → publish
```

Per-OS bits handled in code/config: `java.exe` vs `java` path, Windows process
shutdown, bundle targets (`"all"`), and a PowerShell JRE fetcher
(`scripts/fetch-jre.ps1`) alongside the shell one.

## Uninstall

In-app: menu → *Uninstall Metabase Desktop…* → confirm. It removes the data
folder and the app itself. Or `just uninstall`, or manually drag the app to
Trash + delete the data folder above.

## Notes

- Metabase OSS is **AGPL**-licensed — this wrapper is for personal/internal use;
  distributing a public product has license implications (no white-labeling, no
  Metabase name/logo).
- First launch needs internet (one JAR download). After that, fully offline.
- Metabase logs: `<data folder>/logs/metabase.log`

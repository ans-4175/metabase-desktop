# Metabase Desktop

Metabase OSS (JAR) yang dibungkus jadi native desktop app pakai Tauri 2.
Window native + splash screen saat boot + menu macOS buat settings/update — tanpa buka browser.

## Arsitektur

```
Metabase Desktop.app
├── Tauri shell (Rust)        → spawn Java, healthcheck, splash, menu native
├── resources/metabase.jar    → baked-in fallback (versi pin)
├── resources/runtime/        → Temurin JRE 21 (bundled, user gak perlu install Java)
└── WebView → http://127.0.0.1:9010  (UI Metabase)
```

Data (H2, log, jar hasil update) semua di **satu folder**:
`~/Library/Application Support/com.anshori.metabase-desktop/`

## Setup

```bash
just setup      # download metabase.jar (~350MB) + Temurin JRE (~130MB)
just dev        # coba jalan (mode dev)
just build      # build .app + .dmg release
```

## Update Metabase

- **Dalam app**: menu → *Cek Update Metabase…* — download versi terbaru ke data folder,
  backend otomatis restart pakai jar baru. Versi baked-in jadi fallback.
- **Via CLI**: `just update-jar && just build` (ganti jar baked-in).

Rollback: hapus `metabase.jar` di data folder → app balik pakai jar baked-in.

## Config

Menu → *Edit Settings…* (buka `config.env` di data folder), lalu menu → *Restart Backend*:

```
PORT=9010    # port lokal Metabase (default 9010, fail-fast kalau kepakai)
XMX=1g       # heap JVM maksimal (default 1g; idle cuma pakai ~300-500MB)
```

## Menu macOS

- **Cek Update Metabase…** — download jar terbaru + restart backend
- **Restart Backend** — apply config tanpa quit app
- **Edit Settings…** — buka config.env
- **Reveal Data Folder** — buka folder data di Finder

## Uninstall

```bash
just uninstall   # hapus data folder + app bundle
```
atau manual: drag `.app` ke Trash + `rm -rf ~/Library/Application\ Support/com.anshori.metabase-desktop`

## Catatan

- Metabase OSS berlisensi **AGPL** — aman buat personal/internal use.
  Buat distribusi produk publik: wajib buka source integrasi, gak boleh whitelabel,
  gak boleh pakai nama/logo Metabase.
- First-run: wizard setup Metabase (admin + DB connection) muncul sekali di window.
- Log Metabase: `<data folder>/logs/metabase.log`

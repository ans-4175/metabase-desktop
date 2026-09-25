// Metabase Desktop — wraps Metabase OSS JAR as a native desktop app.
// Flow: splash window -> spawn bundled JRE + metabase.jar -> poll /api/health
//       -> open main window at http://127.0.0.1:<port> -> kill Java on exit.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

const DEFAULT_PORT: u16 = 9010;
const DEFAULT_XMX: &str = "1g";
const METABASE_LATEST_URL: &str = "https://downloads.metabase.com/latest/metabase.jar";
const GITHUB_LATEST_API: &str = "https://api.github.com/repos/metabase/metabase/releases/latest";

struct AppState {
    child: Mutex<Option<Child>>,
    data_dir: PathBuf,
    port: Mutex<u16>,
    xmx: Mutex<String>,
}

// ---------- config ----------

fn load_config(data_dir: &PathBuf) -> (u16, String) {
    let mut port = DEFAULT_PORT;
    let mut xmx = DEFAULT_XMX.to_string();
    let cfg_path = data_dir.join("config.env");
    if !cfg_path.exists() {
        let _ = fs::write(
            &cfg_path,
            "# Metabase Desktop - config\n# Edit and save — the backend restarts automatically.\n\n# PORT=9010\n# XMX=1g\n",
        );
    }
    if let Ok(content) = fs::read_to_string(&cfg_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim().to_ascii_uppercase();
                let v = v.trim().to_string();
                match k.as_str() {
                    "PORT" => {
                        if let Ok(p) = v.parse() {
                            port = p;
                        }
                    }
                    "XMX" => {
                        if !v.is_empty() {
                            xmx = v;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    (port, xmx)
}

// ---------- path helpers ----------

fn java_rel() -> &'static str {
    if cfg!(windows) {
        "resources/runtime/bin/java.exe"
    } else {
        "resources/runtime/bin/java"
    }
}

fn java_bin(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(p) = app.path().resolve(java_rel(), BaseDirectory::Resource) {
        if p.exists() {
            return Some(p);
        }
    }
    // dev fallback: run via `just dev` from project root
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(java_rel());
    dev.exists().then_some(dev)
}

fn baked_jar(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(p) = app.path().resolve("resources/metabase.jar", BaseDirectory::Resource) {
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn port_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn jar_available(app: &AppHandle, data_dir: &PathBuf) -> bool {
    data_dir.join("metabase.jar").exists() || baked_jar(app).is_some()
}

// Ask GitHub what the newest OSS release is. None = couldn't tell (offline?).
async fn latest_release() -> Option<String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(GITHUB_LATEST_API)
        .header("User-Agent", "metabase-desktop")
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    let text = resp.text().await.ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(
        json.get("tag_name")?
            .as_str()?
            .trim()
            .trim_start_matches('v')
            .to_string(),
    )
    .filter(|s| !s.is_empty())
}

fn pinned_url(version: &str) -> String {
    format!("https://downloads.metabase.com/v{version}/metabase.jar")
}

async fn head_content_length(url: &str) -> Option<u64> {
    let client = reqwest::Client::new();
    let resp = client
        .head(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    resp.content_length()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JarMeta {
    version: String,
    etag: Option<String>,
    size: u64,
}

fn meta_path(dest: &PathBuf) -> PathBuf {
    dest.with_extension("meta.json")
}

fn save_meta(dest: &PathBuf, meta: &JarMeta) {
    if let Ok(s) = serde_json::to_string(meta) {
        let _ = std::fs::write(meta_path(dest), s);
    }
}

fn read_meta(dest: &PathBuf) -> Option<JarMeta> {
    serde_json::from_str(&std::fs::read_to_string(meta_path(dest)).ok()?).ok()
}

// Download metabase.jar latest ke `dest` (atomic: tulis ke .download dulu, rename kalau sukses)
async fn download_jar(
    app: &AppHandle,
    dest: &PathBuf,
    url: &str,
    version: Option<&str>,
) -> Result<(), String> {
    let _ = app.emit("update-status", "Connecting to downloads.metabase.com…".to_string());
    let client = reqwest::Client::new();
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    let total = resp.content_length();
    let tmp = dest.with_extension("jar.download");
    let mut file = File::create(&tmp).map_err(|e| format!("Failed to write file: {e}"))?;
    let mut received: u64 = 0;
    let mut last_emit = Instant::now();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                file.write_all(&chunk)
                    .map_err(|e| format!("Failed to write file: {e}"))?;
                received += chunk.len() as u64;
                if last_emit.elapsed() > Duration::from_millis(200) {
                    last_emit = Instant::now();
                    let _ = app.emit("update-progress", json!({"received": received, "total": total}));
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(format!("Download interrupted: {e}"));
            }
        }
    }
    drop(file);
    let _ = app.emit("update-progress", json!({"received": received, "total": total}));
    let _ = fs::remove_file(dest);
    fs::rename(&tmp, dest).map_err(|e| format!("Failed to finalize jar: {e}"))?;
    save_meta(
        dest,
        &JarMeta {
            version: version.unwrap_or("unknown").to_string(),
            etag: resp
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(String::from),
            size: received,
        },
    );
    Ok(())
}

// Watch config.env — tiap user save editan, backend otomatis restart
fn spawn_config_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let cfg = app.state::<AppState>().data_dir.join("config.env");
        let mtime = || fs::metadata(&cfg).and_then(|m| m.modified()).ok();
        let mut last = mtime();
        loop {
            std::thread::sleep(Duration::from_millis(1500));
            let cur = mtime();
            if cur.is_some() && cur != last {
                // debounce: tunggu user beneran selesai edit
                std::thread::sleep(Duration::from_millis(2000));
                last = mtime();
                let _ = app.emit("backend-status", "Config changed — restarting backend…".to_string());
                restart_backend(&app);
            }
        }
    });
}

// ---------- backend lifecycle ----------

fn spawn_backend(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();

    // guard: don't double-spawn if a live process exists
    {
        let mut guard = state.child.lock().unwrap();
        if let Some(c) = guard.as_mut() {
            if c.try_wait().map_or(true, |o| o.is_none()) {
                return Ok(());
            }
            *guard = None;
        }
    }

    let data_dir = state.data_dir.clone();
    let port = *state.port.lock().unwrap();
    let xmx = state.xmx.lock().unwrap().clone();

    if !port_free(port) {
        let msg = format!(
            "Port {port} is already in use by another process.\nChange PORT in config.env (menu: Settings) or stop that process."
        );
        return Err(msg);
    }

    let java = java_bin(app)
        .ok_or_else(|| "Bundled JRE not found. Run: just fetch-jre".to_string())?;
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&java, fs::Permissions::from_mode(0o755));
    }

    // hybrid: prefer user-downloaded jar in data dir, fall back to baked-in jar
    let jar = {
        let p = data_dir.join("metabase.jar");
        if p.exists() {
            p
        } else {
            baked_jar(app)
                .ok_or_else(|| "metabase.jar not found in data folder or resources".to_string())?
        }
    };

    let logs_dir = data_dir.join("logs");
    let _ = fs::create_dir_all(&logs_dir);
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs_dir.join("metabase.log"))
        .map_err(|e| format!("Failed to open log file: {e}"))?;
    let err_log = log.try_clone().map_err(|e| e.to_string())?;

    let child = Command::new(&java)
        .arg(format!("-Xmx{xmx}"))
        .args(["--add-opens", "java.base/java.nio=ALL-UNNAMED"])
        .arg("-jar")
        .arg(&jar)
        .env("MB_JETTY_PORT", port.to_string())
        .env("MB_DB_FILE", data_dir.join("metabase.db").as_os_str())
        .env("MB_ANON_TRACKING_ENABLED", "false")
        .env("MB_CHECK_FOR_UPDATES", "false")
        .current_dir(&data_dir)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log))
        .spawn()
        .map_err(|e| format!("Failed to launch Java: {e}"))?;

    *state.child.lock().unwrap() = Some(child);
    drop(state);

    let _ = app.emit("backend-status", "Starting Metabase… usually takes 15–40s.");

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("client");
        let health = format!("http://127.0.0.1:{port}/api/health");
        let started = Instant::now();
        let mut last_status = Instant::now();
        loop {
            {
                let st = handle.state::<AppState>();
                let mut guard = st.child.lock().unwrap();
                if let Some(c) = guard.as_mut() {
                    if matches!(c.try_wait(), Ok(Some(_))) {
                        *guard = None;
                        drop(guard);
                        let _ = handle.emit(
                            "backend-error",
                            "The Metabase process died unexpectedly.\nCheck logs/metabase.log in the data folder (menu: Reveal Data Folder).",
                        );
                        return;
                    }
                } else {
                    return;
                }
            }
            if let Ok(resp) = client.get(&health).send().await {
                if resp.status().is_success() {
                    let status = resp
                        .text()
                        .await
                        .ok()
                        .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
                        .and_then(|v| {
                            v.get("status")
                                .and_then(|s| s.as_str())
                                .map(String::from)
                        });
                    match status.as_deref() {
                        Some("ok") => {
                            let _ = handle.emit(
                                "backend-ready",
                                json!({"port": port, "boot_secs": started.elapsed().as_secs()}),
                            );
                            open_main(&handle, port);
                            return;
                        }
                        // 200 tapi masih initializing — jangan buka window dulu
                        Some("initializing") => {
                            if last_status.elapsed() > Duration::from_secs(5) {
                                last_status = Instant::now();
                                let _ = handle.emit(
                                    "backend-status",
                                    format!(
                                        "Metabase is initializing the database… {}s",
                                        started.elapsed().as_secs()
                                    ),
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            if started.elapsed() > Duration::from_secs(180) {
                let _ = handle.emit(
                    "backend-error",
                    "Timeout: Metabase isn't ready after 3 minutes.\nCheck logs/metabase.log in the data folder.",
                );
                return;
            }
            if last_status.elapsed() > Duration::from_secs(5) {
                last_status = Instant::now();
                let _ = handle.emit(
                    "backend-status",
                    format!("Still starting… {}s", started.elapsed().as_secs()),
                );
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    Ok(())
}

fn open_main(app: &AppHandle, port: u16) {
    match app.get_webview_window("main") {
        Some(win) => {
            // navigate (bukan cuma reload) — support PORT berubah lewat config
            let _ = win.eval(&format!("location.href = 'http://127.0.0.1:{port}/'"));
        }
        None => {
            let url: tauri::Url = format!("http://127.0.0.1:{port}/").parse().expect("url");
            if let Ok(win) = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("Metabase")
                .inner_size(1280.0, 820.0)
                .min_inner_size(960.0, 600.0)
                .build()
            {
                let _ = win.set_focus();
            }
        }
    }
    if let Some(splash) = app.get_webview_window("splash") {
        let _ = splash.close();
    }
    if let Some(updater) = app.get_webview_window("updater") {
        let _ = updater.close();
    }
}

fn stop_backend(state: &AppState) {
    let mut guard = state.child.lock().unwrap();
    if let Some(mut child) = guard.take() {
        #[cfg(unix)]
        {
            // SIGTERM dulu biar Metabase shutdown bersih (H2 flush)
            unsafe {
                libc::kill(child.id() as i32, libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => {
                        if Instant::now() > deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        }
        #[cfg(windows)]
        {
            // Windows: no SIGTERM — TerminateProcess langsung
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn restart_backend(app: &AppHandle) {
    let state = app.state::<AppState>();
    stop_backend(&state);
    let (port, xmx) = load_config(&state.data_dir);
    *state.port.lock().unwrap() = port;
    *state.xmx.lock().unwrap() = xmx.clone();
    drop(state);
    let _ = app.emit(
        "backend-status",
        format!("Restarting Metabase (Xmx {xmx}, port {port})…"),
    );
    if let Err(e) = spawn_backend(app) {
        let _ = app.emit("backend-error", e);
    }
}

// ---------- update ----------

async fn run_update(app: AppHandle) {
    let data_dir = {
        let st = app.state::<AppState>();
        st.data_dir.clone()
    };
    let _ = app.emit(
        "update-status",
        "Checking the latest Metabase release…".to_string(),
    );
    let latest = match latest_release().await {
        Some(v) => v,
        None => {
            let _ = app.emit(
                "update-status",
                "Couldn't check the latest version (offline?). Try again later.".to_string(),
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
            if let Some(w) = app.get_webview_window("updater") {
                let _ = w.close();
            }
            return;
        }
    };

    let dest = data_dir.join("metabase.jar");
    let current = read_meta(&dest).map(|m| m.version).unwrap_or_default();
    let current = if current == "unknown" { String::new() } else { current };

    // no version metadata (pre-seeded jar)? cheap size check before pulling 640MB
    let up_to_date = if !current.is_empty() {
        current == latest
    } else if let Ok(local) = fs::metadata(&dest) {
        head_content_length(METABASE_LATEST_URL)
            .await
            .map_or(false, |remote| remote == local.len())
    } else {
        false
    };

    if up_to_date {
        let have = if current.is_empty() {
            "your current build".to_string()
        } else {
            format!("v{current}")
        };
        let _ = app.emit("update-status", format!("You're up to date ✓ ({have})"));
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Some(w) = app.get_webview_window("updater") {
            let _ = w.close();
        }
        return;
    }

    let intro = if current.is_empty() {
        format!("Metabase v{latest} — downloading…")
    } else {
        format!("Metabase v{latest} available (you have v{current}) — downloading…")
    };
    let _ = app.emit("update-status", intro);

    match download_jar(&app, &dest, &pinned_url(&latest), Some(&latest)).await {
        Ok(()) => {
            let _ = app.emit(
                "update-status",
                "Download complete. Restarting backend with the new version…".to_string(),
            );
            // stop_backend pake std sleep — jangan blok async worker
            let app2 = app.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || restart_backend(&app2));
        }
        Err(e) => {
            let _ = app.emit(
                "update-status",
                format!("Update failed: {e}\nMetabase keeps running on the old version."),
            );
            if let Some(w) = app.get_webview_window("updater") {
                let _ = w.close();
            }
        }
    }
}

// ---------- commands ----------

#[tauri::command]
fn retry_backend(app: AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = handle.emit("backend-status", "Starting Metabase…");
        let data_dir = handle.state::<AppState>().data_dir.clone();
        if !jar_available(&handle, &data_dir) {
            let latest = latest_release().await;
            let (url, label) = match &latest {
                Some(v) => (pinned_url(v), format!("v{v}")),
                None => (METABASE_LATEST_URL.to_string(), "latest".to_string()),
            };
            let _ = handle.emit("backend-status", format!("Downloading Metabase {label}…"));
            if let Err(e) = download_jar(
                &handle,
                &data_dir.join("metabase.jar"),
                &url,
                latest.as_deref(),
            )
            .await
            {
                let _ = handle.emit("backend-error", e);
                return;
            }
        }
        if let Err(e) = spawn_backend(&handle) {
            let _ = handle.emit("backend-error", e);
        }
    });
}

// ---------- settings ----------

#[derive(serde::Serialize)]
struct ConfigView {
    port: u16,
    xmx: String,
}

#[tauri::command]
fn get_config(app: AppHandle) -> ConfigView {
    let st = app.state::<AppState>();
    let port = *st.port.lock().unwrap();
    let xmx = st.xmx.lock().unwrap().clone();
    ConfigView { port, xmx }
}

// rewrite config.env while preserving comments & unknown keys
fn update_config_file(data_dir: &PathBuf, port: &str, xmx: &str) -> std::io::Result<()> {
    let path = data_dir.join("config.env");
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect();
    let (mut port_done, mut xmx_done) = (false, false);
    for line in lines.iter_mut() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((k, _)) = t.split_once('=') {
            match k.trim().to_ascii_uppercase().as_str() {
                "PORT" => {
                    *line = format!("PORT={port}");
                    port_done = true;
                }
                "XMX" => {
                    *line = format!("XMX={xmx}");
                    xmx_done = true;
                }
                _ => {}
            }
        }
    }
    if !port_done {
        lines.push(format!("PORT={port}"));
    }
    if !xmx_done {
        lines.push(format!("XMX={xmx}"));
    }
    std::fs::write(&path, lines.join("\n") + "\n")
}

#[tauri::command]
fn save_config(app: AppHandle, port: u16, xmx: String) -> Result<(), String> {
    if !(1024..=65535).contains(&port) {
        return Err("Port must be between 1024 and 65535".to_string());
    }
    let x = xmx.trim().to_lowercase();
    let ok_x = x.len() >= 2
        && matches!(x.chars().last(), Some('m') | Some('g'))
        && x[..x.len() - 1].chars().all(|c| c.is_ascii_digit())
        && x[..x.len() - 1].parse::<u32>().map(|n| n > 0).unwrap_or(false);
    if !ok_x {
        return Err("Memory must look like 512m, 1g, 2g…".to_string());
    }
    let st = app.state::<AppState>();
    update_config_file(&st.data_dir, &port.to_string(), &x).map_err(|e| e.to_string())
}

// ---------- main ----------

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            retry_backend,
            get_config,
            save_config
        ])
        .setup(|app| {
            let data_dir = app.path().app_data_dir().expect("no app data dir");
            fs::create_dir_all(&data_dir).expect("cannot create data dir");
            let (port, xmx) = load_config(&data_dir);

            // --- native menu (macOS) ---
            let about = MenuItem::with_id(app, "about", "About Metabase Desktop", true, None::<&str>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let check = MenuItem::with_id(app, "check-updates", "Check for Metabase Updates…", true, None::<&str>)?;
            let restart = MenuItem::with_id(app, "restart-backend", "Restart Backend", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let reveal = MenuItem::with_id(app, "reveal-data", "Reveal Data Folder", true, None::<&str>)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let uninstall =
                MenuItem::with_id(app, "uninstall", "Uninstall Metabase Desktop…", true, None::<&str>)?;
            let quit = PredefinedMenuItem::quit(app, Some("Quit"))?;
            let app_menu = Submenu::with_items(
                app,
                "Metabase Desktop",
                true,
                &[&about, &sep1, &check, &restart, &settings, &reveal, &sep2, &uninstall, &quit],
            )?;

            let edit_menu = Submenu::new(app, "Edit", true)?;
            for item in [
                PredefinedMenuItem::undo(app, None)?,
                PredefinedMenuItem::redo(app, None)?,
                PredefinedMenuItem::separator(app)?,
                PredefinedMenuItem::cut(app, None)?,
                PredefinedMenuItem::copy(app, None)?,
                PredefinedMenuItem::paste(app, None)?,
                PredefinedMenuItem::select_all(app, None)?,
            ] {
                edit_menu.append(&item)?;
            }
            let menu = Menu::with_items(app, &[&app_menu, &edit_menu])?;
            app.set_menu(menu)?;

            let data_dir_for_boot = data_dir.clone();
            app.manage(AppState {
                child: Mutex::new(None),
                data_dir,
                port: Mutex::new(port),
                xmx: Mutex::new(xmx),
            });

            let handle = app.handle().clone();
            spawn_config_watcher(handle.clone());
            tauri::async_runtime::spawn(async move {
                let _ = handle.emit("backend-status", "Preparing environment…");
                // bersihin sisa download yang kepotong sesi sebelumnya
                let _ = fs::remove_file(data_dir_for_boot.join("metabase.jar.download"));
                // preflight: tanpa jar di mana-mana → download sekali di awal
                if !jar_available(&handle, &data_dir_for_boot) {
                    let latest = latest_release().await;
                    let (url, label) = match &latest {
                        Some(v) => (pinned_url(v), format!("v{v}")),
                        None => (METABASE_LATEST_URL.to_string(), "latest".to_string()),
                    };
                    let _ = handle.emit(
                        "backend-status",
                        format!("First run: downloading Metabase {label} (~640MB) — one time only…"),
                    );
                    if let Err(e) = download_jar(
                        &handle,
                        &data_dir_for_boot.join("metabase.jar"),
                        &url,
                        latest.as_deref(),
                    )
                    .await
                    {
                        let _ = handle.emit("backend-error", e);
                        return;
                    }
                }
                if let Err(e) = spawn_backend(&handle) {
                    let _ = handle.emit("backend-error", e);
                }
            });
            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "check-updates" => {
                let a = app.clone();
                tauri::async_runtime::spawn(async move {
                    if a.get_webview_window("updater").is_none() {
                        let _ = WebviewWindowBuilder::new(
                            &a,
                            "updater",
                            WebviewUrl::App("updater.html".into()),
                        )
                        .title("Metabase Update")
                        .inner_size(420.0, 180.0)
                        .resizable(false)
                        .center()
                        .build();
                    }
                    run_update(a).await;
                });
            }
            "restart-backend" => restart_backend(app),
            "about" => {
                match app.get_webview_window("about") {
                    Some(w) => {
                        let _ = w.set_focus();
                    }
                    None => {
                        let _ = WebviewWindowBuilder::new(
                            app,
                            "about",
                            WebviewUrl::App("about.html".into()),
                        )
                        .title("About Metabase Desktop")
                        .inner_size(400.0, 340.0)
                        .resizable(false)
                        .center()
                        .build();
                    }
                }
            }
            "reveal-data" => {
                let dir = app.state::<AppState>().data_dir.display().to_string();
                let _ = app.opener().open_path(dir, None::<&str>);
            }
            "settings" => {
                match app.get_webview_window("settings") {
                    Some(w) => {
                        let _ = w.set_focus();
                    }
                    None => {
                        let _ = WebviewWindowBuilder::new(
                            app,
                            "settings",
                            WebviewUrl::App("settings.html".into()),
                        )
                        .title("Settings — Metabase Desktop")
                        .inner_size(420.0, 400.0)
                        .resizable(false)
                        .center()
                        .build();
                    }
                }
            }
            "uninstall" => {
                let confirmed = app
                    .dialog()
                    .message("Delete ALL Metabase Desktop data (database, settings, logs) and remove this app?\nThis action cannot be undone.")
                    .title("Uninstall Metabase Desktop")
                    .kind(MessageDialogKind::Warning)
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "Delete Permanently".to_string(),
                        "Cancel".to_string(),
                    ))
                    .blocking_show();
                if confirmed {
                    let state = app.state::<AppState>();
                    let data_dir = state.data_dir.clone();
                    stop_backend(&state);
                    drop(state);
                    let _ = fs::remove_dir_all(&data_dir);
                    if let Ok(exe) = std::env::current_exe() {
                        if let Some(bundle) = exe
                            .ancestors()
                            .find(|p| p.extension().map_or(false, |e| e == "app"))
                        {
                            let _ = fs::remove_dir_all(bundle);
                        }
                    }
                    app.exit(0);
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                let state = app.state::<AppState>();
                stop_backend(&state);
            }
        });
}

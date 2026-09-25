// Metabase Desktop — wraps Metabase OSS JAR as a native desktop app.
// Flow: splash window -> spawn bundled JRE + metabase.jar -> poll /api/health
//       -> open main window at http://127.0.0.1:<port> -> kill Java on exit.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

const DEFAULT_PORT: u16 = 9010;
const DEFAULT_XMX: &str = "1g";
const METABASE_LATEST_URL: &str = "https://downloads.metabase.com/latest/metabase.jar";

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
            "# Metabase Desktop - config\n# Edit lalu pilih menu 'Restart Backend' untuk apply.\n\n# PORT=9010\n# XMX=1g\n",
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

fn java_bin(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(p) = app.path().resolve("resources/runtime/bin/java", BaseDirectory::Resource) {
        if p.exists() {
            return Some(p);
        }
    }
    // dev fallback: run via `just dev` from project root
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/runtime/bin/java");
    dev.exists().then_some(dev)
}

fn baked_jar(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(p) = app.path().resolve("resources/metabase.jar", BaseDirectory::Resource) {
        if p.exists() {
            return Some(p);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/metabase.jar");
    dev.exists().then_some(dev)
}

fn port_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
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
            "Port {port} sudah dipakai proses lain.\nUbah PORT di config.env (menu: Edit Settings) atau matikan prosesnya."
        );
        return Err(msg);
    }

    let java = java_bin(app)
        .ok_or_else(|| "Bundled JRE tidak ditemukan. Jalankan: just fetch-jre".to_string())?;
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
                .ok_or_else(|| "metabase.jar tidak ditemukan. Jalankan: just fetch-jar".to_string())?
        }
    };

    let logs_dir = data_dir.join("logs");
    let _ = fs::create_dir_all(&logs_dir);
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs_dir.join("metabase.log"))
        .map_err(|e| format!("Gagal buka log file: {e}"))?;
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
        .map_err(|e| format!("Gagal spawn Java: {e}"))?;

    *state.child.lock().unwrap() = Some(child);
    drop(state);

    let _ = app.emit("backend-status", "Menyalakan Metabase… biasanya 15–40 detik.");

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
                            "Proses Metabase mati mendadak.\nCek logs/metabase.log di data folder (menu: Reveal Data Folder).",
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
                                        "Metabase menginisialisasi database… {}s",
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
                    "Timeout: Metabase tidak siap dalam 3 menit.\nCek logs/metabase.log di data folder.",
                );
                return;
            }
            if last_status.elapsed() > Duration::from_secs(5) {
                last_status = Instant::now();
                let _ = handle.emit(
                    "backend-status",
                    format!("Masih menyalakan… {}s", started.elapsed().as_secs()),
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
            let _ = win.eval("location.reload()");
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
        format!("Menyalakan ulang Metabase (Xmx {xmx}, port {port})…"),
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
    let _ = app.emit("update-status", "Mengunduh metabase.jar terbaru…".to_string());

    let client = reqwest::Client::new();
    match client.get(METABASE_LATEST_URL).send().await {
        Ok(mut resp) => {
            let total = resp.content_length();
            let tmp = data_dir.join("metabase.jar.download");
            match File::create(&tmp) {
                Ok(mut file) => {
                    let mut received: u64 = 0;
                    let mut last_emit = Instant::now();
                    let mut ok = true;
                    loop {
                        match resp.chunk().await {
                            Ok(Some(chunk)) => {
                                if file.write_all(&chunk).is_err() {
                                    ok = false;
                                    break;
                                }
                                received += chunk.len() as u64;
                                if last_emit.elapsed() > Duration::from_millis(200) {
                                    last_emit = Instant::now();
                                    let _ = app.emit(
                                        "update-progress",
                                        json!({"received": received, "total": total}),
                                    );
                                }
                            }
                            Ok(None) => break,
                            Err(_) => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    let _ = app.emit(
                        "update-progress",
                        json!({"received": received, "total": total}),
                    );
                    if ok {
                        let final_path = data_dir.join("metabase.jar");
                        let _ = fs::remove_file(&final_path);
                        if fs::rename(&tmp, &final_path).is_ok() {
                            let _ = app.emit(
                                "update-status",
                                "Selesai didownload. Restart backend dengan versi baru…".to_string(),
                            );
                            // stop_backend pake std sleep — jangan blok async worker
                            let app2 = app.clone();
                            let _ = tauri::async_runtime::spawn_blocking(move || {
                                restart_backend(&app2)
                            });
                            return;
                        }
                    }
                }
                Err(e) => {
                    let _ = app.emit("update-status", format!("Gagal tulis file: {e}"));
                }
            }
        }
        Err(e) => {
            let _ = app.emit("update-status", format!("Gagal download: {e}"));
        }
    }
    let _ = fs::remove_file(data_dir.join("metabase.jar.download"));
    let _ = app.emit(
        "update-status",
        "Update gagal. Metabase lanjut jalan pakai versi lama.".to_string(),
    );
    if let Some(w) = app.get_webview_window("updater") {
        let _ = w.close();
    }
}

// ---------- commands ----------

#[tauri::command]
fn retry_backend(app: AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = handle.emit("backend-status", "Menyalakan ulang Metabase…");
        if let Err(e) = spawn_backend(&handle) {
            let _ = handle.emit("backend-error", e);
        }
    });
}

// ---------- main ----------

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![retry_backend])
        .setup(|app| {
            let data_dir = app.path().app_data_dir().expect("no app data dir");
            fs::create_dir_all(&data_dir).expect("cannot create data dir");
            let (port, xmx) = load_config(&data_dir);

            // --- native menu (macOS) ---
            let about =
                PredefinedMenuItem::about(app, Some("Tentang Metabase Desktop"), None::<AboutMetadata>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let check = MenuItem::with_id(app, "check-updates", "Cek Update Metabase…", true, None::<&str>)?;
            let restart = MenuItem::with_id(app, "restart-backend", "Restart Backend", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "edit-settings", "Edit Settings…", true, None::<&str>)?;
            let reveal = MenuItem::with_id(app, "reveal-data", "Reveal Data Folder", true, None::<&str>)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let quit = PredefinedMenuItem::quit(app, Some("Keluar"))?;
            let app_menu = Submenu::with_items(
                app,
                "Metabase Desktop",
                true,
                &[&about, &sep1, &check, &restart, &settings, &reveal, &sep2, &quit],
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

            app.manage(AppState {
                child: Mutex::new(None),
                data_dir,
                port: Mutex::new(port),
                xmx: Mutex::new(xmx),
            });

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = handle.emit("backend-status", "Menyiapkan environment…");
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
                        .title("Update Metabase")
                        .inner_size(420.0, 180.0)
                        .resizable(false)
                        .center()
                        .build();
                    }
                    run_update(a).await;
                });
            }
            "restart-backend" => restart_backend(app),
            "reveal-data" => {
                let dir = app.state::<AppState>().data_dir.display().to_string();
                let _ = app.opener().open_path(dir, None::<&str>);
            }
            "edit-settings" => {
                let cfg = app.state::<AppState>().data_dir.join("config.env");
                let _ = Command::new("open").arg("-t").arg(&cfg).spawn();
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

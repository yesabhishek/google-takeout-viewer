mod api;
mod db;
mod devserver;
mod export;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "windows")]
mod win;
mod meta;
mod protocol;
mod scan;
mod state;
mod takeout;
mod thumbs;
mod video;

use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Emitter, Manager};

use state::AppState;

#[tauri::command]
async fn api(state: tauri::State<'_, Arc<AppState>>, name: String, args: Value) -> Result<Value, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match api::dispatch(&st, &name, args)? {
        api::Out::Json(v) => Ok(v),
        api::Out::Bytes(_) => Err("binary result: use api_bin".into()),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn api_bin(
    state: tauri::State<'_, Arc<AppState>>,
    name: String,
    args: Value,
) -> Result<tauri::ipc::Response, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match api::dispatch(&st, &name, args)? {
        api::Out::Bytes(b) => Ok(tauri::ipc::Response::new(b)),
        api::Out::Json(v) => Ok(tauri::ipc::Response::new(serde_json::to_vec(&v).unwrap_or_default())),
    })
    .await
    .map_err(|e| e.to_string())?
}

fn default_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/com.yesabhishek.takeoutviewer")
    } else {
        home.join(".local/share/com.yesabhishek.takeoutviewer")
    }
}

/// Headless modes, handy for very large libraries and for development:
///   takeout-viewer index <folder> [--data DIR] [--thumbs]
///   takeout-viewer serve [--data DIR] [--port 1421] [--source <folder>]
fn cli(args: &[String]) -> Option<i32> {
    let cmd = args.get(1)?.as_str();
    if cmd != "index" && cmd != "serve" {
        return None;
    }
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let data = flag("--data").map(PathBuf::from).unwrap_or_else(default_data_dir);
    let state = match AppState::open(&data) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return Some(1);
        }
    };
    state.set_emitter(Box::new(|name, payload| {
        if name == "scan-progress" {
            if let Some(msg) = payload.get("message").and_then(|m| m.as_str()) {
                let done = payload.get("done").and_then(|v| v.as_u64()).unwrap_or(0);
                let total = payload.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                eprint!("\r\x1b[2K{msg} ({done}/{total})");
            }
        }
    }));
    let source = if cmd == "index" {
        args.get(2).filter(|a| !a.starts_with("--")).cloned()
    } else {
        flag("--source")
    };
    if let Some(src) = source {
        let p = PathBuf::from(&src).canonicalize().unwrap_or(PathBuf::from(&src));
        let _ = state.db.write(|c| {
            c.execute(
                "INSERT INTO sources(path, added_at) VALUES(?1, ?2) ON CONFLICT(path) DO NOTHING",
                rusqlite::params![p.to_string_lossy(), scan::now()],
            )
        });
        let t = std::time::Instant::now();
        if let Err(e) = scan::run_blocking(&state) {
            eprintln!("\nscan failed: {e}");
            return Some(1);
        }
        eprintln!("\nindexed in {:.2}s", t.elapsed().as_secs_f64());
    }
    match cmd {
        "index" => {
            if let Ok(api::Out::Json(v)) = api::dispatch(&state, "status", Value::Null) {
                println!("{}", serde_json::to_string_pretty(&v["counts"]).unwrap_or_default());
            }
            if args.iter().any(|a| a == "--thumbs") {
                let t = std::time::Instant::now();
                state.queue_all_thumbs();
                loop {
                    let (done, total, _) = state.thumbs.progress();
                    eprint!("\r\x1b[2Kthumbnails {done}/{total}");
                    if done >= total {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                eprintln!("\nthumbnails in {:.2}s", t.elapsed().as_secs_f64());
            }
            Some(0)
        }
        _ => {
            let port = flag("--port").and_then(|p| p.parse().ok()).unwrap_or(1421);
            state.queue_all_thumbs();
            match devserver::serve(state, port) {
                Ok(()) => Some(0),
                Err(e) => {
                    eprintln!("error: {e}");
                    Some(1)
                }
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = cli(&args) {
        std::process::exit(code);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("gp", |ctx, request, responder| {
            let Some(state) = ctx.app_handle().try_state::<Arc<AppState>>().map(|s| s.inner().clone()) else {
                responder.respond(
                    tauri::http::Response::builder().status(503).body(Vec::new()).unwrap(),
                );
                return;
            };
            let path = request.uri().path().to_string();
            let range = request
                .headers()
                .get("range")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            tauri::async_runtime::spawn_blocking(move || {
                protocol::handle(
                    state,
                    &path,
                    range,
                    Box::new(move |resp| {
                        let (status, headers, body) = resp.into_vec();
                        let mut b = tauri::http::Response::builder()
                            .status(status)
                            .header("Access-Control-Allow-Origin", "*");
                        for (k, v) in headers {
                            b = b.header(k, v);
                        }
                        responder.respond(b.body(body).unwrap());
                    }),
                );
            });
        })
        .setup(|app| {
            // TAKEOUT_VIEWER_DATA lets you keep separate libraries (or test data).
            let dir = match std::env::var_os("TAKEOUT_VIEWER_DATA") {
                Some(d) => PathBuf::from(d),
                None => app.path().app_data_dir()?,
            };
            let state = AppState::open(&dir).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let handle = app.handle().clone();
            state.set_emitter(Box::new(move |name, payload| {
                let _ = handle.emit(name, payload);
            }));
            app.manage(state.clone());
            std::thread::spawn(move || state.queue_all_thumbs());
            // Development aid: run a script inside the real web view, e.g. to
            // check media playback in WebKit (results come back via `log`).
            if let Some(probe) = std::env::var_os("TAKEOUT_VIEWER_PROBE") {
                if let Some(w) = app.get_webview_window("main") {
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(3));
                        if let Ok(js) = std::fs::read_to_string(probe) {
                            let _ = w.eval(&js);
                        }
                    });
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![api, api_bin])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

//! Local HTTP bridge exposing the same API and media routes, so the UI can be
//! developed and tested in a normal browser: `takeout-viewer serve`.
//! Binds to 127.0.0.1 and only answers the dev UI, so other web pages open in
//! the same browser can't read your photos through it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use tiny_http::{Header, Method, Response, Server, StatusCode};

use crate::api::{self, Out};
use crate::diskio::{Prio, DRIVE};
use crate::protocol::{self, Body};
use crate::state::AppState;

const DEV_ORIGINS: &[&str] = &["http://localhost:1420", "http://127.0.0.1:1420"];

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("valid header")
}

pub fn serve(state: Arc<AppState>, port: u16) -> Result<(), String> {
    let server = Server::http(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    eprintln!("dev bridge listening on http://127.0.0.1:{port}");
    for mut req in server.incoming_requests() {
        let state = state.clone();
        std::thread::spawn(move || {
            // Browsers send Origin on cross-origin requests; anything that isn't
            // the dev UI is refused. Media elements send none, which is fine.
            let origin = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Origin"))
                .map(|h| h.value.as_str().to_string());
            let allowed = match &origin {
                None => true,
                Some(o) => DEV_ORIGINS.contains(&o.as_str()),
            };
            if !allowed {
                let _ = req.respond(Response::empty(403));
                return;
            }
            let cors = [
                header("Access-Control-Allow-Origin", origin.as_deref().unwrap_or(DEV_ORIGINS[0])),
                header("Access-Control-Allow-Headers", "*"),
                header("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
            ];
            if *req.method() == Method::Options {
                let mut r = Response::empty(204);
                for h in cors {
                    r.add_header(h);
                }
                let _ = req.respond(r);
                return;
            }
            let url = req.url().to_string();
            if let Some(name) = url.strip_prefix("/api/") {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let args: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::json!({}));
                let (status, ctype, bytes) = match api::dispatch(&state, name, args) {
                    Ok(Out::Json(v)) => (200, "application/json", serde_json::to_vec(&v).unwrap_or_default()),
                    Ok(Out::Bytes(b)) => (200, "application/octet-stream", b),
                    Err(e) => (500, "text/plain", e.into_bytes()),
                };
                let mut r = Response::from_data(bytes).with_status_code(status);
                r.add_header(header("Content-Type", ctype));
                for h in cors {
                    r.add_header(h);
                }
                let _ = req.respond(r);
                return;
            }
            if let Some(path) = url.strip_prefix("/gp") {
                let range = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Range"))
                    .map(|h| h.value.as_str().to_string());
                let (tx, rx) = std::sync::mpsc::channel();
                protocol::handle(state, path, range, Box::new(move |r| {
                    let _ = tx.send(r);
                }));
                let Ok(resp) = rx.recv() else { return };
                let mut headers: Vec<Header> = resp
                    .headers
                    .iter()
                    .filter(|(k, _)| *k != "Content-Length")
                    .map(|(k, v)| header(k, v))
                    .collect();
                headers.extend(cors);
                let status = StatusCode(resp.status);
                let _ = match resp.body {
                    Body::Bytes(b) => {
                        let len = b.len();
                        req.respond(Response::new(status, headers, std::io::Cursor::new((*b).clone()), Some(len), None))
                    }
                    Body::File { path, start, len } => match File::open(&path) {
                        Ok(mut f) => {
                            let _drive = DRIVE.acquire(Prio::Interactive);
                            let _ = f.seek(SeekFrom::Start(start));
                            req.respond(Response::new(status, headers, f.take(len), Some(len as usize), None))
                        }
                        Err(_) => req.respond(Response::empty(404)),
                    },
                };
                return;
            }
            let _ = req.respond(Response::empty(404));
        });
    }
    Ok(())
}

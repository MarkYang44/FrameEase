//! Debug-only HTTP bridge (cargo feature `dev-bridge`, debug builds only).
//!
//! Serves the window's commands and events on 127.0.0.1:1430 so the very same
//! UI, opened in a regular browser with `?bridge`, drives the real backend.
//! Used for automated UI checks where OS screenshots are unavailable.

use crate::{commands as c, AppState};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager};

pub const ADDR: &str = "127.0.0.1:1430";

struct Events {
    seq: u64,
    items: VecDeque<(u64, String, Value)>,
}

fn events() -> &'static Mutex<Events> {
    static E: OnceLock<Mutex<Events>> = OnceLock::new();
    E.get_or_init(|| Mutex::new(Events { seq: 0, items: VecDeque::new() }))
}

pub fn record<T: Serialize + ?Sized>(event: &str, payload: &T) {
    let mut g = events().lock().unwrap_or_else(|e| e.into_inner());
    g.seq += 1;
    let seq = g.seq;
    g.items.push_back((seq, event.to_string(), serde_json::to_value(payload).unwrap_or(Value::Null)));
    while g.items.len() > 4000 {
        g.items.pop_front();
    }
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let Ok(server) = tiny_http::Server::http(ADDR) else {
            eprintln!("dev-bridge: cannot bind {ADDR}");
            return;
        };
        eprintln!("dev-bridge: listening on http://{ADDR}");
        for req in server.incoming_requests() {
            let app = app.clone();
            std::thread::spawn(move || handle(app, req));
        }
    });
}

fn header(k: &str, v: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("valid header")
}

fn respond(req: tiny_http::Request, status: u16, body: Vec<u8>, content_type: &str) {
    let resp = tiny_http::Response::from_data(body)
        .with_status_code(status)
        .with_header(header("Content-Type", content_type))
        .with_header(header("Access-Control-Allow-Origin", "*"))
        .with_header(header("Access-Control-Allow-Methods", "GET, POST, OPTIONS"))
        .with_header(header("Access-Control-Allow-Headers", "content-type"));
    let _ = req.respond(resp);
}

fn query(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn handle(app: AppHandle, mut req: tiny_http::Request) {
    let url = req.url().to_string();
    if req.method() == &tiny_http::Method::Options {
        return respond(req, 204, vec![], "text/plain");
    }
    if url.starts_with("/events") {
        let since: u64 = query(&url, "since").and_then(|s| s.parse().ok()).unwrap_or(0);
        let g = events().lock().unwrap_or_else(|e| e.into_inner());
        let list: Vec<Value> = g.items.iter().filter(|(s, _, _)| *s > since).map(|(_, e, p)| json!({ "event": e, "payload": p })).collect();
        let body = json!({ "seq": g.seq, "events": list });
        drop(g);
        return respond(req, 200, body.to_string().into_bytes(), "application/json");
    }
    if url.starts_with("/file") {
        let path = query(&url, "path").unwrap_or_default();
        let allowed = app.state::<AppState>().paths.get().map(|p| std::path::Path::new(&path).starts_with(&p.previews)).unwrap_or(false);
        if !allowed {
            return respond(req, 403, b"forbidden".to_vec(), "text/plain");
        }
        let ct = if path.ends_with(".png") { "image/png" } else { "application/octet-stream" };
        return match std::fs::read(&path) {
            Ok(b) => respond(req, 200, b, ct),
            Err(_) => respond(req, 404, vec![], "text/plain"),
        };
    }
    if let Some(cmd) = url.strip_prefix("/invoke/") {
        let cmd = cmd.split('?').next().unwrap_or("").to_string();
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let args: Value = serde_json::from_str(&body).unwrap_or(json!({}));
        let out = tauri::async_runtime::block_on(dispatch(&app, &cmd, args));
        let body = match out {
            Ok(v) => json!({ "ok": true, "data": v }),
            Err(e) => json!({ "ok": false, "error": e }),
        };
        return respond(req, 200, body.to_string().into_bytes(), "application/json");
    }
    respond(req, 404, vec![], "text/plain");
}

fn arg<T: DeserializeOwned>(a: &Value, k: &str) -> Result<T, String> {
    serde_json::from_value(a.get(k).cloned().unwrap_or(Value::Null)).map_err(|e| format!("{k}: {e}"))
}

fn to<T: Serialize>(r: Result<T, String>) -> Result<Value, String> {
    r.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

async fn dispatch(app: &AppHandle, cmd: &str, a: Value) -> Result<Value, String> {
    let st = || app.state::<AppState>();
    match cmd {
        "bootstrap" => to(c::bootstrap(app.clone(), st()).await),
        "set_ffmpeg_dir" => to(c::set_ffmpeg_dir(app.clone(), st(), arg(&a, "dir")?).await),
        "redetect_caps" => to(c::redetect_caps(st()).await),
        "describe_settings" => to(c::describe_settings(st(), arg(&a, "settings")?).await),
        "probe_files" => to(c::probe_files(st(), arg(&a, "paths")?).await),
        "scan_inputs" => to(c::scan_inputs(st(), arg(&a, "paths")?, arg(&a, "recursive")?).await),
        "plan_preview" => to(c::plan_preview(st(), arg(&a, "items")?).await),
        "settings_apply_fix" => to(c::settings_apply_fix(arg(&a, "settings")?, arg(&a, "fix")?).await),
        "enqueue" => to(c::enqueue(st(), arg(&a, "specs")?).await),
        "queue_snapshot" => to(c::queue_snapshot(st()).await),
        "queue_set_paused" => to(c::queue_set_paused(st(), arg(&a, "paused")?).await),
        "queue_cancel" => to(c::queue_cancel(st(), arg(&a, "id")?).await),
        "queue_retry" => to(c::queue_retry(st(), arg(&a, "id")?).await),
        "queue_remove" => to(c::queue_remove(st(), arg(&a, "ids")?).await),
        "queue_move" => to(c::queue_move(st(), arg(&a, "id")?, arg(&a, "to")?).await),
        "queue_clear_finished" => to(c::queue_clear_finished(st()).await),
        "queue_resolve" => to(c::queue_resolve(st(), arg(&a, "id")?, arg(&a, "fix")?).await),
        "queue_set_concurrency" => to(c::queue_set_concurrency(st(), arg(&a, "concurrency")?).await),
        "manifest_flush" => to(c::manifest_flush(st()).await),
        "read_log" => to(c::read_log(st(), arg(&a, "id")?).await),
        "config_get" => to(c::config_get(st()).await),
        "config_set" => to(c::config_set(st(), arg(&a, "config")?).await),
        "presets_list" => to(c::presets_list(st()).await),
        "preset_save" => to(c::preset_save(st(), arg(&a, "name")?, arg(&a, "settings")?).await),
        "preset_delete" => to(c::preset_delete(st(), arg(&a, "id")?).await),
        "trial_run" => to(c::trial_run(st(), arg(&a, "args")?).await),
        "trial_cancel" => to(c::trial_cancel(st()).await),
        "extract_frame" => to(c::extract_frame(st(), arg(&a, "path")?, arg(&a, "time")?, arg(&a, "maxWidth")?, arg(&a, "tag")?).await),
        "keyframes" => to(c::keyframes(st(), arg(&a, "path")?).await),
        // Revealing/opening files and quitting act on the desktop session; not bridged.
        other => Err(format!("not available over the dev bridge: {other}")),
    }
}

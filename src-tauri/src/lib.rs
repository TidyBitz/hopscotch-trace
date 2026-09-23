//! The desktop shell: Tauri commands and events over hopscotch-core.
//!
//! Everything blocking runs on worker threads so the webview stays smooth.
//! Traces and probers are keyed by an id the UI chooses, so several tabs can
//! trace and probe at once.

use hopscotch_core::{dns, geo, netinfo, ping, rdap, trace};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
struct Registry {
    traces: Mutex<HashMap<String, trace::Cancel>>,
    probers: Mutex<HashMap<String, ping::Prober>>,
}

#[derive(Serialize, Clone)]
struct TraceMsg {
    id: String,
    #[serde(flatten)]
    event: trace::TraceEvent,
}

#[derive(Serialize, Clone)]
struct NameMsg {
    id: String,
    ip: String,
    name: Option<String>,
}

#[derive(Serialize, Clone)]
struct SampleMsg {
    id: String,
    #[serde(flatten)]
    sample: ping::Sample,
}

#[derive(Serialize)]
struct Platform {
    os: String,
    arch: String,
    tool: Option<String>,
    tool_error: Option<String>,
    version: String,
}

#[tauri::command]
fn platform() -> Platform {
    let (tool, tool_error) = match trace::pick_tool("example.org", &trace::TraceOpts::default()) {
        Ok((t, _)) => (Some(format!("{} {}", t.program, t.args[..t.args.len() - 1].join(" "))), None),
        Err(e) => (None, Some(e)),
    };
    Platform { os: std::env::consts::OS.into(), arch: std::env::consts::ARCH.into(), tool, tool_error, version: env!("CARGO_PKG_VERSION").into() }
}

#[tauri::command]
async fn resolve(target: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || dns::resolve(&target).map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn reverse(ip: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || dns::reverse(&ip)).await.ok().flatten()
}

/// Start a trace. Hops arrive as `trace:event` events carrying `id`; reverse
/// names for each answering hop follow as `trace:name` events.
#[tauri::command]
fn start_trace(app: AppHandle, reg: State<'_, Registry>, id: String, target: String, max_hops: Option<u8>, wait_ms: Option<u32>) -> Result<(), String> {
    let cancel = trace::Cancel::default();
    if let Some(old) = reg.traces.lock().unwrap().insert(id.clone(), cancel.clone()) {
        old.cancel();
    }
    let opts = trace::TraceOpts { max_hops: max_hops.unwrap_or(30).clamp(1, 64), wait_ms: wait_ms.unwrap_or(1500).clamp(200, 10_000), queries: 3 };
    std::thread::Builder::new()
        .name(format!("trace-{id}"))
        .spawn(move || {
            let app2 = app.clone();
            let id2 = id.clone();
            let c2 = cancel.clone();
            trace::run(&target, &opts, &cancel, move |ev| {
                if let trace::TraceEvent::Hop(h) = &ev {
                    if let Some(ip) = h.ip() {
                        // reverse lookups are slow and independent: one thread each, results as events
                        let ip = ip.to_string();
                        let app3 = app2.clone();
                        let id3 = id2.clone();
                        let c3 = c2.clone();
                        std::thread::spawn(move || {
                            let name = dns::reverse(&ip);
                            if !c3.is_cancelled() {
                                let _ = app3.emit("trace:name", NameMsg { id: id3, ip, name });
                            }
                        });
                    }
                }
                let _ = app2.emit("trace:event", TraceMsg { id: id2.clone(), event: ev });
            });
            if let Some(reg) = app.try_state::<Registry>() {
                reg.traces.lock().unwrap().remove(&id);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn cancel_trace(reg: State<'_, Registry>, id: String) {
    if let Some(c) = reg.traces.lock().unwrap().remove(&id) {
        c.cancel();
    }
}

#[tauri::command]
fn start_probe(app: AppHandle, reg: State<'_, Registry>, id: String, ips: Vec<String>, interval_ms: Option<u64>, timeout_ms: Option<u32>) {
    let interval = Duration::from_millis(interval_ms.unwrap_or(1000).clamp(200, 600_000));
    let timeout = timeout_ms.unwrap_or(1500).clamp(200, 10_000);
    let id2 = id.clone();
    let prober = ping::Prober::start(ips, interval, timeout, move |s| {
        let _ = app.emit("probe:sample", SampleMsg { id: id2.clone(), sample: s });
    });
    reg.probers.lock().unwrap().insert(id, prober); // dropping a previous one stops it
}

#[tauri::command]
fn stop_probe(reg: State<'_, Registry>, id: String) {
    reg.probers.lock().unwrap().remove(&id);
}

#[tauri::command]
async fn geo_lookup(ips: Vec<String>) -> Result<Vec<geo::GeoInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || geo::lookup(&ips).map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn geo_self() -> Result<geo::GeoInfo, String> {
    tauri::async_runtime::spawn_blocking(|| geo::lookup_self().map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn rdap_ip(ip: String) -> Result<rdap::IpRecord, String> {
    tauri::async_runtime::spawn_blocking(move || rdap::ip(&ip).map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn rdap_domain(name: String) -> Result<rdap::DomainRecord, String> {
    tauri::async_runtime::spawn_blocking(move || rdap::domain(&rdap::registrable(&name)).map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_text(path: String, text: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || std::fs::write(&path, text.as_bytes()).map_err(|e| format!("{path}: {e}"))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn net_info() -> netinfo::NetInfo {
    tauri::async_runtime::spawn_blocking(netinfo::current).await.unwrap_or_default()
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Registry::default())
        .invoke_handler(tauri::generate_handler![
            platform, resolve, reverse, start_trace, cancel_trace, start_probe, stop_probe, geo_lookup, geo_self, rdap_ip, rdap_domain, net_info, save_text
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hopscotch");
}

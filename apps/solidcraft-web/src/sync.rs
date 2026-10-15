//! Keeping the stored designs on the SolidCraft design server (`deploy/store`), when there is one.
//!
//! The browser's private storage stays the working copy; the server mirrors it. A save is sent to
//! the server, and at start (and every half minute) the app fetches designs the server has and
//! the browser lacks or has older. Where the page has no `/api` (a static host, `trunk serve`)
//! nothing here does anything. The rules for a design that differs are in `plan.rs`.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use js_sys::Uint8Array;
use solidcraft_engine::io::{MAX_DESIGN_BYTES, vfs};
use wasm_bindgen::{JsCast as _, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::files::{self, Entry, Files, OPFS};
use crate::plan::{self, Action};

/// Where the app remembers, per design, `(server modified, browser modified)` at the last sync.
const STATE_KEY: &str = "solidcraft.sync";
const EVERY_MS: i32 = 30_000;

thread_local! {
    /// The page has a design server (it answered).
    static ON: Cell<bool> = const { Cell::new(false) };
    /// The page has no design server: stop asking.
    static ABSENT: Cell<bool> = const { Cell::new(false) };
    /// A full sync is running.
    static BUSY: Cell<bool> = const { Cell::new(false) };
    /// The last attempt failed (say so once, not every half minute).
    static FAILING: Cell<bool> = const { Cell::new(false) };
}

struct Reply {
    status: u16,
    body: Vec<u8>,
}

async fn request(method: &str, path: &str, body: Option<&[u8]>) -> Result<Reply, String> {
    let opts = web_sys::RequestInit::new();
    opts.set_method(method);
    opts.set_cache(web_sys::RequestCache::NoStore);
    let headers = web_sys::Headers::new().map_err(files::err)?;
    // The server only takes changes that carry this (a page on another origin cannot add it).
    headers.set("X-Requested-With", "solidcraft").map_err(files::err)?;
    opts.set_headers(&headers);
    if let Some(b) = body {
        opts.set_body(&Uint8Array::from(b));
    }
    let req = web_sys::Request::new_with_str_and_init(path, &opts).map_err(files::err)?;
    let window = web_sys::window().ok_or("no window")?;
    let resp: web_sys::Response =
        JsFuture::from(window.fetch_with_request(&req)).await.map_err(files::err)?.dyn_into().map_err(|_| "no response".to_string())?;
    let status = resp.status();
    let buf = JsFuture::from(resp.array_buffer().map_err(files::err)?).await.map_err(files::err)?;
    Ok(Reply { status, body: Uint8Array::new(&buf).to_vec() })
}

fn file_url(name: &str) -> String {
    format!("api/files/{}", js_sys::encode_uri_component(name))
}

struct Remote {
    modified: u64,
}

/// The server's designs. `Ok(None)`: this page has no design server.
async fn remote_list() -> Result<Option<BTreeMap<String, Remote>>, String> {
    let r = request("GET", "api/files", None).await?;
    if r.status != 200 {
        return Ok(None);
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&r.body) else { return Ok(None) };
    let Some(list) = v.as_array() else { return Ok(None) };
    let mut out = BTreeMap::new();
    for e in list {
        if let (Some(name), Some(modified)) = (e["name"].as_str(), e["modified"].as_u64()) {
            out.insert(name.to_string(), Remote { modified });
        }
    }
    Ok(Some(out))
}

type State = BTreeMap<String, (u64, u64)>;

fn load_state() -> State {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(STATE_KEY).ok().flatten())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Remember (or forget) what the two sides looked like when they were last the same. Read fresh
/// each time: a save and a full sync may both be updating it.
fn set_synced(name: &str, to: Option<(u64, u64)>) {
    let mut state = load_state();
    match to {
        Some(t) => state.insert(name.to_string(), t),
        None => state.remove(name),
    };
    if let (Some(storage), Ok(text)) = (web_sys::window().and_then(|w| w.local_storage().ok().flatten()), serde_json::to_string(&state)) {
        let _ = storage.set_item(STATE_KEY, &text);
    }
}

fn is_internal(name: &str) -> bool {
    files::is_recovery_file(name)
}

fn report(files: &Rc<Files>, e: &str) {
    if !FAILING.replace(true) {
        files.say(format!("The design server can't be reached ({e}); designs stay in this browser until it can."), true);
    }
}

/// Store a design the server sent (or a copy made here) in the browser, as a synced design.
async fn store_local(files: &Rc<Files>, name: &str, bytes: Vec<u8>, remote_modified: Option<u64>) -> Result<(), String> {
    files::write_file(name, &bytes).await?;
    let size = bytes.len() as u64;
    vfs::insert(&format!("{OPFS}{name}"), bytes);
    let modified = files::stored_modified(name).await.unwrap_or_else(js_sys::Date::now);
    {
        let mut e = files.entries.borrow_mut();
        e.retain(|x| x.name != name);
        e.push(Entry { name: name.to_string(), size, modified });
        e.sort_by_key(|a| a.name.to_lowercase());
    }
    if let Some(rm) = remote_modified {
        set_synced(name, Some((rm, modified as u64)));
    }
    files.wake();
    Ok(())
}

async fn download(name: &str) -> Result<Vec<u8>, String> {
    let r = request("GET", &file_url(name), None).await?;
    if r.status != 200 {
        return Err(format!("{name}: the server answered {}", r.status));
    }
    if r.body.len() > MAX_DESIGN_BYTES {
        return Err(format!("{name}: larger than a design can be"));
    }
    Ok(r.body)
}

async fn pull(files: &Rc<Files>, name: &str, remote_modified: u64) -> Result<(), String> {
    let bytes = download(name).await?;
    store_local(files, name, bytes, Some(remote_modified)).await
}

/// Send `bytes` as the design `name`; the server keeps the version it replaces.
async fn push_bytes(name: &str, bytes: &[u8]) -> Result<(), String> {
    let modified = files::stored_modified(name).await.unwrap_or_else(js_sys::Date::now) as u64;
    let r = request("PUT", &format!("{}?modified={modified}", file_url(name)), Some(bytes)).await?;
    if r.status != 200 {
        return Err(format!("{name}: the server answered {} ({})", r.status, String::from_utf8_lossy(&r.body)));
    }
    let remote = serde_json::from_slice::<serde_json::Value>(&r.body).ok().and_then(|v| v["modified"].as_u64()).unwrap_or(modified);
    set_synced(name, Some((remote, modified)));
    Ok(())
}

async fn push(name: &str) -> Result<(), String> {
    let bytes = vfs::read(&format!("{OPFS}{name}"), MAX_DESIGN_BYTES).map_err(|e| e.to_string())?;
    push_bytes(name, &bytes).await
}

async fn delete_remote(name: &str) -> Result<(), String> {
    let r = request("DELETE", &file_url(name), None).await?;
    // 404: already gone.
    if r.status == 204 || r.status == 200 || r.status == 404 {
        set_synced(name, None);
        Ok(())
    } else {
        Err(format!("{name}: the server answered {}", r.status))
    }
}

/// A design was saved in the browser: send it to the server.
pub async fn after_write(files: &Rc<Files>, name: &str, bytes: &[u8]) {
    if !ON.with(Cell::get) || is_internal(name) || !plan::syncable(name) {
        return;
    }
    match push_bytes(name, bytes).await {
        Ok(()) => FAILING.set(false),
        Err(e) => report(files, &e),
    }
}

/// A design was deleted from the browser: delete it on the server (its versions stay there).
pub async fn after_delete(files: &Rc<Files>, name: &str) {
    if !ON.with(Cell::get) || is_internal(name) || !plan::syncable(name) {
        return;
    }
    match delete_remote(name).await {
        Ok(()) => FAILING.set(false),
        // The sync at the next start sees the design gone here and unchanged there, and retries.
        Err(e) => report(files, &e),
    }
}

/// Bring the two sides together. `Ok(None)`: this page has no design server.
async fn sync_all(files: &Rc<Files>) -> Result<Option<usize>, String> {
    // Without the browser's private file system there is no reliable "modified" to compare.
    if files::local_only() {
        return Ok(None);
    }
    let Some(remote) = remote_list().await? else { return Ok(None) };
    ON.set(true);
    let local: BTreeMap<String, Entry> = files.entries.borrow().iter().map(|e| (e.name.clone(), e.clone())).collect();
    let state = load_state();
    let names: BTreeSet<&String> = local.keys().chain(remote.keys()).chain(state.keys()).collect();
    let mut pulled = 0;
    for name in names {
        if is_internal(name) || !plan::syncable(name) {
            continue;
        }
        let l = local.get(name).map(|e| e.modified as u64);
        let r = remote.get(name).map(|e| e.modified);
        let synced = load_state().get(name).copied();
        match plan::decide(l, r, synced) {
            Action::Nothing => {}
            Action::Pull => {
                if let Some(rm) = r {
                    pull(files, name, rm).await?;
                    pulled += 1;
                }
            }
            Action::Push => push(name).await?,
            Action::DeleteRemote => delete_remote(name).await?,
            Action::DeleteLocal => {
                set_synced(name, None);
                files::delete_local(files, name).await;
            }
            Action::Conflict => {
                let (Some(lm), Some(rm)) = (l, r) else { continue };
                let theirs = download(name).await?;
                let mine = vfs::read(&format!("{OPFS}{name}"), MAX_DESIGN_BYTES).map_err(|e| e.to_string())?;
                if theirs == mine {
                    // The same design on both sides (a first contact): nothing to move.
                    set_synced(name, Some((rm, lm)));
                } else if lm >= rm {
                    // This browser's is newer: keep the server's as a copy here, send this one.
                    let copy = copy_name(name, "server copy");
                    store_local(files, &copy, theirs, None).await?;
                    push(name).await?;
                    files.say(format!("{name} changed here and on the server. This browser's is newer and was sent; the server's is kept as {copy}."), false);
                } else {
                    // The server's is newer: keep this browser's as a copy, take the server's.
                    let copy = copy_name(name, "this browser");
                    store_local(files, &copy, mine, None).await?;
                    store_local(files, name, theirs, Some(rm)).await?;
                    pulled += 1;
                    files.say(format!("{name} changed here and on the server. The server's is newer and was taken; this browser's is kept as {copy}."), false);
                }
            }
        }
    }
    Ok(Some(pulled))
}

/// A name for a copy of `name` that no stored design has yet.
fn copy_name(name: &str, label: &str) -> String {
    let path = files::free_name(&plan::labelled(name, label));
    path.strip_prefix(OPFS).unwrap_or(&path).to_string()
}

/// One round (skipped while another runs, or when the tab is in the background).
async fn round(files: &Rc<Files>) {
    if ABSENT.with(Cell::get) || BUSY.replace(true) {
        return;
    }
    let result = sync_all(files).await;
    BUSY.set(false);
    match result {
        Ok(Some(pulled)) => {
            FAILING.set(false);
            if pulled > 0 {
                files.say(format!("Updated {pulled} design(s) from the design server."), false);
            }
        }
        Ok(None) => ABSENT.set(true),
        Err(e) => report(files, &e),
    }
}

/// Start syncing (after the browser's designs are loaded): now, then every half minute.
pub fn start(files: &Rc<Files>) {
    let first = files.clone();
    spawn_local(async move { round(&first).await });
    let Some(window) = web_sys::window() else { return };
    let files = files.clone();
    let tick = Closure::<dyn FnMut()>::new(move || {
        if web_sys::window().and_then(|w| w.document()).is_some_and(|d| d.hidden()) {
            return;
        }
        let files = files.clone();
        spawn_local(async move { round(&files).await });
    });
    let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(tick.as_ref().unchecked_ref(), EVERY_MS);
    tick.forget();
}

//! SolidCraft design store: the small server behind the web app's "keep my designs on the server".
//!
//! It is for one person, on a private network (localhost or Tailscale), so there are no accounts.
//! What it does guard against is a web page in the same browser writing to it: every change needs
//! the `X-Requested-With: solidcraft` header, which a page on another origin cannot send without
//! the server's consent (there is no CORS).
//!
//! ```text
//! GET    /api/health
//! GET    /api/files                          [{name, size, modified}]
//! GET    /api/files/{name}                   the design
//! PUT    /api/files/{name}?modified=<ms>     store it (the old version goes to the history)
//! DELETE /api/files/{name}                   remove it (its versions stay)
//! GET    /api/files/{name}/versions          [{ts, size}]
//! GET    /api/files/{name}/versions/{ts}     an earlier version
//! GET    /library                            every design and version, to browse and download
//! ```
//!
//! Environment: `STORE_ADDR` (0.0.0.0:8081), `STORE_DIR` (/data), `STORE_MAX_BYTES` (256 MiB, the
//! app's own limit), `STORE_KEEP_VERSIONS` (100 per design).

mod library;
mod store;

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use store::{Store, StoreError};

type Shared = Arc<Store>;

/// A store call off the async threads (file reads and writes block).
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, StoreError> + Send + 'static) -> Result<T, StoreError> {
    tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| Err(StoreError::Io(std::io::Error::other(e.to_string()))))
}

struct ApiError(StoreError);

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, text) = match self.0 {
            StoreError::BadName => (StatusCode::BAD_REQUEST, "a design name is one plain file name"),
            StoreError::NotFound => (StatusCode::NOT_FOUND, "not found"),
            StoreError::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "the design is larger than the store accepts"),
            StoreError::Io(e) => {
                eprintln!("store error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "the store could not read or write its files")
            }
        };
        (status, text).into_response()
    }
}

/// Changes need a header a cross-origin page cannot add without a CORS preflight (we send none).
fn require_app(headers: &HeaderMap) -> Result<(), ApiError> {
    match headers.get("x-requested-with").and_then(|v| v.to_str().ok()) {
        Some("solidcraft") => Ok(()),
        _ => Err(ApiError(StoreError::BadName)),
    }
}

fn attachment(name: &str, bytes: Vec<u8>) -> Response {
    let disposition = format!("attachment; filename*=UTF-8''{}", library::url_encode(name));
    let mut res = bytes.into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    res
}

async fn health() -> &'static str {
    "ok"
}

async fn list(State(s): State<Shared>) -> Result<Response, ApiError> {
    let files = blocking(move || s.list()).await?;
    let mut res = Json(files).into_response();
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(res)
}

async fn get_file(State(s): State<Shared>, UrlPath(name): UrlPath<String>) -> Result<Response, ApiError> {
    let n = name.clone();
    Ok(attachment(&name, blocking(move || s.read(&n)).await?))
}

#[derive(Deserialize)]
struct PutQuery {
    modified: Option<u64>,
}

async fn put_file(
    State(s): State<Shared>,
    UrlPath(name): UrlPath<String>,
    Query(q): Query<PutQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    require_app(&headers)?;
    let out = blocking(move || s.put(&name, &body, q.modified)).await?;
    Ok(Json(out).into_response())
}

async fn delete_file(State(s): State<Shared>, UrlPath(name): UrlPath<String>, headers: HeaderMap) -> Result<StatusCode, ApiError> {
    require_app(&headers)?;
    blocking(move || s.delete(&name)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn versions(State(s): State<Shared>, UrlPath(name): UrlPath<String>) -> Result<Response, ApiError> {
    Ok(Json(blocking(move || s.versions(&name)).await?).into_response())
}

async fn get_version(State(s): State<Shared>, UrlPath((name, ts)): UrlPath<(String, u64)>) -> Result<Response, ApiError> {
    let n = name.clone();
    Ok(attachment(&format!("{ts}-{name}"), blocking(move || s.read_version(&n, ts)).await?))
}

async fn library_page(State(s): State<Shared>) -> Result<Html<String>, ApiError> {
    Ok(Html(blocking(move || library::render(&s)).await?))
}

fn router(store: Shared) -> Router {
    let limit = store.max_bytes();
    Router::new()
        .route("/api/health", get(health))
        .route("/api/files", get(list))
        .route("/api/files/{name}", get(get_file).put(put_file).delete(delete_file))
        .route("/api/files/{name}/versions", get(versions))
        .route("/api/files/{name}/versions/{ts}", get(get_version))
        .route("/library", get(library_page))
        .layer(DefaultBodyLimit::max(limit))
        .with_state(store)
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// `solidcraft-store --health`: exit 0 when the running server answers (for the container's healthcheck).
fn health_check() -> bool {
    let addr: String = env_or("STORE_ADDR", "0.0.0.0:8081".to_string());
    let port = addr.rsplit(':').next().unwrap_or("8081");
    let Ok(mut s) = TcpStream::connect(format!("127.0.0.1:{port}")) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    if s.write_all(b"GET /api/health HTTP/1.0\r\n\r\n").is_err() {
        return false;
    }
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.starts_with("HTTP/1.0 200") || out.starts_with("HTTP/1.1 200")
}

#[tokio::main]
async fn main() {
    if std::env::args().any(|a| a == "--health") {
        std::process::exit(if health_check() { 0 } else { 1 });
    }
    let addr: String = env_or("STORE_ADDR", "0.0.0.0:8081".to_string());
    let dir = PathBuf::from(env_or("STORE_DIR", "/data".to_string()));
    let store = match Store::open(&dir, env_or("STORE_MAX_BYTES", 256usize << 20), env_or("STORE_KEEP_VERSIONS", 100usize)) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("cannot open the store at {}: {e}", dir.display());
            std::process::exit(1);
        }
    };
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    println!("solidcraft-store: {} on {addr}", dir.display());
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    if let Err(e) = axum::serve(listener, router(store)).with_graceful_shutdown(shutdown).await {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn changes_need_the_app_header() {
        let mut h = HeaderMap::new();
        assert!(require_app(&h).is_err());
        h.insert("x-requested-with", HeaderValue::from_static("XMLHttpRequest"));
        assert!(require_app(&h).is_err());
        h.insert("x-requested-with", HeaderValue::from_static("solidcraft"));
        assert!(require_app(&h).is_ok());
    }
}

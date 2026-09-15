//! HTTP routes.
//!
//! | Route | Consumer | Purpose |
//! |---|---|---|
//! | `GET /screen` | device, preview | current spec JSON; `ETag` = content hash, `If-None-Match` → 304 |
//! | `PUT /screen`, `POST /screen` | curl / scripts / preview page | replace the spec; 204, or 400 with the parser's message |
//! | `GET /`, `/preview.js`, `/web_preview.wasm` | browser | the preview page and its wasm bundle |

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::RustEmbed;
use screen_spec::MAX_JSON_BYTES;
use tower_http::trace::TraceLayer;

use crate::state::{AppState, SetError};

/// The preview page. In release builds the files are embedded in the
/// binary; in debug builds rust-embed reads them from disk on each request,
/// so editing `preview.js` needs no rebuild.
#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

/// Where the preview page's files come from.
#[derive(Clone, Debug)]
pub enum AssetSource {
    /// The copy compiled into the binary (see [`Assets`]).
    Embedded,
    /// A directory on disk, read per request.
    Dir(PathBuf),
}

#[derive(Clone)]
struct Ctx {
    state: Arc<AppState>,
    assets: AssetSource,
}

/// Builds the application router.
pub fn router(state: Arc<AppState>, assets: AssetSource) -> Router {
    let ctx = Ctx { state, assets };
    Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/preview.js", get(preview_js))
        .route("/web_preview.wasm", get(preview_wasm))
        .route("/screen", get(get_screen).put(put_screen).post(put_screen))
        // Bodies over the device's limit are refused with 413 before we
        // read them; the error message in `put_screen` covers the edge case
        // of exactly-at-limit documents that still fail validation.
        .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(ctx)
}

async fn get_screen(State(ctx): State<Ctx>, headers: HeaderMap) -> Response {
    let current = ctx.state.current();
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|tag| tag.trim() == current.etag))
    {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, current.etag)]).into_response();
    }
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::ETAG,
                HeaderValue::from_str(&current.etag).expect("hex etag"),
            ),
        ],
        current.json,
    )
        .into_response()
}

async fn put_screen(State(ctx): State<Ctx>, body: Bytes) -> Response {
    let json = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "body is not valid UTF-8\n").into_response(),
    };
    match ctx.state.set(json) {
        Ok(current) => {
            tracing::info!("screen updated, etag {}", current.etag);
            (StatusCode::NO_CONTENT, [(header::ETAG, current.etag)]).into_response()
        }
        Err(e @ SetError::TooLarge(_)) => {
            (StatusCode::PAYLOAD_TOO_LARGE, format!("{e}\n")).into_response()
        }
        Err(e @ SetError::Invalid(_)) => {
            (StatusCode::BAD_REQUEST, format!("{e}\n")).into_response()
        }
        Err(e @ SetError::Io(_)) => {
            tracing::error!("{e}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")).into_response()
        }
    }
}

async fn index(State(ctx): State<Ctx>) -> Response {
    asset(&ctx.assets, "index.html", "text/html; charset=utf-8").await
}

async fn preview_js(State(ctx): State<Ctx>) -> Response {
    asset(&ctx.assets, "preview.js", "text/javascript; charset=utf-8").await
}

async fn preview_wasm(State(ctx): State<Ctx>) -> Response {
    asset(&ctx.assets, "web_preview.wasm", "application/wasm").await
}

async fn asset(source: &AssetSource, name: &str, content_type: &'static str) -> Response {
    let bytes: Option<Vec<u8>> = match source {
        AssetSource::Embedded => Assets::get(name).map(|f| f.data.into_owned()),
        AssetSource::Dir(dir) => tokio::fs::read(dir.join(name)).await.ok(),
    };
    match bytes {
        Some(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            ],
            bytes,
        )
            .into_response(),
        None if name.ends_with(".wasm") => (
            StatusCode::NOT_FOUND,
            "web_preview.wasm has not been built: run scripts/build-web-preview.sh and rebuild/restart the server\n",
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, format!("{name} not found\n")).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const KITCHEN: &str = include_str!("../../screen-spec/samples/kitchen.json");
    const MINIMAL: &str = include_str!("../../screen-spec/samples/minimal.json");

    fn app(state: AppState) -> Router {
        router(Arc::new(state), AssetSource::Embedded)
    }

    async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, String) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        // Lossy: the wasm asset is binary.
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    fn get(path: &str) -> Request<Body> {
        Request::get(path).body(Body::empty()).unwrap()
    }

    fn put(body: impl Into<Body>) -> Request<Body> {
        Request::put("/screen").body(body.into()).unwrap()
    }

    #[tokio::test]
    async fn get_screen_serves_current_with_etag() {
        let app = app(AppState::in_memory(KITCHEN));
        let (status, headers, body) = send(&app, get("/screen")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, KITCHEN);
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        let etag = headers[header::ETAG].to_str().unwrap().to_owned();
        assert_eq!(
            etag,
            format!("\"{:016x}\"", screen_spec::content_hash(KITCHEN.as_bytes()))
        );

        let req = Request::get("/screen")
            .header(header::IF_NONE_MATCH, &etag)
            .body(Body::empty())
            .unwrap();
        let (status, _, body) = send(&app, req).await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);
        assert_eq!(body, "");

        let req = Request::get("/screen")
            .header(header::IF_NONE_MATCH, "\"deadbeef\"")
            .body(Body::empty())
            .unwrap();
        let (status, _, _) = send(&app, req).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn put_replaces_document_and_rejects_bad_ones() {
        let app = app(AppState::in_memory(KITCHEN));

        let (status, headers, _) = send(&app, put(MINIMAL)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(headers.contains_key(header::ETAG));
        let (_, _, body) = send(&app, get("/screen")).await;
        assert_eq!(body, MINIMAL);

        let (status, _, body) = send(&app, put("{\"version\":1,\"regions\":[{")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("invalid screen spec JSON"), "{body}");

        let (status, _, body) = send(&app, put("{\"version\":9}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("unsupported screen spec version 9"), "{body}");

        let (status, _, _) = send(&app, put(vec![0xff, 0xfe])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // POST is an alias.
        let req = Request::post("/screen").body(Body::from(KITCHEN)).unwrap();
        let (status, _, _) = send(&app, req).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        // Rejected documents leave the current one untouched.
        let (_, _, body) = send(&app, get("/screen")).await;
        assert_eq!(body, KITCHEN);
    }

    #[tokio::test]
    async fn oversized_put_is_refused() {
        let app = app(AppState::in_memory(KITCHEN));
        let padding = " ".repeat(MAX_JSON_BYTES);
        let big = format!("{{\"version\":1{padding}}}");
        let (status, _, _) = send(&app, put(big)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn state_file_round_trip() {
        let dir =
            std::env::temp_dir().join(format!("reterminal-server-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("screen.json");

        // First start: no file, default sample.
        let state = AppState::load(Some(path.clone())).unwrap();
        assert_eq!(state.current().json, crate::state::DEFAULT_SPEC);
        let app = router(Arc::new(state), AssetSource::Embedded);
        let (status, _, _) = send(&app, put(MINIMAL)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), MINIMAL);
        assert!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count() == 1,
            "no temp file left behind"
        );

        // Second start: the file is picked up.
        let state = AppState::load(Some(path.clone())).unwrap();
        assert_eq!(state.current().json, MINIMAL);

        // A corrupt file is ignored rather than fatal.
        std::fs::write(&path, "not json").unwrap();
        let state = AppState::load(Some(path.clone())).unwrap();
        assert_eq!(state.current().json, crate::state::DEFAULT_SPEC);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn preview_assets_are_served() {
        let app = app(AppState::in_memory(KITCHEN));
        let (status, headers, body) = send(&app, get("/")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        assert!(body.contains("<canvas"));

        let (status, headers, body) = send(&app, get("/preview.js")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/javascript"));
        assert!(body.contains("wp_render"));

        // The wasm may or may not have been built in this checkout; either
        // answer must be a sensible one.
        let (status, headers, body) = send(&app, get("/web_preview.wasm")).await;
        match status {
            StatusCode::OK => assert_eq!(headers[header::CONTENT_TYPE], "application/wasm"),
            StatusCode::NOT_FOUND => assert!(body.contains("build-web-preview")),
            other => panic!("unexpected {other}"),
        }

        let (status, _, _) = send(&app, get("/nope")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}

//! HTTP routes.
//!
//! | Route | Consumer | Purpose |
//! |---|---|---|
//! | `GET /screen` | device, preview | template filled with the current values; `ETag` = content hash, `If-None-Match` → 304 |
//! | `PUT /screen`, `POST /screen` | curl / scripts / layout page | replace the template; 204, or 400 with the parser's message |
//! | `GET /template` | layout page | the stored template, placeholders unfilled |
//! | `GET /values` | values page | the current values JSON |
//! | `PUT /values`, `POST /values` | values page / scripts | replace the values; 204, or 400 |
//! | `GET /`, `/layout`, `/preview.js`, `/values.js`, `/web_preview.wasm` | browser | the values and layout pages and their scripts |

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

use crate::state::{AppState, Current, SetError};

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
        .route("/layout", get(layout))
        .route("/layout.html", get(layout))
        .route("/preview.js", get(preview_js))
        .route("/values.js", get(values_js))
        .route("/web_preview.wasm", get(preview_wasm))
        .route("/screen", get(get_screen).put(put_screen).post(put_screen))
        .route("/template", get(get_template))
        .route("/values", get(get_values).put(put_values).post(put_values))
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
    updated("template", ctx.state.set_template(json))
}

async fn get_template(State(ctx): State<Ctx>) -> Response {
    json_response(ctx.state.template())
}

async fn get_values(State(ctx): State<Ctx>) -> Response {
    json_response(ctx.state.values().to_json())
}

async fn put_values(State(ctx): State<Ctx>, body: Bytes) -> Response {
    updated("values", ctx.state.set_values(&body))
}

fn json_response(body: String) -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body,
    )
        .into_response()
}

/// 204 with the new ETag, or the error mapped to a status.
fn updated(what: &str, result: Result<Current, SetError>) -> Response {
    match result {
        Ok(current) => {
            tracing::info!("{what} updated, etag {}", current.etag);
            (StatusCode::NO_CONTENT, [(header::ETAG, current.etag)]).into_response()
        }
        Err(e @ SetError::TooLarge(_)) => {
            (StatusCode::PAYLOAD_TOO_LARGE, format!("{e}\n")).into_response()
        }
        Err(e @ (SetError::Invalid(_) | SetError::Template(_) | SetError::BadValues(_))) => {
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

async fn layout(State(ctx): State<Ctx>) -> Response {
    asset(&ctx.assets, "layout.html", "text/html; charset=utf-8").await
}

async fn values_js(State(ctx): State<Ctx>) -> Response {
    asset(&ctx.assets, "values.js", "text/javascript; charset=utf-8").await
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

    const TEMPLATE: &str = r#"{"version":1,"regions":[
        {"rect":[0,0,800,90],"text":"Today: {{todo}}"},
        {"rect":[0,100,400,40],"text":"{{meals.0}}|{{meals.1}}|{{meals.2}}"},
        {"rect":[400,100,400,300],"text":"{{to_eat}}"}]}"#;
    const VALUES: &str = r#"{"todo":"Make \"Stock\"","meals":["Cereal","Sandwich","Roast Chicken"],"to_eat":["ham","milk"]}"#;

    fn put_values(body: impl Into<Body>) -> Request<Body> {
        Request::put("/values").body(body.into()).unwrap()
    }

    #[tokio::test]
    async fn template_is_filled_with_values() {
        let app = app(AppState::in_memory(KITCHEN));
        let (status, _, _) = send(&app, put(TEMPLATE)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, headers, before) = send(&app, get("/screen")).await;
        assert!(before.contains(r#""text":"Today: ""#), "{before}");

        let (status, _, _) = send(&app, put_values(VALUES)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, headers2, body) = send(&app, get("/screen")).await;
        assert_ne!(headers[header::ETAG], headers2[header::ETAG]);
        assert!(body.contains(r#""Today: Make \"Stock\"""#), "{body}");
        assert!(body.contains("Cereal|Sandwich|Roast Chicken"), "{body}");
        assert!(body.contains(r#""- ham\n- milk""#), "{body}");

        // The template and values are served back as stored.
        let (_, _, body) = send(&app, get("/template")).await;
        assert_eq!(body, TEMPLATE);
        let (_, headers, body) = send(&app, get("/values")).await;
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
        let values: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(values["meals"][2], "Roast Chicken");

        // Re-putting the template keeps the values.
        let (status, _, _) = send(&app, put(TEMPLATE)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, _, body) = send(&app, get("/screen")).await;
        assert!(body.contains("Cereal|Sandwich"), "{body}");
    }

    #[tokio::test]
    async fn bad_templates_and_values_are_rejected() {
        let app = app(AppState::in_memory(TEMPLATE));
        let (status, _, body) = send(&app, put(r#"{"version":1,"x":"{{nope}}"}"#)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("unknown placeholder"), "{body}");

        let (status, _, body) = send(&app, put_values(r#"{"meals":["a"]}"#)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("invalid values JSON"), "{body}");

        let nine = format!(r#"{{"to_eat":{:?}}}"#, ["x"; 9]);
        let (status, _, body) = send(&app, put_values(nine)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("limit is 8"), "{body}");

        // Values that make a region's text too long for the device.
        let long = "x".repeat(screen_spec::MAX_TEXT + 1);
        let (status, _, _) = send(&app, put_values(format!(r#"{{"todo":"{long}"}}"#))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (_, _, body) = send(&app, get("/template")).await;
        assert_eq!(body, TEMPLATE);
    }

    #[tokio::test]
    async fn values_file_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "reterminal-server-values-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("screen.json");

        let app = router(
            Arc::new(AppState::load(Some(path.clone())).unwrap()),
            AssetSource::Embedded,
        );
        send(&app, put(TEMPLATE)).await;
        let (status, _, _) = send(&app, put_values(VALUES)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(dir.join(crate::state::VALUES_FILE).exists());

        let state = AppState::load(Some(path)).unwrap();
        assert_eq!(state.template(), TEMPLATE);
        assert_eq!(state.values().meals[1], "Sandwich");
        assert!(state.current().json.contains("Cereal|Sandwich"));

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

        let (status, _, body) = send(&app, get("/layout")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("id=\"json\""));

        let (status, _, body) = send(&app, get("/values.js")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("/values"));

        let (status, _, _) = send(&app, get("/nope")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}

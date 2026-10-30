//! Server side: the HTML shell with a per-response CSP nonce, the Leptos page routes, the PWA
//! manifest, icon and service worker, and the binary's composition over `polaris-ui-web`'s
//! session owner, JSON API and event stream.

use std::path::PathBuf;

use axum::Router;
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use leptos::prelude::*;
use leptos_axum::{LeptosRoutes, ResponseOptions, generate_route_list};
use leptos_meta::MetaTags;

use crate::app::App;

/// The hand-written service worker (the only JavaScript file in this app).
const SERVICE_WORKER: &str = include_str!("../assets/sw.js");
/// Web app manifest.
const MANIFEST: &str = r##"{
  "name": "Polaris",
  "short_name": "Polaris",
  "description": "Local coding workspace",
  "start_url": "/",
  "scope": "/",
  "display": "standalone",
  "background_color": "#000000",
  "theme_color": "#000000",
  "icons": [{ "src": "/icon.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "any" }]
}
"##;
/// App icon.
const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" fill="none"><rect width="64" height="64" rx="14" fill="#000"/><path fill="#1C1C1C" d="M15 15h34v34H15z"/><path d="M21 32h8m6 0h8" stroke="#76B900" stroke-width="2"/><path d="m32 27 1.7 3.3L37 32l-3.3 1.7L32 37l-1.7-3.3L27 32l3.3-1.7L32 27Z" fill="#24D046"/></svg>
"##;

/// Content Security Policy for a page whose inline scripts carry `nonce`.
#[must_use]
pub fn content_security_policy(nonce: &str) -> String {
    format!(
        "default-src 'self'; script-src 'self' 'nonce-{nonce}' 'wasm-unsafe-eval'; style-src 'self'; \
         img-src 'self' data:; font-src 'self'; connect-src 'self'; worker-src 'self'; \
         manifest-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; \
         frame-ancestors 'none'"
    )
}

/// The document around [`App`]; rendered only on the server.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    if let (Some(nonce), Some(response)) = (use_nonce(), use_context::<ResponseOptions>())
        && let Ok(value) = HeaderValue::from_str(&content_security_policy(&nonce))
    {
        response.insert_header(header::CONTENT_SECURITY_POLICY, value);
    }
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width,initial-scale=1"/>
                <meta name="color-scheme" content="dark"/>
                <meta name="theme-color" content="#000000"/>
                <meta name="referrer" content="no-referrer"/>
                <link rel="manifest" href="/manifest.webmanifest"/>
                <link rel="icon" href="/icon.svg" type="image/svg+xml"/>
                <link rel="stylesheet" href="/app.css"/>
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

fn static_text(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

async fn service_worker() -> Response {
    static_text("text/javascript; charset=utf-8", SERVICE_WORKER)
}

async fn manifest() -> Response {
    static_text("application/manifest+json", MANIFEST)
}

async fn icon() -> Response {
    static_text("image/svg+xml", ICON)
}

async fn harden(mut response: Response) -> Response {
    let headers = response.headers_mut();
    for (name, value) in [
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (HeaderName::from_static("x-frame-options"), "DENY"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

/// Page routes: the Leptos app at `/`, the compiled browser bundle under `/pkg`, and the PWA
/// files. Merged into the `polaris-ui-web` server, which keeps the API and event stream.
pub fn pages(options: LeptosOptions) -> Router {
    let routes = generate_route_list(App);
    let pkg = PathBuf::from(options.site_root.as_ref()).join(options.site_pkg_dir.as_ref());
    let shell_options = options.clone();
    Router::new()
        .leptos_routes(&options, routes, move || shell(shell_options.clone()))
        .route("/sw.js", get(service_worker))
        .route("/manifest.webmanifest", get(manifest))
        .route("/icon.svg", get(icon))
        .nest_service("/pkg", tower_http::services::ServeDir::new(pkg))
        .layer(axum::middleware::map_response(harden))
        .with_state(options)
}

/// Leptos options for a site built by `cargo leptos build` into `site_root`.
#[must_use]
pub fn site_options(site_root: &str) -> LeptosOptions {
    LeptosOptions::builder()
        .output_name("polaris-web")
        .site_root(site_root)
        .site_pkg_dir("pkg")
        .build()
}

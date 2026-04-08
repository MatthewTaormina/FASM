//! Admin API router — mounts all `/api/v1/` endpoints.
//!
//! Attach to the main axum `Router` with:
//! ```rust,ignore
//! let app = Router::new()
//!     .merge(admin::router())
//!     // ...
//! ```

pub mod apps;
pub mod auth;
pub mod files;
pub mod namespace;
pub mod registry;
pub mod routes;

use crate::http_handler::AppState;
use axum::{
    routing::{delete, get, put},
    Router,
};

pub use registry::AppRegistry;

/// Build the `/api/v1/` axum sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        // ── Namespaces ──────────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces",
            get(namespace::list_namespaces).post(namespace::create_namespace),
        )
        .route(
            "/api/v1/namespaces/:ns",
            get(namespace::get_namespace).delete(namespace::delete_namespace),
        )
        // ── Apps ────────────────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/:ns/apps",
            get(apps::list_apps).post(apps::create_app),
        )
        .route(
            "/api/v1/namespaces/:ns/apps/:app",
            get(apps::get_app).delete(apps::delete_app),
        )
        // ── Files ───────────────────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/:ns/apps/:app/files",
            get(files::list_files),
        )
        .route(
            "/api/v1/namespaces/:ns/apps/:app/files/:filename",
            put(files::upload_file)
                .get(files::download_file)
                .delete(files::delete_file),
        )
        // ── Routes (entry points) ────────────────────────────────────────────
        .route(
            "/api/v1/namespaces/:ns/apps/:app/routes",
            get(routes::list_routes).post(routes::register_route),
        )
        .route(
            "/api/v1/namespaces/:ns/apps/:app/routes/:route_id",
            delete(routes::unregister_route),
        )
}

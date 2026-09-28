#![deny(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::too_many_lines)]

mod admin;
mod auth;
mod board;
mod db;
mod error;
mod middleware;
mod models;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware::from_fn_with_state,
    routing::{get, post},
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::{io::Read, sync::Arc};
use tower_http::limit::RequestBodyLimitLayer;

#[tokio::main]
async fn main() -> Result<(), error::AppError> {
    tracing_subscriber::fmt::init();

    let exe_path = std::env::current_exe()?;
    let mut file = std::fs::File::open(&exe_path)?;

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let hash_result = hasher.finalize();

    let sig_bytes = std::fs::read("senseaim.sig")?;
    if sig_bytes.len() != 64 {
        return Err(error::AppError::Integrity);
    }

    let signature = Signature::from_slice(&sig_bytes).map_err(|_| error::AppError::Integrity)?;
    let public_key_bytes: [u8; 32] = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];

    let verifying_key =
        VerifyingKey::from_bytes(&public_key_bytes).map_err(|_| error::AppError::Integrity)?;
    verifying_key
        .verify(&hash_result, &signature)
        .map_err(|_| error::AppError::Integrity)?;

    let integrity_hash = hex::encode(hash_result);

    let db_instance = tokio::task::spawn_blocking(db::init_db).await??;
    let db_arc = Arc::new(db_instance);
    let db_clone = Arc::clone(&db_arc);

    tokio::task::spawn_blocking(move || db::run_first_time_setup(&db_clone)).await??;

    let app_state = models::AppState {
        db: db_arc,
        integrity_hash,
    };

    let protected_routes = Router::new()
        .route("/profile", get(auth::handlers::profile_handler))
        .route(
            "/profile/sessions/revoke",
            post(auth::handlers::revoke_other_sessions),
        )
        .route(
            "/board",
            get(board::threads::board_page).post(board::threads::create_thread),
        )
        .route("/board/search", post(board::threads::search_content))
        .route(
            "/board/:id",
            get(board::threads::thread_page).post(board::posts::create_post),
        )
        .route("/board/:id/vote", post(board::interactions::vote_poll))
        .route("/board/:id/plus", post(board::interactions::toggle_plus))
        .route(
            "/board/:id/giveaway",
            post(board::interactions::participate_giveaway),
        )
        .route("/admin", get(admin::dashboard::admin_dashboard))
        .route("/admin/invite", post(admin::invites::admin_generate_invite))
        .route(
            "/admin/invites/grant_all",
            post(admin::invites::admin_invites_grant_all),
        )
        .route(
            "/admin/invites/revoke_one_all",
            post(admin::invites::admin_invites_revoke_one_all),
        )
        .route(
            "/admin/invites/revoke_all",
            post(admin::invites::admin_invites_revoke_all),
        )
        .route("/admin/grant", post(admin::invites::admin_grant_to_user))
        .route(
            "/admin/revoke",
            post(admin::invites::admin_revoke_from_user),
        )
        .route("/admin/roles", post(admin::users::admin_update_roles))
        .route("/admin/search", post(admin::dashboard::admin_search))
        .route("/admin/ban", post(admin::users::admin_ban))
        .route("/admin/unban", post(admin::users::admin_unban))
        .route(
            "/admin/thread/delete",
            post(admin::moderation::admin_delete_thread),
        )
        .route(
            "/admin/post/delete",
            post(admin::moderation::admin_delete_post),
        )
        .route(
            "/admin/thread/pin",
            post(admin::moderation::admin_pin_thread),
        )
        .route(
            "/admin/thread/unpin",
            post(admin::moderation::admin_unpin_thread),
        )
        .route(
            "/admin/thread/lock",
            post(admin::moderation::admin_lock_thread),
        )
        .route(
            "/admin/thread/unlock",
            post(admin::moderation::admin_unlock_thread),
        )
        .route(
            "/admin/thread/solve",
            post(admin::moderation::admin_solve_thread),
        )
        .route(
            "/admin/thread/unsolve",
            post(admin::moderation::admin_unsolve_thread),
        )
        .route("/admin/post/best", post(admin::moderation::admin_mark_best))
        .route(
            "/admin/post/unbest",
            post(admin::moderation::admin_unmark_best),
        )
        .route("/admin/audit", get(admin::dashboard::admin_audit))
        .route_layer(from_fn_with_state(
            app_state.clone(),
            middleware::require_auth,
        ));

    let app = Router::new()
        .route("/", get(auth::handlers::index_handler))
        .route(
            "/login",
            get(auth::handlers::login_page).post(auth::handlers::login_post),
        )
        .route(
            "/register",
            get(auth::handlers::register_page).post(auth::handlers::register_post),
        )
        .route(
            "/reset",
            get(auth::handlers::reset_page).post(auth::handlers::reset_post),
        )
        .route("/assets/styles.css", get(auth::handlers::styles_handler))
        .merge(protected_routes)
        .layer(DefaultBodyLimit::max(8192))
        .layer(RequestBodyLimitLayer::new(8192))
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

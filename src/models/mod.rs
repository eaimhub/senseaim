#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

pub mod board;
pub mod user;

use redb::Database;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub integrity_hash: String,
}

impl axum::extract::FromRef<AppState> for Arc<Database> {
    fn from_ref(state: &AppState) -> Self {
        state.db.clone()
    }
}

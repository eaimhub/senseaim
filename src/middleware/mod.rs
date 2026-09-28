#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::CookieJar;
use std::sync::Arc;

use crate::{db, error::AppError, models};

pub async fn require_auth(
    State(state): State<models::AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let db_instance = state.db;
    let session_id = jar
        .get("session")
        .map_or_else(String::new, |c| c.value().to_string());
    if session_id.is_empty() {
        return Ok(Redirect::to("/login").into_response());
    }

    let db_clone = Arc::clone(&db_instance);
    let check_result = tokio::task::spawn_blocking(
        move || -> Result<Option<(models::user::Session, models::user::User)>, AppError> {
            let read_txn = db_clone.begin_read()?;
            let sessions = read_txn.open_table(db::SESSIONS_TABLE)?;

            let session_bytes = match sessions.get(session_id.as_str())? {
                Some(b) => b.value().to_vec(),
                None => return Ok(None),
            };

            let session: models::user::Session = bincode::deserialize(&session_bytes)?;

            let users_by_id = read_txn.open_table(db::USERS_BY_ID_TABLE)?;
            let login = match users_by_id.get(session.user_id)? {
                Some(b) => b.value().to_string(),
                None => return Ok(None),
            };

            let users = read_txn.open_table(db::USERS_TABLE)?;
            let user = match users.get(login.as_str())? {
                Some(b) => bincode::deserialize::<models::user::User>(b.value())?,
                None => return Ok(None),
            };

            Ok(Some((session, user)))
        },
    )
    .await??;

    match check_result {
        Some((session, user)) => {
            if (user.roles & models::user::FLAG_BANNED) != 0 {
                let db_clone2 = Arc::clone(&db_instance);
                let sid = jar
                    .get("session")
                    .map_or_else(String::new, |c| c.value().to_string());

                if !sid.is_empty() {
                    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
                        let write_txn = db_clone2.begin_write()?;
                        {
                            let mut active_sessions = write_txn.open_table(db::SESSIONS_TABLE)?;
                            active_sessions.remove(sid.as_str())?;
                        }
                        write_txn.commit()?;
                        Ok(())
                    })
                    .await??;
                }

                let cleared_jar = jar.remove(axum_extra::extract::cookie::Cookie::from("session"));
                return Ok((StatusCode::FORBIDDEN, cleared_jar, "EXILED").into_response());
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();

            if session.expires_at < now {
                return Ok(Redirect::to("/login").into_response());
            }

            request.extensions_mut().insert(user);
            Ok(next.run(request).await)
        }
        None => Ok(Redirect::to("/login").into_response()),
    }
}

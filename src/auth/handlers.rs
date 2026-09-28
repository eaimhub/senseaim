#![deny(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(
    clippy::struct_excessive_bools,
    clippy::too_many_lines,
    clippy::collapsible_if,
    clippy::cast_possible_truncation
)]

use askama::Template;
use axum::{
    Form,
    extract::{Extension, Query, State},
    http::header,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use rand::Rng;
use redb::{ReadableTable, ReadableTableMetadata};
use secrecy::{ExposeSecret, Secret};
use serde::Deserialize;

use crate::{
    auth::crypto,
    db,
    error::AppError,
    models::{self, user::User},
};

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate;

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate;

#[derive(Template)]
#[template(path = "register.html")]
pub struct RegisterTemplate;

#[derive(Template)]
#[template(path = "register_success.html")]
pub struct RegisterSuccessTemplate {
    pub id: u64,
    pub recovery_code: String,
}

#[derive(Template)]
#[template(path = "profile.html")]
pub struct ProfileTemplate {
    pub id: u64,
    pub login: String,
    pub recovery_code: String,
    pub is_user: bool,
    pub is_subscriber: bool,
    pub is_admin: bool,
    pub active_invites: Vec<String>,
    pub tree: Vec<UserNode>,
    pub core_integrity: String,
    pub sessions: Vec<SessionView>,
    pub plus_count: u32,
    pub post_ids: Vec<u64>,
    pub post_page: usize,
    pub has_next_posts: bool,
    pub has_prev_posts: bool,
    pub total_posts: usize,
}

#[derive(Template)]
#[template(path = "reset.html")]
pub struct ResetTemplate;

#[derive(Template)]
#[template(path = "reset_success.html")]
pub struct ResetSuccessTemplate {
    pub new_recovery_code: String,
}

pub struct UserNode {
    pub id: u64,
    pub login: String,
    pub is_user: bool,
    pub is_subscriber: bool,
    pub is_admin: bool,
    pub is_banned: bool,
    pub padding_rem: f64,
}

pub struct SessionView {
    pub id_preview: String,
    pub is_current: bool,
    pub expires_in_hrs: u64,
}

#[derive(Deserialize)]
pub struct RegisterForm {
    pub login: String,
    pub invite: String,
    pub password: Secret<String>,
}

#[derive(Deserialize)]
pub struct LoginForm {
    pub login: String,
    pub password: Secret<String>,
}

#[derive(Deserialize)]
pub struct ResetPasswordForm {
    pub login: String,
    pub recovery_code: String,
    pub new_password: Secret<String>,
}

#[derive(Deserialize)]
pub struct ProfileQuery {
    pub page: Option<usize>,
}

type ProfileData = (
    Vec<String>,
    Vec<UserNode>,
    Vec<SessionView>,
    Vec<u64>,
    usize,
    bool,
    bool,
);

pub async fn index_handler() -> Result<IndexTemplate, AppError> {
    Ok(IndexTemplate)
}

pub async fn styles_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("../../assets/styles.css"),
    )
}

pub async fn login_page() -> Result<LoginTemplate, AppError> {
    Ok(LoginTemplate)
}

pub async fn register_page() -> Result<RegisterTemplate, AppError> {
    Ok(RegisterTemplate)
}

pub async fn reset_page() -> Result<ResetTemplate, AppError> {
    Ok(ResetTemplate)
}

pub async fn register_post(
    State(state): State<models::AppState>,
    Form(form): Form<RegisterForm>,
) -> Result<RegisterSuccessTemplate, AppError> {
    if form.login.len() < 4
        || form.login.len() > 24
        || !form
            .login
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AppError::Validation);
    }
    if form.password.expose_secret().len() < 8
        || form.password.expose_secret().len() > 32
        || !form.password.expose_secret().is_ascii()
    {
        return Err(AppError::Validation);
    }
    if form.invite.len() != 24 {
        return Err(AppError::Validation);
    }

    let db_instance = state.db;
    let (id, recovery_code) =
        tokio::task::spawn_blocking(move || -> Result<(u64, String), AppError> {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();

            {
                let write_txn = db_instance.begin_write()?;
                db::repository::check_rate_limit(&write_txn, form.login.as_str(), 900, 5, now)?;
                write_txn.commit()?;
            }

            let creator_id = {
                let read_txn = db_instance.begin_read()?;
                let invites = read_txn.open_table(db::INVITES_TABLE)?;
                match invites.get(form.invite.as_str())? {
                    Some(val) => val.value(),
                    None => return Err(AppError::InvalidInvite),
                }
            };

            {
                let read_txn = db_instance.begin_read()?;
                if db::repository::get_user(&read_txn, form.login.as_str())?.is_some() {
                    return Err(AppError::UsernameTaken);
                }
            }

            let password_hash = crypto::hash_password(&form.password)?;
            let recovery_code = crypto::generate_alphanumeric_24();
            let user_id: u64 = rand::thread_rng().gen_range(1_000_000_000..=9_999_999_999);

            let write_txn = db_instance.begin_write()?;
            {
                let mut invites = write_txn.open_table(db::INVITES_TABLE)?;
                invites.remove(form.invite.as_str())?;

                let mut invites_by_owner =
                    write_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;
                invites_by_owner.remove(creator_id, form.invite.as_str())?;

                let users = write_txn.open_table(db::USERS_TABLE)?;
                let is_first = users.is_empty()?;
                drop(users);

                let roles = if is_first {
                    models::user::ROLE_ROOT
                } else {
                    models::user::ROLE_USER
                };

                let user = models::user::User {
                    id: user_id,
                    login: form.login.clone(),
                    password_hash,
                    recovery_code: recovery_code.clone(),
                    roles,
                    invited_by: creator_id,
                    plus_count: 0,
                    post_count: 0,
                };

                db::repository::save_user(&write_txn, &user)?;

                let mut users_by_id = write_txn.open_table(db::USERS_BY_ID_TABLE)?;
                users_by_id.insert(user_id, form.login.as_str())?;

                let mut invited_idx = write_txn.open_multimap_table(db::INVITED_BY_INDEX)?;
                invited_idx.insert(creator_id, user_id)?;
            }
            write_txn.commit()?;

            Ok((user_id, recovery_code))
        })
        .await??;

    Ok(RegisterSuccessTemplate { id, recovery_code })
}

pub async fn login_post(
    State(state): State<models::AppState>,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> Result<(CookieJar, Redirect), AppError> {
    if form.login.len() < 4 || form.login.len() > 24 {
        return Err(AppError::Validation);
    }

    let db_instance = state.db;
    let session_id = tokio::task::spawn_blocking(move || -> Result<String, AppError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();

        {
            let write_txn = db_instance.begin_write()?;
            db::repository::check_rate_limit(&write_txn, form.login.as_str(), 900, 5, now)?;
            write_txn.commit()?;
        }

        let user_opt = {
            let read_txn = db_instance.begin_read()?;
            db::repository::get_user(&read_txn, form.login.as_str())?
        };

        let user_id = if let Some(user) = user_opt {
            if (user.roles & models::user::FLAG_BANNED) != 0 {
                return Err(AppError::InvalidCredentials);
            }
            crypto::verify_password(&form.password, &user.password_hash)?;
            user.id
        } else {
            crypto::verify_password_dummy();
            return Err(AppError::InvalidCredentials);
        };

        let session_id = crypto::generate_session_id();
        let session = models::user::Session {
            user_id,
            expires_at: now + 86400,
        };

        let session_data = bincode::serialize(&session)?;
        let write_txn = db_instance.begin_write()?;
        {
            let mut sessions = write_txn.open_table(db::SESSIONS_TABLE)?;
            sessions.insert(session_id.as_str(), session_data.as_slice())?;
        }
        write_txn.commit()?;

        Ok(session_id)
    })
    .await??;

    let cookie = Cookie::build(("session", session_id))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .path("/")
        .build();

    Ok((jar.add(cookie), Redirect::to("/profile")))
}

pub async fn reset_post(
    State(state): State<models::AppState>,
    Form(form): Form<ResetPasswordForm>,
) -> Result<ResetSuccessTemplate, AppError> {
    if form.login.len() < 4 || form.login.len() > 24 {
        return Err(AppError::Validation);
    }
    if form.recovery_code.len() != 24
        || form.new_password.expose_secret().len() < 8
        || form.new_password.expose_secret().len() > 32
        || !form.new_password.expose_secret().is_ascii()
    {
        return Err(AppError::Validation);
    }

    let db_instance = state.db;
    let new_recovery_code = tokio::task::spawn_blocking(move || -> Result<String, AppError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();

        {
            let write_txn = db_instance.begin_write()?;
            db::repository::check_rate_limit(&write_txn, form.login.as_str(), 900, 5, now)?;
            write_txn.commit()?;
        }

        let user_opt = {
            let read_txn = db_instance.begin_read()?;
            db::repository::get_user(&read_txn, form.login.as_str())?
        };

        let mut target_user = None;
        if let Some(user) = user_opt {
            if crypto::constant_time_compare_24(&user.recovery_code, &form.recovery_code) {
                target_user = Some(user);
            } else {
                crypto::verify_password_dummy();
            }
        } else {
            crypto::verify_password_dummy();
        }

        let mut user = target_user.ok_or(AppError::InvalidCredentials)?;

        user.password_hash = crypto::hash_password(&form.new_password)?;
        let new_recovery_code = crypto::generate_alphanumeric_24();
        user.recovery_code.clone_from(&new_recovery_code);

        let write_txn = db_instance.begin_write()?;
        db::repository::save_user(&write_txn, &user)?;
        write_txn.commit()?;

        Ok(new_recovery_code)
    })
    .await??;

    Ok(ResetSuccessTemplate { new_recovery_code })
}

pub async fn profile_handler(
    Extension(user): Extension<User>,
    State(state): State<models::AppState>,
    jar: CookieJar,
    Query(query): Query<ProfileQuery>,
) -> Result<ProfileTemplate, AppError> {
    let db_instance = state.db;
    let integrity_hash = state.integrity_hash;
    let user_id = user.id;
    let page = query.page.unwrap_or(0);
    let limit = 50;

    let current_session_id = jar
        .get("session")
        .map_or_else(String::new, |c| c.value().to_string());

    let (active_invites, tree, sessions, post_ids, total_posts, has_next_posts, has_prev_posts) =
        tokio::task::spawn_blocking(move || -> Result<ProfileData, AppError> {
            let read_txn = db_instance.begin_read()?;

            let users_table = read_txn.open_table(db::USERS_TABLE)?;
            let users_by_id = read_txn.open_table(db::USERS_BY_ID_TABLE)?;
            let invited_idx = read_txn.open_multimap_table(db::INVITED_BY_INDEX)?;
            let invites_by_owner = read_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;
            let posts_by_author = read_txn.open_multimap_table(db::POSTS_BY_AUTHOR_INDEX)?;
            let threads_by_author = read_txn.open_multimap_table(db::THREADS_BY_AUTHOR_INDEX)?;

            let mut list = Vec::new();
            if let Ok(iter) = invites_by_owner.get(user_id) {
                for code in iter.flatten() {
                    list.push(code.value().to_string());
                }
            }

            let mut all_post_ids = Vec::new();
            if let Ok(iter) = threads_by_author.get(user_id) {
                for t in iter.flatten() {
                    all_post_ids.push(t.value());
                }
            }
            if let Ok(iter) = posts_by_author.get(user_id) {
                for p in iter.flatten() {
                    all_post_ids.push(p.value());
                }
            }
            all_post_ids.sort_unstable_by(|a, b| b.cmp(a));

            let total_posts = all_post_ids.len();
            let skip = page * limit;
            let post_ids: Vec<u64> = all_post_ids.into_iter().skip(skip).take(limit).collect();
            let has_next_posts = skip + limit < total_posts;
            let has_prev_posts = page > 0;

            let mut tree = Vec::new();
            let mut stack = Vec::new();

            if let Ok(iter) = invited_idx.get(user_id) {
                let mut children = Vec::new();
                for child_id in iter.flatten() {
                    children.push(child_id.value());
                }

                let mut child_users = Vec::new();
                for cid in children {
                    if let Some(login_val) = users_by_id.get(cid)? {
                        if let Some(u_val) = users_table.get(login_val.value())? {
                            if let Ok(u) = bincode::deserialize::<User>(u_val.value()) {
                                child_users.push(u);
                            }
                        }
                    }
                }
                child_users.sort_by_key(|u| u.id);
                for child in child_users.into_iter().rev() {
                    stack.push((child, 0));
                }
            }

            while let Some((curr, depth)) = stack.pop() {
                tree.push(UserNode {
                    id: curr.id,
                    login: curr.login.clone(),
                    is_user: (curr.roles & models::user::ROLE_USER) != 0,
                    is_subscriber: (curr.roles & models::user::ROLE_SUBSCRIBER) != 0,
                    is_admin: (curr.roles & models::user::ROLE_ADMIN) != 0,
                    is_banned: (curr.roles & models::user::FLAG_BANNED) != 0,
                    padding_rem: f64::from(depth) * 1.5,
                });

                if let Ok(iter) = invited_idx.get(curr.id) {
                    let mut children = Vec::new();
                    for child_id in iter.flatten() {
                        children.push(child_id.value());
                    }

                    let mut child_users = Vec::new();
                    for cid in children {
                        if let Some(login_val) = users_by_id.get(cid)? {
                            if let Some(u_val) = users_table.get(login_val.value())? {
                                if let Ok(u) = bincode::deserialize::<User>(u_val.value()) {
                                    child_users.push(u);
                                }
                            }
                        }
                    }
                    child_users.sort_by_key(|u| u.id);
                    for child in child_users.into_iter().rev() {
                        stack.push((child, depth + 1));
                    }
                }
            }

            let sessions_table = read_txn.open_table(db::SESSIONS_TABLE)?;
            let mut user_sessions = Vec::new();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();

            for item in sessions_table.iter()? {
                let (k, v) = item?;
                let session: models::user::Session = bincode::deserialize(v.value())?;

                if session.user_id == user_id && session.expires_at > now {
                    let s_id = k.value().to_string();
                    let is_current = s_id == current_session_id;
                    let preview = if s_id.len() >= 16 {
                        s_id[..16].to_string()
                    } else {
                        s_id
                    };
                    let expires_in_hrs = (session.expires_at - now) / 3600;

                    user_sessions.push(SessionView {
                        id_preview: preview,
                        is_current,
                        expires_in_hrs,
                    });
                }
            }

            user_sessions.sort_by_key(|a| std::cmp::Reverse(a.is_current));

            Ok((
                list,
                tree,
                user_sessions,
                post_ids,
                total_posts,
                has_next_posts,
                has_prev_posts,
            ))
        })
        .await??;

    Ok(ProfileTemplate {
        id: user.id,
        login: user.login.clone(),
        recovery_code: user.recovery_code.clone(),
        is_user: (user.roles & models::user::ROLE_USER) != 0,
        is_subscriber: (user.roles & models::user::ROLE_SUBSCRIBER) != 0,
        is_admin: (user.roles & models::user::ROLE_ADMIN) != 0,
        active_invites,
        tree,
        core_integrity: integrity_hash,
        sessions,
        plus_count: user.plus_count,
        post_ids,
        post_page: page,
        has_next_posts,
        has_prev_posts,
        total_posts,
    })
}

pub async fn revoke_other_sessions(
    Extension(user): Extension<User>,
    State(state): State<models::AppState>,
    jar: CookieJar,
) -> Result<Response, AppError> {
    let current_session_id = jar
        .get("session")
        .map_or_else(String::new, |c| c.value().to_string());
    if current_session_id.is_empty() {
        return Ok(Redirect::to("/login").into_response());
    }

    let db_instance = state.db;
    let user_id = user.id;
    let login = user.login.clone();

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        let mut to_remove = Vec::new();

        {
            let sessions_table = write_txn.open_table(db::SESSIONS_TABLE)?;
            for item in sessions_table.iter()? {
                let (k, v) = item?;
                let session: models::user::Session = bincode::deserialize(v.value())?;

                if session.user_id == user_id {
                    let s_id = k.value().to_string();
                    if s_id != current_session_id {
                        to_remove.push(s_id);
                    }
                }
            }
        }

        {
            let mut sessions_table = write_txn.open_table(db::SESSIONS_TABLE)?;
            for s_id in &to_remove {
                sessions_table.remove(s_id.as_str())?;
            }

            if !to_remove.is_empty() {
                let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                let ts = u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_micros(),
                )
                .unwrap_or(0);
                let action = format!(
                    "AGENT [{}] REVOKED {} ACTIVE SESSIONS",
                    login,
                    to_remove.len()
                );
                audit.insert(ts, action.as_str())?;
            }
        }

        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/profile").into_response())
}

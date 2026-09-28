#![deny(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::cast_possible_truncation)]

use axum::{
    extract::{Extension, Form, State},
    response::{IntoResponse, Redirect, Response},
};
use redb::ReadableMultimapTable;
use serde::Deserialize;

use crate::{
    admin::is_admin,
    db,
    error::AppError,
    models::{
        self,
        user::{FLAG_BANNED, ROLE_ADMIN, ROLE_SUBSCRIBER, ROLE_USER, User},
    },
};

#[derive(Deserialize)]
pub struct BanForm {
    pub target_id: u64,
    pub cascade: Option<String>,
}

#[derive(Deserialize)]
pub struct UnbanForm {
    pub target_id: u64,
    pub cascade: Option<String>,
}

#[derive(Deserialize)]
pub struct RolesForm {
    pub target_id: u64,
    pub role_user: Option<String>,
    pub role_subscriber: Option<String>,
    pub role_admin: Option<String>,
}

pub async fn admin_update_roles(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<RolesForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }
    if form.target_id == 1 {
        return Err(AppError::Validation);
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;

        let target_login = match db::repository::get_user_by_id_w(&write_txn, form.target_id)? {
            Some(u) => u.login,
            None => return Ok(()),
        };

        if let Some(mut u) = db::repository::get_user_w(&write_txn, target_login.as_str())? {
            let mut new_roles = 0;
            if form.role_user.is_some() {
                new_roles |= ROLE_USER;
            }
            if form.role_subscriber.is_some() {
                new_roles |= ROLE_SUBSCRIBER;
            }
            if form.role_admin.is_some() {
                new_roles |= ROLE_ADMIN;
            }

            if (u.roles & FLAG_BANNED) != 0 {
                new_roles |= FLAG_BANNED;
            }

            u.roles = new_roles;
            db::repository::save_user(&write_txn, &u)?;

            let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
            let ts = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_micros(),
            )
            .unwrap_or(0);
            let action = format!(
                "OVERSEER [{}] UPDATED ROLES FOR [{}]",
                admin_user.login, target_login
            );
            audit.insert(ts, action.as_str())?;
        }

        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_ban(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<BanForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }
    if form.target_id == admin_user.id || form.target_id == 1 {
        return Err(AppError::Validation);
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;

        let target_login = match db::repository::get_user_by_id_w(&write_txn, form.target_id)? {
            Some(u) => u.login,
            None => return Ok(()),
        };

        let mut to_ban = vec![form.target_id];

        if form.cascade.is_some() {
            let invited_idx = write_txn.open_multimap_table(db::INVITED_BY_INDEX)?;
            let mut i = 0;
            while i < to_ban.len() {
                let current = to_ban[i];
                if let Ok(iter) = invited_idx.get(current) {
                    for child_item in iter {
                        let cid = child_item?.value();
                        if !to_ban.contains(&cid) {
                            to_ban.push(cid);
                        }
                    }
                }
                i += 1;
            }
        }

        let mut purged_invites_count = 0;

        {
            let mut invites_table = write_txn.open_table(db::INVITES_TABLE)?;
            let mut invites_by_owner = write_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;

            for uid in &to_ban {
                if *uid == 1 {
                    continue;
                }

                if let Some(mut user) = db::repository::get_user_by_id_w(&write_txn, *uid)? {
                    user.roles |= FLAG_BANNED;
                    db::repository::save_user(&write_txn, &user)?;
                }

                let mut codes_to_remove = Vec::new();
                if let Ok(iter) = invites_by_owner.get(*uid) {
                    for code_item in iter {
                        let code_val = code_item?;
                        codes_to_remove.push(code_val.value().to_string());
                    }
                }

                for code in codes_to_remove {
                    invites_table.remove(code.as_str())?;
                    invites_by_owner.remove(*uid, code.as_str())?;
                    purged_invites_count += 1;
                }
            }

            let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
            let ts = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_micros(),
            )
            .unwrap_or(0);

            let action = if form.cascade.is_some() {
                format!(
                    "OVERSEER [{}] EXILED BRANCH OF [{}] (CASCADE: {} AGENTS, PURGED {} INVITES)",
                    admin_user.login,
                    target_login,
                    to_ban.len(),
                    purged_invites_count
                )
            } else {
                format!(
                    "OVERSEER [{}] EXILED SINGLE AGENT [{}] (PURGED {} INVITES)",
                    admin_user.login, target_login, purged_invites_count
                )
            };

            audit.insert(ts, action.as_str())?;
        }

        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_unban(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<UnbanForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }
    if form.target_id == admin_user.id || form.target_id == 1 {
        return Err(AppError::Validation);
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;

        let target_login = match db::repository::get_user_by_id_w(&write_txn, form.target_id)? {
            Some(u) => u.login,
            None => return Ok(()),
        };

        let mut to_unban = vec![form.target_id];

        if form.cascade.is_some() {
            let invited_idx = write_txn.open_multimap_table(db::INVITED_BY_INDEX)?;
            let mut i = 0;
            while i < to_unban.len() {
                let current = to_unban[i];
                if let Ok(iter) = invited_idx.get(current) {
                    for child_item in iter {
                        let cid = child_item?.value();
                        if !to_unban.contains(&cid) {
                            to_unban.push(cid);
                        }
                    }
                }
                i += 1;
            }
        }

        {
            for uid in &to_unban {
                if *uid == 1 {
                    continue;
                }
                if let Some(mut user) = db::repository::get_user_by_id_w(&write_txn, *uid)? {
                    user.roles &= !FLAG_BANNED;
                    db::repository::save_user(&write_txn, &user)?;
                }
            }

            let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
            let ts = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_micros(),
            )
            .unwrap_or(0);

            let action = if form.cascade.is_some() {
                format!(
                    "OVERSEER [{}] PARDONED BRANCH OF [{}] (CASCADE: {} AGENTS)",
                    admin_user.login,
                    target_login,
                    to_unban.len()
                )
            } else {
                format!(
                    "OVERSEER [{}] PARDONED SINGLE AGENT [{}]",
                    admin_user.login, target_login
                )
            };

            audit.insert(ts, action.as_str())?;
        }

        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

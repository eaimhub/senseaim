#![deny(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::cast_possible_truncation, clippy::useless_let_if_seq)]

use axum::{
    extract::{Extension, Form, State},
    response::{IntoResponse, Redirect, Response},
};
use redb::{ReadableMultimapTable, ReadableTable};
use serde::Deserialize;

use crate::{
    admin::is_admin,
    auth::crypto,
    db,
    error::AppError,
    models::{self, user::User},
};

#[derive(Deserialize)]
pub struct GrantForm {
    pub target_id: u64,
}

#[derive(Deserialize)]
pub struct RevokeForm {
    pub target_id: u64,
}

pub async fn admin_generate_invite(
    Extension(user): Extension<User>,
    State(state): State<models::AppState>,
) -> Result<Response, AppError> {
    if !is_admin(&user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let invite = crypto::generate_alphanumeric_24();
        let write_txn = db_instance.begin_write()?;
        {
            let mut invites = write_txn.open_table(db::INVITES_TABLE)?;
            invites.insert(invite.as_str(), user.id)?;

            let mut invites_by_owner = write_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;
            invites_by_owner.insert(user.id, invite.as_str())?;

            let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
            let ts = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_micros(),
            )
            .unwrap_or(0);
            let action = format!("OVERSEER [{}] ISSUED ROOT CLEARANCE FOR SELF", user.login);
            audit.insert(ts, action.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_invites_grant_all(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;

        let mut valid_users = Vec::new();
        {
            let users_table = write_txn.open_table(db::USERS_TABLE)?;
            for item in users_table.iter()? {
                let (_, v) = item?;
                let u: User = bincode::deserialize(v.value())?;
                if (u.roles & models::user::FLAG_BANNED) == 0 {
                    valid_users.push(u.id);
                }
            }
        }

        {
            let mut invites = write_txn.open_table(db::INVITES_TABLE)?;
            let mut invites_by_owner = write_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;
            for uid in &valid_users {
                let invite = crypto::generate_alphanumeric_24();
                invites.insert(invite.as_str(), *uid)?;
                invites_by_owner.insert(*uid, invite.as_str())?;
            }

            let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
            let ts = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_micros(),
            )
            .unwrap_or(0);
            let action = format!(
                "OVERSEER [{}] ISSUED 1 CLEARANCE TO ALL {} AGENTS",
                admin_user.login,
                valid_users.len()
            );
            audit.insert(ts, action.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_invites_revoke_one_all(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;

        let to_remove: Vec<(u64, String)> = {
            let invites_table = write_txn.open_table(db::INVITES_TABLE)?;
            let mut owners_map = std::collections::HashMap::new();

            for item in invites_table.iter()? {
                let (k, v) = item?;
                let owner_id = v.value();
                let code = k.value().to_string();
                owners_map.entry(owner_id).or_insert(code);
            }
            owners_map.into_iter().collect()
        };

        {
            let mut invites_table = write_txn.open_table(db::INVITES_TABLE)?;
            let mut invites_by_owner = write_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;
            for (owner_id, code) in &to_remove {
                invites_table.remove(code.as_str())?;
                invites_by_owner.remove(*owner_id, code.as_str())?;
            }

            let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
            let ts = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_micros(),
            )
            .unwrap_or(0);
            let action = format!(
                "OVERSEER [{}] REVOKED 1 CLEARANCE FROM ALL AGENTS (TOTAL: {})",
                admin_user.login,
                to_remove.len()
            );
            audit.insert(ts, action.as_str())?;
        }

        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_invites_revoke_all(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        let mut to_remove = Vec::new();

        {
            let invites_table = write_txn.open_table(db::INVITES_TABLE)?;
            for item in invites_table.iter()? {
                let (k, v) = item?;
                to_remove.push((k.value().to_string(), v.value()));
            }
        }

        {
            let mut invites_table = write_txn.open_table(db::INVITES_TABLE)?;
            let mut invites_by_owner = write_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;
            for (code, owner_id) in &to_remove {
                invites_table.remove(code.as_str())?;
                invites_by_owner.remove(*owner_id, code.as_str())?;
            }

            let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
            let ts = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_micros(),
            )
            .unwrap_or(0);
            let action = format!(
                "OVERSEER [{}] PURGED ALL {} GLOBAL CLEARANCES",
                admin_user.login,
                to_remove.len()
            );
            audit.insert(ts, action.as_str())?;
        }

        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_grant_to_user(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<GrantForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;

        let target_login = match db::repository::get_user_by_id_w(&write_txn, form.target_id)? {
            Some(u) => u.login,
            None => return Ok(()),
        };

        let invite = crypto::generate_alphanumeric_24();

        {
            let mut invites = write_txn.open_table(db::INVITES_TABLE)?;
            invites.insert(invite.as_str(), form.target_id)?;

            let mut invites_by_owner = write_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;
            invites_by_owner.insert(form.target_id, invite.as_str())?;

            let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
            let ts = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_micros(),
            )
            .unwrap_or(0);
            let action = format!(
                "OVERSEER [{}] GRANTED INVITE QUOTA TO AGENT [{}]",
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

pub async fn admin_revoke_from_user(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<RevokeForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;

        let target_login = match db::repository::get_user_by_id_w(&write_txn, form.target_id)? {
            Some(u) => u.login,
            None => return Ok(()),
        };

        let found_code = {
            let invites_by_owner = write_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;
            if let Some(item) = invites_by_owner.get(form.target_id)?.next() {
                Some(item?.value().to_string())
            } else {
                None
            }
        };

        if let Some(code) = found_code {
            let mut invites = write_txn.open_table(db::INVITES_TABLE)?;
            invites.remove(code.as_str())?;

            let mut invites_by_owner = write_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;
            invites_by_owner.remove(form.target_id, code.as_str())?;

            let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
            let ts = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_micros(),
            )
            .unwrap_or(0);
            let action = format!(
                "OVERSEER [{}] REVOKED 1 CLEARANCE FROM AGENT [{}]",
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

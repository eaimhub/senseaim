#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

use axum::{
    extract::{Extension, Form, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use redb::{ReadableTable, ReadableTableMetadata};
use serde::Deserialize;
use std::collections::HashMap;

use crate::{
    admin::{
        ActiveInvite, AdminAuditTemplate, AdminSearchTemplate, AdminTemplate, LogEntry, UserNode,
        UserView, is_admin,
    },
    db,
    error::AppError,
    models::{
        self,
        user::{FLAG_BANNED, ROLE_ADMIN, ROLE_SUBSCRIBER, ROLE_USER, User},
    },
};

#[derive(Deserialize)]
pub struct DashboardQuery {
    pub page: Option<usize>,
}

#[derive(Deserialize)]
pub struct SearchForm {
    pub query: String,
}

#[derive(Deserialize)]
pub struct AuditQuery {
    pub page: Option<usize>,
}

pub async fn admin_dashboard(
    Extension(user): Extension<User>,
    State(state): State<models::AppState>,
    Query(query): Query<DashboardQuery>,
) -> Result<Response, AppError> {
    if !is_admin(&user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    let invites_page = query.page.unwrap_or(0);
    let limit = 40;

    let data = tokio::task::spawn_blocking(move || -> Result<AdminTemplate, AppError> {
        let read_txn = db_instance.begin_read()?;
        let users_table = read_txn.open_table(db::USERS_TABLE)?;
        let invites_table = read_txn.open_table(db::INVITES_TABLE)?;

        let users_count = users_table.len()?;
        let invites_count = invites_table.len()?;

        let mut banned_count = 0;
        let mut users_map = HashMap::new();

        for item in users_table.iter()? {
            let (_, v) = item?;
            let u: User = bincode::deserialize(v.value())?;
            if (u.roles & FLAG_BANNED) != 0 {
                banned_count += 1;
            }
            users_map.insert(u.id, u.login);
        }

        let mut active_invites = Vec::new();
        let skip = invites_page * limit;

        for (count, item) in invites_table.iter()?.enumerate() {
            if active_invites.len() >= limit {
                break;
            }
            if count >= skip {
                let (k, v) = item?;
                let owner_id = v.value();
                let owner_login = users_map
                    .get(&owner_id)
                    .cloned()
                    .unwrap_or_else(|| "UNKNOWN".to_string());

                active_invites.push(ActiveInvite {
                    code: k.value().to_string(),
                    owner: owner_login,
                });
            }
        }

        #[allow(clippy::cast_possible_truncation)]
        let has_next_invites = (skip + limit) < (invites_count as usize);
        let has_prev_invites = invites_page > 0;

        Ok(AdminTemplate {
            users_count,
            invites_count,
            banned_count,
            active_invites,
            invites_page,
            has_next_invites,
            has_prev_invites,
        })
    })
    .await??;

    Ok(data.into_response())
}

type AdminSearchTuple = (Option<User>, Vec<String>, Vec<UserNode>, Vec<u64>);

pub async fn admin_search(
    Extension(user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<SearchForm>,
) -> Result<Response, AppError> {
    if !is_admin(&user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    let query_str = form.query;
    let query_clone = query_str.clone();

    let (found_user, active_invites, tree, post_ids) =
        tokio::task::spawn_blocking(move || -> Result<AdminSearchTuple, AppError> {
            let read_txn = db_instance.begin_read()?;
            let invited_idx = read_txn.open_multimap_table(db::INVITED_BY_INDEX)?;
            let invites_by_owner = read_txn.open_multimap_table(db::INVITES_BY_OWNER_INDEX)?;
            let posts_by_author = read_txn.open_multimap_table(db::POSTS_BY_AUTHOR_INDEX)?;
            let threads_by_author = read_txn.open_multimap_table(db::THREADS_BY_AUTHOR_INDEX)?;

            let target_login = match query_clone.parse::<u64>() {
                Ok(id) => match db::repository::get_user_by_id(&read_txn, id)? {
                    Some(u) => u.login,
                    None => query_clone,
                },
                Err(_) => query_clone,
            };

            let user_opt = db::repository::get_user(&read_txn, target_login.as_str())?;

            let mut tree = Vec::new();
            let mut active_invites = Vec::new();
            let mut post_ids = Vec::new();

            if let Some(ref target) = user_opt {
                if let Ok(iter) = threads_by_author.get(target.id) {
                    for t in iter.flatten() {
                        post_ids.push(t.value());
                    }
                }
                if let Ok(iter) = posts_by_author.get(target.id) {
                    for p in iter.flatten() {
                        post_ids.push(p.value());
                    }
                }
                post_ids.sort_unstable();

                if let Ok(iter) = invites_by_owner.get(target.id) {
                    for code_item in iter {
                        let code_val = code_item?;
                        active_invites.push(code_val.value().to_string());
                    }
                }

                let mut stack = vec![(target.clone(), 0)];
                while let Some((curr, depth)) = stack.pop() {
                    tree.push(UserNode {
                        id: curr.id,
                        login: curr.login.clone(),
                        is_subscriber: (curr.roles & ROLE_SUBSCRIBER) != 0,
                        is_admin: (curr.roles & ROLE_ADMIN) != 0,
                        is_banned: (curr.roles & FLAG_BANNED) != 0,
                        padding_rem: f64::from(depth) * 1.5,
                    });

                    let mut children = Vec::new();
                    if let Ok(iter) = invited_idx.get(curr.id) {
                        for child_item in iter {
                            let child_id = child_item?.value();
                            children.push(child_id);
                        }
                    }

                    let mut child_users = Vec::new();
                    for cid in children {
                        if let Some(u) = db::repository::get_user_by_id(&read_txn, cid)? {
                            child_users.push(u);
                        }
                    }
                    child_users.sort_by_key(|u| u.id);
                    for child in child_users.into_iter().rev() {
                        stack.push((child, depth + 1));
                    }
                }
            }

            Ok((user_opt, active_invites, tree, post_ids))
        })
        .await??;

    let view = found_user.map(|u| UserView {
        id: u.id,
        login: u.login,
        invited_by: u.invited_by,
        is_banned: (u.roles & FLAG_BANNED) != 0,
        is_user: (u.roles & ROLE_USER) != 0,
        is_subscriber: (u.roles & ROLE_SUBSCRIBER) != 0,
        is_admin: (u.roles & ROLE_ADMIN) != 0,
        post_count: u.post_count,
        post_ids,
    });

    let tmpl = AdminSearchTemplate {
        query: query_str,
        user: view,
        active_invites,
        tree,
    };

    Ok(tmpl.into_response())
}

pub async fn admin_audit(
    Extension(user): Extension<User>,
    State(state): State<models::AppState>,
    Query(query): Query<AuditQuery>,
) -> Result<Response, AppError> {
    if !is_admin(&user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    let page = query.page.unwrap_or(0);
    let limit = 20;

    let data = tokio::task::spawn_blocking(move || -> Result<AdminAuditTemplate, AppError> {
        let read_txn = db_instance.begin_read()?;
        let audit_table = read_txn.open_table(db::AUDIT_LOG_TABLE)?;
        let total_count = audit_table.len()?;

        let mut logs = Vec::new();
        let skip = page * limit;

        for (count, item) in audit_table.iter()?.rev().enumerate() {
            if count >= skip + limit {
                break;
            }
            if count >= skip {
                let (k, v) = item?;
                logs.push(LogEntry {
                    timestamp: k.value(),
                    action: v.value().to_string(),
                });
            }
        }

        #[allow(clippy::cast_possible_truncation)]
        let has_next = (skip as u64 + limit as u64) < total_count;
        let has_prev = page > 0;

        Ok(AdminAuditTemplate {
            logs,
            page,
            has_next,
            has_prev,
        })
    })
    .await??;

    Ok(data.into_response())
}

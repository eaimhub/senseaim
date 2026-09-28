#![deny(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    clippy::collapsible_if
)]

use axum::{
    extract::{Extension, Form, State},
    response::{IntoResponse, Redirect, Response},
};
use redb::{ReadableMultimapTable, ReadableTable};
use serde::Deserialize;
use std::collections::HashMap;

use crate::{
    admin::is_admin,
    db,
    error::AppError,
    models::{self, user::User},
};

#[derive(Deserialize)]
pub struct DeleteThreadForm {
    pub target_id: u64,
}

#[derive(Deserialize)]
pub struct DeletePostForm {
    pub target_id: u64,
}

#[derive(Deserialize)]
pub struct PinThreadForm {
    pub target_id: u64,
}

#[derive(Deserialize)]
pub struct LockThreadForm {
    pub target_id: u64,
}

#[derive(Deserialize)]
pub struct SolveThreadForm {
    pub target_id: u64,
}

#[derive(Deserialize)]
pub struct BestPostForm {
    pub target_id: u64,
}

pub async fn admin_delete_thread(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<DeleteThreadForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        let mut purged_posts = 0;

        {
            let mut threads_table = write_txn.open_table(db::THREADS_TABLE)?;
            let mut bump_idx = write_txn.open_table(db::THREADS_BUMP_INDEX)?;
            let mut posts_table = write_txn.open_table(db::POSTS_TABLE)?;
            let mut posts_by_thread = write_txn.open_multimap_table(db::POSTS_BY_THREAD_INDEX)?;
            let mut posts_by_author = write_txn.open_multimap_table(db::POSTS_BY_AUTHOR_INDEX)?;
            let mut threads_by_author =
                write_txn.open_multimap_table(db::THREADS_BY_AUTHOR_INDEX)?;

            let mut polls_table = write_txn.open_table(db::POLLS_TABLE)?;
            let mut poll_votes = write_txn.open_table(db::POLL_VOTES_TABLE)?;
            let mut poll_voters_idx =
                write_txn.open_multimap_table(db::POLL_VOTERS_BY_THREAD_INDEX)?;

            let mut thread_plus = write_txn.open_table(db::THREAD_PLUS_TABLE)?;
            let mut plus_by_thread = write_txn.open_multimap_table(db::PLUS_BY_THREAD_INDEX)?;

            let mut views_table = write_txn.open_table(db::THREAD_VIEWS_TABLE)?;
            let mut views_idx = write_txn.open_multimap_table(db::VIEWS_BY_THREAD_INDEX)?;

            if let Some(thread) = db::repository::get_thread_w(&write_txn, form.target_id)? {
                bump_idx.remove(thread.bumped_at)?;
                threads_table.remove(form.target_id)?;
                threads_by_author.remove(thread.author_id, form.target_id)?;

                let mut posts_to_remove = Vec::new();
                if let Ok(iter) = posts_by_thread.get(form.target_id) {
                    for p_id_val in iter.flatten() {
                        posts_to_remove.push(p_id_val.value());
                    }
                }

                let mut user_decrements = HashMap::new();
                *user_decrements.entry(thread.author_id).or_insert(0u32) += 1;

                for p_id in posts_to_remove {
                    if let Some(p) = db::repository::get_post_w(&write_txn, p_id)? {
                        *user_decrements.entry(p.author_id).or_insert(0u32) += 1;
                        posts_by_author.remove(p.author_id, p_id)?;
                    }
                    posts_table.remove(p_id)?;
                    posts_by_thread.remove(form.target_id, p_id)?;
                    purged_posts += 1;
                }

                if polls_table.get(form.target_id)?.is_some() {
                    polls_table.remove(form.target_id)?;
                    let mut voters = Vec::new();
                    if let Ok(iter) = poll_voters_idx.get(form.target_id) {
                        for v in iter.flatten() {
                            voters.push(v.value());
                        }
                    }
                    for uid in voters {
                        let vote_key = (u128::from(form.target_id) << 64) | u128::from(uid);
                        poll_votes.remove(vote_key)?;
                        poll_voters_idx.remove(form.target_id, uid)?;
                    }
                }

                let mut plus_voters = Vec::new();
                if let Ok(iter) = plus_by_thread.get(form.target_id) {
                    for v in iter.flatten() {
                        plus_voters.push(v.value());
                    }
                }
                for uid in plus_voters {
                    let plus_key = (u128::from(form.target_id) << 64) | u128::from(uid);
                    thread_plus.remove(plus_key)?;
                    plus_by_thread.remove(form.target_id, uid)?;
                }

                let mut viewers = Vec::new();
                if let Ok(iter) = views_idx.get(form.target_id) {
                    for v in iter.flatten() {
                        viewers.push(v.value());
                    }
                }
                for uid in viewers {
                    let view_key = (u128::from(form.target_id) << 64) | u128::from(uid);
                    views_table.remove(view_key)?;
                    views_idx.remove(form.target_id, uid)?;
                }

                let mut ga_parts = write_txn.open_table(db::GIVEAWAY_PARTICIPANTS_TABLE)?;
                let mut ga_idx = write_txn.open_multimap_table(db::GIVEAWAY_PARTICIPANTS_INDEX)?;
                let mut ga_voters = Vec::new();
                if let Ok(iter) = ga_idx.get(form.target_id) {
                    for v in iter.flatten() {
                        ga_voters.push(v.value());
                    }
                }
                for uid in ga_voters {
                    let vote_key = (u128::from(form.target_id) << 64) | u128::from(uid);
                    ga_parts.remove(vote_key)?;
                    ga_idx.remove(form.target_id, uid)?;
                }

                for (uid, dec) in user_decrements {
                    if let Some(mut u) = db::repository::get_user_by_id_w(&write_txn, uid)? {
                        u.post_count = u.post_count.saturating_sub(dec);
                        if uid == thread.author_id {
                            u.plus_count = u.plus_count.saturating_sub(thread.plus_count);
                        }
                        db::repository::save_user(&write_txn, &u)?;
                    }
                }

                let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                let ts = u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_micros(),
                )
                .unwrap_or(0);
                let action = format!(
                    "OVERSEER [{}] PURGED THREAD [{}] AND {} REPLIES",
                    admin_user.login, form.target_id, purged_posts
                );
                audit.insert(ts, action.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_delete_post(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<DeletePostForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        {
            let mut posts_table = write_txn.open_table(db::POSTS_TABLE)?;
            let mut posts_by_thread = write_txn.open_multimap_table(db::POSTS_BY_THREAD_INDEX)?;
            let mut posts_by_author = write_txn.open_multimap_table(db::POSTS_BY_AUTHOR_INDEX)?;

            if let Some(post) = db::repository::get_post_w(&write_txn, form.target_id)? {
                posts_table.remove(form.target_id)?;
                posts_by_thread.remove(post.thread_id, form.target_id)?;
                posts_by_author.remove(post.author_id, form.target_id)?;

                if let Some(mut thread) = db::repository::get_thread_w(&write_txn, post.thread_id)?
                {
                    thread.replies_count = thread.replies_count.saturating_sub(1);
                    if thread.best_post_id == Some(form.target_id) {
                        thread.best_post_id = None;
                    }
                    db::repository::save_thread(&write_txn, &thread)?;
                }

                if let Some(mut u) = db::repository::get_user_by_id_w(&write_txn, post.author_id)? {
                    u.post_count = u.post_count.saturating_sub(1);
                    db::repository::save_user(&write_txn, &u)?;
                }

                let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                let ts = u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_micros(),
                )
                .unwrap_or(0);
                let action = format!(
                    "OVERSEER [{}] PURGED POST [{}] FROM THREAD [{}]",
                    admin_user.login, form.target_id, post.thread_id
                );
                audit.insert(ts, action.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_pin_thread(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<PinThreadForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        {
            let mut bump_idx = write_txn.open_table(db::THREADS_BUMP_INDEX)?;

            if let Some(mut thread) = db::repository::get_thread_w(&write_txn, form.target_id)? {
                let is_pinned = thread.bumped_at > u64::MAX - 2_000_000_000;

                if !is_pinned {
                    bump_idx.remove(thread.bumped_at)?;
                    thread.bumped_at = u64::MAX - thread.id;
                    bump_idx.insert(thread.bumped_at, thread.id)?;

                    db::repository::save_thread(&write_txn, &thread)?;

                    let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                    let ts = u64::try_from(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?
                            .as_micros(),
                    )
                    .unwrap_or(0);
                    let action = format!(
                        "OVERSEER [{}] PINNED THREAD [{}]",
                        admin_user.login, form.target_id
                    );
                    audit.insert(ts, action.as_str())?;
                }
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_unpin_thread(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<PinThreadForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        {
            let mut bump_idx = write_txn.open_table(db::THREADS_BUMP_INDEX)?;

            if let Some(mut thread) = db::repository::get_thread_w(&write_txn, form.target_id)? {
                let is_pinned = thread.bumped_at > u64::MAX - 2_000_000_000;

                if is_pinned {
                    bump_idx.remove(thread.bumped_at)?;
                    let now = u64::try_from(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?
                            .as_micros(),
                    )
                    .unwrap_or(0);
                    let mut unique_bump = now;
                    while bump_idx.get(unique_bump)?.is_some() {
                        unique_bump += 1;
                    }
                    thread.bumped_at = unique_bump;
                    bump_idx.insert(thread.bumped_at, thread.id)?;

                    db::repository::save_thread(&write_txn, &thread)?;

                    let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                    let ts = u64::try_from(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?
                            .as_micros(),
                    )
                    .unwrap_or(0);
                    let action = format!(
                        "OVERSEER [{}] UNPINNED THREAD [{}]",
                        admin_user.login, form.target_id
                    );
                    audit.insert(ts, action.as_str())?;
                }
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_lock_thread(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<LockThreadForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        if let Some(mut thread) = db::repository::get_thread_w(&write_txn, form.target_id)? {
            if !thread.is_locked {
                thread.is_locked = true;
                db::repository::save_thread(&write_txn, &thread)?;

                let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                let ts = u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_micros(),
                )
                .unwrap_or(0);
                let action = format!(
                    "OVERSEER [{}] LOCKED THREAD [{}]",
                    admin_user.login, form.target_id
                );
                audit.insert(ts, action.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_unlock_thread(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<LockThreadForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        if let Some(mut thread) = db::repository::get_thread_w(&write_txn, form.target_id)? {
            if thread.is_locked {
                thread.is_locked = false;
                db::repository::save_thread(&write_txn, &thread)?;

                let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                let ts = u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_micros(),
                )
                .unwrap_or(0);
                let action = format!(
                    "OVERSEER [{}] UNLOCKED THREAD [{}]",
                    admin_user.login, form.target_id
                );
                audit.insert(ts, action.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_solve_thread(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<SolveThreadForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        if let Some(mut thread) = db::repository::get_thread_w(&write_txn, form.target_id)? {
            if !thread.is_solved {
                thread.is_solved = true;
                db::repository::save_thread(&write_txn, &thread)?;

                let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                let ts = u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_micros(),
                )
                .unwrap_or(0);
                let action = format!(
                    "OVERSEER [{}] MARKED THREAD [{}] AS SOLVED",
                    admin_user.login, form.target_id
                );
                audit.insert(ts, action.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_unsolve_thread(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<SolveThreadForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        if let Some(mut thread) = db::repository::get_thread_w(&write_txn, form.target_id)? {
            if thread.is_solved {
                thread.is_solved = false;
                db::repository::save_thread(&write_txn, &thread)?;

                let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                let ts = u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_micros(),
                )
                .unwrap_or(0);
                let action = format!(
                    "OVERSEER [{}] UNMARKED THREAD [{}] AS SOLVED",
                    admin_user.login, form.target_id
                );
                audit.insert(ts, action.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_mark_best(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<BestPostForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;

        if let Some(post) = db::repository::get_post_w(&write_txn, form.target_id)? {
            if let Some(mut thread) = db::repository::get_thread_w(&write_txn, post.thread_id)? {
                thread.best_post_id = Some(post.id);
                db::repository::save_thread(&write_txn, &thread)?;

                let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                let ts = u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_micros(),
                )
                .unwrap_or(0);
                let action = format!(
                    "OVERSEER [{}] MARKED POST [{}] AS BEST IN THREAD [{}]",
                    admin_user.login, post.id, thread.id
                );
                audit.insert(ts, action.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

pub async fn admin_unmark_best(
    Extension(admin_user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<BestPostForm>,
) -> Result<Response, AppError> {
    if !is_admin(&admin_user) {
        return Ok(Redirect::to("/profile").into_response());
    }

    let db_instance = state.db;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;

        if let Some(post) = db::repository::get_post_w(&write_txn, form.target_id)? {
            if let Some(mut thread) = db::repository::get_thread_w(&write_txn, post.thread_id)? {
                if thread.best_post_id == Some(post.id) {
                    thread.best_post_id = None;
                    db::repository::save_thread(&write_txn, &thread)?;

                    let mut audit = write_txn.open_table(db::AUDIT_LOG_TABLE)?;
                    let ts = u64::try_from(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?
                            .as_micros(),
                    )
                    .unwrap_or(0);
                    let action = format!(
                        "OVERSEER [{}] UNMARKED POST [{}] AS BEST IN THREAD [{}]",
                        admin_user.login, post.id, thread.id
                    );
                    audit.insert(ts, action.as_str())?;
                }
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to("/admin").into_response())
}

#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

use axum::{
    extract::{Extension, Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use rand::Rng;
use redb::ReadableTable;
use serde::Deserialize;

use crate::{
    db,
    error::AppError,
    models::{self, board::Post, user::User},
};

#[derive(Deserialize)]
pub struct CreatePostForm {
    pub content: String,
}

pub async fn create_post(
    Extension(user): Extension<User>,
    Path(thread_id): Path<u64>,
    State(state): State<models::AppState>,
    Form(form): Form<CreatePostForm>,
) -> Result<Response, AppError> {
    let db_instance = state.db;
    let content = form.content.trim().to_string();

    if content.is_empty() || content.len() > 4096 {
        return Err(AppError::Validation);
    }

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        let now = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_micros(),
        )
        .unwrap_or(0);
        let now_sec = now / 1_000_000;

        let rl_key = format!("post_{}", user.id);
        db::repository::check_rate_limit(&write_txn, &rl_key, 30, 1, now_sec)?;

        {
            let mut bump_idx = write_txn.open_table(db::THREADS_BUMP_INDEX)?;
            let mut posts = write_txn.open_table(db::POSTS_TABLE)?;
            let mut posts_by_thread = write_txn.open_multimap_table(db::POSTS_BY_THREAD_INDEX)?;
            let mut posts_by_author = write_txn.open_multimap_table(db::POSTS_BY_AUTHOR_INDEX)?;

            let Some(mut thread) = db::repository::get_thread_w(&write_txn, thread_id)? else {
                return Err(AppError::NotFound);
            };

            if thread.is_locked {
                return Err(AppError::Validation);
            }

            let post_id = rand::thread_rng().gen_range(100_000_000..=999_999_999);
            let post = Post {
                id: post_id,
                thread_id,
                author_id: user.id,
                content,
                created_at: now,
            };

            let p_bytes = bincode::serialize(&post)?;
            posts.insert(post_id, p_bytes.as_slice())?;
            posts_by_thread.insert(thread_id, post_id)?;
            posts_by_author.insert(user.id, post_id)?;

            thread.replies_count = thread.replies_count.saturating_add(1);
            let is_pinned = thread.bumped_at > u64::MAX - 2_000_000_000;

            if thread.replies_count <= 300 && !is_pinned {
                bump_idx.remove(thread.bumped_at)?;
                let mut unique_bump = now;
                while bump_idx.get(unique_bump)?.is_some() {
                    unique_bump += 1;
                }
                thread.bumped_at = unique_bump;
                bump_idx.insert(unique_bump, thread_id)?;
            }

            db::repository::save_thread(&write_txn, &thread)?;
        }

        {
            let mut users = write_txn.open_table(db::USERS_TABLE)?;
            let user_bytes_opt = users.get(user.login.as_str())?.map(|v| v.value().to_vec());

            if let Some(u_bytes) = user_bytes_opt
                && let Ok(mut u) = bincode::deserialize::<User>(&u_bytes)
            {
                u.post_count = u.post_count.saturating_add(1);
                users.insert(user.login.as_str(), bincode::serialize(&u)?.as_slice())?;
            }
            drop(users);
        }

        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to(&format!("/board/{thread_id}")).into_response())
}

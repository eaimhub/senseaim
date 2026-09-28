#![deny(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::too_many_lines)]

use axum::{
    extract::{Extension, Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use rand::Rng;
use redb::{ReadableTable, ReadableTableMetadata};
use serde::Deserialize;

use crate::{
    board::{BoardTemplate, PollOptionView, PollView, PostView, ThreadTemplate, ThreadView},
    db,
    error::AppError,
    models::{
        self,
        board::{Poll, Post, Thread},
        user::{ROLE_USER, User},
    },
};

#[derive(Deserialize)]
pub struct CreateThreadForm {
    pub content: String,
    pub poll_option_1: Option<String>,
    pub poll_option_2: Option<String>,
    pub poll_option_3: Option<String>,
    pub poll_option_4: Option<String>,
    pub poll_option_5: Option<String>,
    pub poll_option_6: Option<String>,
    pub gw_winners: Option<String>,
    pub gw_participants: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchContentForm {
    pub query: String,
}

pub async fn board_page(
    Extension(_user): Extension<User>,
    State(state): State<models::AppState>,
) -> Result<Response, AppError> {
    let db_instance = state.db;
    let data = tokio::task::spawn_blocking(move || -> Result<BoardTemplate, AppError> {
        let read_txn = db_instance.begin_read()?;
        let bump_idx = read_txn.open_table(db::THREADS_BUMP_INDEX)?;
        let threads_table = read_txn.open_table(db::THREADS_TABLE)?;
        let posts_idx = read_txn.open_multimap_table(db::POSTS_BY_THREAD_INDEX)?;
        let posts_table = read_txn.open_table(db::POSTS_TABLE)?;
        let polls_table = read_txn.open_table(db::POLLS_TABLE)?;

        let total_threads = threads_table.len()?;
        let mut threads = Vec::new();

        for item in bump_idx.iter()?.rev().take(50) {
            let (_, v) = item?;
            let thread_id = v.value();

            if let Some(t_bytes) = threads_table.get(thread_id)? {
                let thread: Thread = bincode::deserialize(t_bytes.value())?;
                let is_pinned = thread.bumped_at > u64::MAX - 2_000_000_000;
                let is_locked = thread.is_locked;
                let is_solved = thread.is_solved;
                let has_poll = polls_table.get(thread_id)?.is_some();

                let op_role =
                    if let Some(u) = db::repository::get_user_by_id(&read_txn, thread.author_id)? {
                        u.roles
                    } else {
                        ROLE_USER
                    };

                let mut all_posts = Vec::new();
                if let Ok(iter) = posts_idx.get(thread_id) {
                    for p_id_item in iter {
                        let p_id_val = p_id_item?;
                        let p_id = p_id_val.value();
                        if let Some(p_bytes) = posts_table.get(p_id)?
                            && let Ok(post) = bincode::deserialize::<Post>(p_bytes.value())
                        {
                            all_posts.push(post);
                        }
                    }
                }
                all_posts.sort_by_key(|p| p.created_at);

                let omitted = u32::try_from(all_posts.len().saturating_sub(3)).unwrap_or(0);

                let recent: Vec<_> = all_posts.into_iter().rev().take(3).collect();
                let mut recent_views = Vec::new();
                for p in recent.into_iter().rev() {
                    let p_role =
                        if let Some(u) = db::repository::get_user_by_id(&read_txn, p.author_id)? {
                            u.roles
                        } else {
                            ROLE_USER
                        };
                    let is_best = Some(p.id) == thread.best_post_id;
                    recent_views.push(PostView {
                        post: p,
                        author_role: p_role,
                        is_best,
                    });
                }

                threads.push(ThreadView {
                    thread,
                    op_role,
                    recent_posts: recent_views,
                    omitted_posts: omitted,
                    is_pinned,
                    is_locked,
                    is_solved,
                    has_poll,
                });
            }
        }

        Ok(BoardTemplate {
            threads,
            total_threads,
        })
    })
    .await??;

    Ok(data.into_response())
}

pub async fn thread_page(
    Extension(user): Extension<User>,
    Path(thread_id): Path<u64>,
    State(state): State<models::AppState>,
) -> Result<Response, AppError> {
    let db_instance = state.db;
    let current_user_id = user.id;

    let data =
        tokio::task::spawn_blocking(move || -> Result<ThreadTemplate, AppError> {
            let (
                mut thread,
                op_role,
                posts,
                poll,
                user_has_plus,
                user_participating,
                requires_update,
            ) = {
                let read_txn = db_instance.begin_read()?;
                let posts_idx = read_txn.open_multimap_table(db::POSTS_BY_THREAD_INDEX)?;
                let polls_table = read_txn.open_table(db::POLLS_TABLE)?;
                let poll_votes_table = read_txn.open_table(db::POLL_VOTES_TABLE)?;
                let thread_plus = read_txn.open_table(db::THREAD_PLUS_TABLE)?;
                let thread_views = read_txn.open_table(db::THREAD_VIEWS_TABLE)?;

                let thread =
                    db::repository::get_thread(&read_txn, thread_id)?.ok_or(AppError::NotFound)?;

                let view_key = (u128::from(thread_id) << 64) | u128::from(current_user_id);
                let requires_update = thread_views.get(view_key)?.is_none();

                let plus_key = (u128::from(thread_id) << 64) | u128::from(current_user_id);
                let user_has_plus = thread_plus.get(plus_key)?.is_some();

                let user_participating = if thread.giveaway_active {
                    let ga_parts = read_txn.open_table(db::GIVEAWAY_PARTICIPANTS_TABLE)?;
                    let part_key: u128 =
                        (u128::from(thread_id) << 64) | u128::from(current_user_id);
                    ga_parts.get(part_key)?.is_some()
                } else {
                    false
                };

                let op_role =
                    if let Some(u) = db::repository::get_user_by_id(&read_txn, thread.author_id)? {
                        u.roles
                    } else {
                        ROLE_USER
                    };

                let poll_view = if let Some(poll_bytes) = polls_table.get(thread_id)? {
                    let poll: Poll = bincode::deserialize(poll_bytes.value())?;
                    let vote_key: u128 =
                        (u128::from(thread_id) << 64) | u128::from(current_user_id);

                    let user_vote = poll_votes_table.get(vote_key)?.map(|g| g.value() as usize);

                    let mut options_view = Vec::new();
                    for (i, opt) in poll.options.into_iter().enumerate() {
                        let votes = poll.votes[i];
                        let percentage = if poll.total_votes > 0 {
                            u32::try_from(
                                u64::from(votes).saturating_mul(100) / u64::from(poll.total_votes),
                            )
                            .unwrap_or(0)
                        } else {
                            0
                        };
                        let is_users_choice = user_vote == Some(i);

                        options_view.push(PollOptionView {
                            text: opt,
                            votes,
                            percentage,
                            is_users_choice,
                            index: i,
                        });
                    }

                    Some(PollView {
                        options: options_view,
                        total_votes: poll.total_votes,
                        user_voted: user_vote.is_some(),
                    })
                } else {
                    None
                };

                let mut all_posts = Vec::new();
                if let Ok(iter) = posts_idx.get(thread_id) {
                    for p_id_item in iter {
                        let p_id_val = p_id_item?;
                        let p_id = p_id_val.value();
                        if let Some(post) = db::repository::get_post(&read_txn, p_id)? {
                            all_posts.push(post);
                        }
                    }
                }
                all_posts.sort_by_key(|p| p.created_at);

                let mut posts_views = Vec::new();
                for p in all_posts {
                    let p_role =
                        if let Some(u) = db::repository::get_user_by_id(&read_txn, p.author_id)? {
                            u.roles
                        } else {
                            ROLE_USER
                        };
                    let is_best = Some(p.id) == thread.best_post_id;
                    posts_views.push(PostView {
                        post: p,
                        author_role: p_role,
                        is_best,
                    });
                }

                (
                    thread,
                    op_role,
                    posts_views,
                    poll_view,
                    user_has_plus,
                    user_participating,
                    requires_update,
                )
            };

            if requires_update {
                let write_txn = db_instance.begin_write()?;
                {
                    let mut thread_views = write_txn.open_table(db::THREAD_VIEWS_TABLE)?;
                    let mut views_idx = write_txn.open_multimap_table(db::VIEWS_BY_THREAD_INDEX)?;
                    let view_key = (u128::from(thread_id) << 64) | u128::from(current_user_id);

                    if thread_views.get(view_key)?.is_none() {
                        thread_views.insert(view_key, 1)?;
                        views_idx.insert(thread_id, current_user_id)?;

                        let mut threads_table = write_txn.open_table(db::THREADS_TABLE)?;
                        let t_bytes = threads_table.get(thread_id)?.map(|g| g.value().to_vec());

                        if let Some(bytes) = t_bytes {
                            let mut t: Thread = bincode::deserialize(&bytes)?;
                            t.views_count = t.views_count.saturating_add(1);
                            threads_table.insert(thread_id, bincode::serialize(&t)?.as_slice())?;
                            thread.views_count = t.views_count;
                        }
                    }
                }
                write_txn.commit()?;
            }

            let is_pinned = thread.bumped_at > u64::MAX - 2_000_000_000;
            let is_locked = thread.is_locked;
            let is_solved = thread.is_solved;

            Ok(ThreadTemplate {
                thread,
                op_role,
                posts,
                poll,
                user_has_plus,
                user_participating,
                is_pinned,
                is_locked,
                is_solved,
            })
        })
        .await??;

    Ok(data.into_response())
}

pub async fn create_thread(
    Extension(user): Extension<User>,
    State(state): State<models::AppState>,
    Form(form): Form<CreateThreadForm>,
) -> Result<Response, AppError> {
    let db_instance = state.db;
    let content = form.content.trim().to_string();

    if content.is_empty() || content.len() > 4096 {
        return Err(AppError::Validation);
    }

    let raw_options = vec![
        form.poll_option_1,
        form.poll_option_2,
        form.poll_option_3,
        form.poll_option_4,
        form.poll_option_5,
        form.poll_option_6,
    ];

    let valid_poll_options: Vec<String> = raw_options
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        let now = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_micros(),
        )
        .unwrap_or(0);
        let now_sec = now / 1_000_000;

        let rl_key = format!("thread_{}", user.id);
        db::repository::check_rate_limit(&write_txn, &rl_key, 900, 1, now_sec)?;

        let mut giveaway_active = false;
        let mut giveaway_winners_count = 0;
        let mut giveaway_target_participants = 0;
        let giveaway_current_participants = 0;
        let giveaway_winners = Vec::new();

        let winners_str = form.gw_winners.as_deref().unwrap_or("").trim();
        let participants_str = form.gw_participants.as_deref().unwrap_or("").trim();

        if !winners_str.is_empty() && !participants_str.is_empty() {
            if let (Ok(w), Ok(p)) = (winners_str.parse::<u32>(), participants_str.parse::<u32>()) {
                if (1..=10).contains(&w) && (10..=100).contains(&p) && w < p {
                    giveaway_active = true;
                    giveaway_winners_count = w;
                    giveaway_target_participants = p;
                } else {
                    return Err(AppError::Validation);
                }
            } else {
                return Err(AppError::Validation);
            }
        } else if !winners_str.is_empty() || !participants_str.is_empty() {
            return Err(AppError::Validation);
        }

        {
            let mut bump_idx = write_txn.open_table(db::THREADS_BUMP_INDEX)?;
            let mut threads_by_author =
                write_txn.open_multimap_table(db::THREADS_BY_AUTHOR_INDEX)?;

            let thread_id = rand::thread_rng().gen_range(100_000_000..=999_999_999);

            let mut unique_bump = now;
            while bump_idx.get(unique_bump)?.is_some() {
                unique_bump += 1;
            }

            let thread = Thread {
                id: thread_id,
                author_id: user.id,
                content,
                created_at: now,
                bumped_at: unique_bump,
                replies_count: 0,
                views_count: 0,
                plus_count: 0,
                is_locked: false,
                is_solved: false,
                best_post_id: None,
                giveaway_active,
                giveaway_winners_count,
                giveaway_target_participants,
                giveaway_current_participants,
                giveaway_winners,
            };

            db::repository::save_thread(&write_txn, &thread)?;
            bump_idx.insert(unique_bump, thread_id)?;
            threads_by_author.insert(user.id, thread_id)?;

            if valid_poll_options.len() > 1 {
                let poll = Poll {
                    thread_id,
                    options: valid_poll_options.clone(),
                    votes: vec![0; valid_poll_options.len()],
                    total_votes: 0,
                };
                let mut polls_table = write_txn.open_table(db::POLLS_TABLE)?;
                polls_table.insert(thread_id, bincode::serialize(&poll)?.as_slice())?;
            }
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

    Ok(Redirect::to("/board").into_response())
}

pub async fn search_content(
    State(state): State<models::AppState>,
    Form(form): Form<SearchContentForm>,
) -> Result<Response, AppError> {
    let db_instance = state.db;
    let target_id = form
        .query
        .trim()
        .parse::<u64>()
        .map_err(|_| AppError::Validation)?;

    let target_thread_id = tokio::task::spawn_blocking(move || -> Result<u64, AppError> {
        let read_txn = db_instance.begin_read()?;

        if db::repository::get_thread(&read_txn, target_id)?.is_some() {
            return Ok(target_id);
        }

        if let Some(post) = db::repository::get_post(&read_txn, target_id)? {
            return Ok(post.thread_id);
        }

        Err(AppError::NotFound)
    })
    .await??;

    Ok(Redirect::to(&format!("/board/{target_thread_id}")).into_response())
}

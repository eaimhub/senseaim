#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

use axum::{
    extract::{Extension, Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use rand::seq::SliceRandom;
use redb::{ReadableMultimapTable, ReadableTable};
use serde::Deserialize;

use crate::{
    db,
    error::AppError,
    models::{
        self,
        board::{Poll, Thread},
        user::User,
    },
};

#[derive(Deserialize)]
pub struct VoteForm {
    pub option_index: usize,
}

#[derive(Deserialize)]
pub struct PlusForm {
    pub source: String,
}

pub async fn vote_poll(
    Extension(user): Extension<User>,
    Path(thread_id): Path<u64>,
    State(state): State<models::AppState>,
    Form(form): Form<VoteForm>,
) -> Result<Response, AppError> {
    let db_instance = state.db;

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        {
            let mut polls = write_txn.open_table(db::POLLS_TABLE)?;
            let mut poll_votes = write_txn.open_table(db::POLL_VOTES_TABLE)?;
            let mut poll_voters_idx =
                write_txn.open_multimap_table(db::POLL_VOTERS_BY_THREAD_INDEX)?;

            let vote_key: u128 = (u128::from(thread_id) << 64) | u128::from(user.id);

            if poll_votes.get(vote_key)?.is_some() {
                return Err(AppError::Validation);
            }

            let poll_bytes_opt = polls.get(thread_id)?.map(|v| v.value().to_vec());

            if let Some(poll_bytes) = poll_bytes_opt {
                let mut poll: Poll = bincode::deserialize(&poll_bytes)?;

                if form.option_index >= poll.options.len() {
                    return Err(AppError::Validation);
                }

                poll.votes[form.option_index] = poll.votes[form.option_index].saturating_add(1);
                poll.total_votes = poll.total_votes.saturating_add(1);

                #[allow(clippy::cast_possible_truncation)]
                poll_votes.insert(vote_key, form.option_index as u8)?;
                poll_voters_idx.insert(thread_id, user.id)?;
                polls.insert(thread_id, bincode::serialize(&poll)?.as_slice())?;
            } else {
                return Err(AppError::NotFound);
            }
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to(&format!("/board/{thread_id}")).into_response())
}

pub async fn toggle_plus(
    Extension(user): Extension<User>,
    Path(thread_id): Path<u64>,
    State(state): State<models::AppState>,
    Form(form): Form<PlusForm>,
) -> Result<Response, AppError> {
    let db_instance = state.db;
    let user_id = user.id;

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        {
            let mut threads = write_txn.open_table(db::THREADS_TABLE)?;
            let mut users = write_txn.open_table(db::USERS_TABLE)?;
            let users_by_id = write_txn.open_table(db::USERS_BY_ID_TABLE)?;

            let mut thread_plus = write_txn.open_table(db::THREAD_PLUS_TABLE)?;
            let mut plus_by_thread = write_txn.open_multimap_table(db::PLUS_BY_THREAD_INDEX)?;

            let t_bytes = threads
                .get(thread_id)?
                .ok_or(AppError::NotFound)?
                .value()
                .to_vec();
            let mut thread: Thread = bincode::deserialize(&t_bytes)?;

            let author_login = users_by_id
                .get(thread.author_id)?
                .ok_or(AppError::NotFound)?
                .value()
                .to_string();
            let author_bytes = users
                .get(author_login.as_str())?
                .ok_or(AppError::NotFound)?
                .value()
                .to_vec();
            let mut author: User = bincode::deserialize(&author_bytes)?;

            let plus_key: u128 = (u128::from(thread_id) << 64) | u128::from(user_id);

            if thread_plus.get(plus_key)?.is_some() {
                thread_plus.remove(plus_key)?;
                plus_by_thread.remove(thread_id, user_id)?;

                thread.plus_count = thread.plus_count.saturating_sub(1);
                author.plus_count = author.plus_count.saturating_sub(1);
            } else {
                thread_plus.insert(plus_key, 1)?;
                plus_by_thread.insert(thread_id, user_id)?;

                thread.plus_count = thread.plus_count.saturating_add(1);
                author.plus_count = author.plus_count.saturating_add(1);
            }

            threads.insert(thread_id, bincode::serialize(&thread)?.as_slice())?;
            users.insert(
                author_login.as_str(),
                bincode::serialize(&author)?.as_slice(),
            )?;
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    let redirect_url = if form.source == "board" {
        "/board".to_string()
    } else {
        format!("/board/{thread_id}")
    };

    Ok(Redirect::to(&redirect_url).into_response())
}

pub async fn participate_giveaway(
    Extension(user): Extension<User>,
    Path(thread_id): Path<u64>,
    State(state): State<models::AppState>,
) -> Result<Response, AppError> {
    let db_instance = state.db;
    let user_id = user.id;

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let write_txn = db_instance.begin_write()?;
        {
            let mut threads = write_txn.open_table(db::THREADS_TABLE)?;
            let t_bytes = threads
                .get(thread_id)?
                .ok_or(AppError::NotFound)?
                .value()
                .to_vec();
            let mut thread: Thread = bincode::deserialize(&t_bytes)?;

            if !thread.giveaway_active || !thread.giveaway_winners.is_empty() {
                return Err(AppError::Validation);
            }

            let mut ga_parts = write_txn.open_table(db::GIVEAWAY_PARTICIPANTS_TABLE)?;
            let mut ga_idx = write_txn.open_multimap_table(db::GIVEAWAY_PARTICIPANTS_INDEX)?;

            let part_key: u128 = (u128::from(thread_id) << 64) | u128::from(user_id);

            if ga_parts.get(part_key)?.is_some() {
                return Err(AppError::Validation);
            }

            ga_parts.insert(part_key, 1)?;
            ga_idx.insert(thread_id, user_id)?;

            thread.giveaway_current_participants =
                thread.giveaway_current_participants.saturating_add(1);

            if thread.giveaway_current_participants >= thread.giveaway_target_participants {
                let mut all_parts = Vec::new();
                if let Ok(iter) = ga_idx.get(thread_id) {
                    for p_val in iter.flatten() {
                        all_parts.push(p_val.value());
                    }
                }

                let mut rng = rand::thread_rng();
                all_parts.shuffle(&mut rng);

                thread.giveaway_winners = all_parts
                    .into_iter()
                    .take(thread.giveaway_winners_count as usize)
                    .collect();
            }

            threads.insert(thread_id, bincode::serialize(&thread)?.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    })
    .await??;

    Ok(Redirect::to(&format!("/board/{thread_id}")).into_response())
}

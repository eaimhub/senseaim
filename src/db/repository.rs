#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

use crate::{
    db,
    error::AppError,
    models::{
        board::{Post, Thread},
        user::User,
    },
};
use redb::{ReadTransaction, ReadableTable, WriteTransaction};

#[inline]
pub fn get_user(txn: &ReadTransaction, login: &str) -> Result<Option<User>, AppError> {
    let table = txn.open_table(db::USERS_TABLE)?;
    if let Some(guard) = table.get(login)? {
        let user: User = bincode::deserialize(guard.value())?;
        return Ok(Some(user));
    }
    Ok(None)
}

#[inline]
pub fn get_user_w(txn: &WriteTransaction, login: &str) -> Result<Option<User>, AppError> {
    let table = txn.open_table(db::USERS_TABLE)?;
    if let Some(guard) = table.get(login)? {
        let user: User = bincode::deserialize(guard.value())?;
        return Ok(Some(user));
    }
    Ok(None)
}

#[inline]
pub fn get_user_by_id(txn: &ReadTransaction, id: u64) -> Result<Option<User>, AppError> {
    let table = txn.open_table(db::USERS_BY_ID_TABLE)?;
    if let Some(guard) = table.get(id)? {
        return get_user(txn, guard.value());
    }
    Ok(None)
}

#[inline]
pub fn get_user_by_id_w(txn: &WriteTransaction, id: u64) -> Result<Option<User>, AppError> {
    let table = txn.open_table(db::USERS_BY_ID_TABLE)?;
    if let Some(guard) = table.get(id)? {
        return get_user_w(txn, guard.value());
    }
    Ok(None)
}

#[inline]
pub fn save_user(txn: &WriteTransaction, user: &User) -> Result<(), AppError> {
    let mut table = txn.open_table(db::USERS_TABLE)?;
    let bytes = bincode::serialize(user)?;
    table.insert(user.login.as_str(), bytes.as_slice())?;
    Ok(())
}

#[inline]
pub fn get_thread(txn: &ReadTransaction, id: u64) -> Result<Option<Thread>, AppError> {
    let table = txn.open_table(db::THREADS_TABLE)?;
    if let Some(guard) = table.get(id)? {
        let thread: Thread = bincode::deserialize(guard.value())?;
        return Ok(Some(thread));
    }
    Ok(None)
}

#[inline]
pub fn get_thread_w(txn: &WriteTransaction, id: u64) -> Result<Option<Thread>, AppError> {
    let table = txn.open_table(db::THREADS_TABLE)?;
    if let Some(guard) = table.get(id)? {
        let thread: Thread = bincode::deserialize(guard.value())?;
        return Ok(Some(thread));
    }
    Ok(None)
}

#[inline]
pub fn save_thread(txn: &WriteTransaction, thread: &Thread) -> Result<(), AppError> {
    let mut table = txn.open_table(db::THREADS_TABLE)?;
    let bytes = bincode::serialize(thread)?;
    table.insert(thread.id, bytes.as_slice())?;
    Ok(())
}

#[inline]
pub fn get_post(txn: &ReadTransaction, id: u64) -> Result<Option<Post>, AppError> {
    let table = txn.open_table(db::POSTS_TABLE)?;
    if let Some(guard) = table.get(id)? {
        let post: Post = bincode::deserialize(guard.value())?;
        return Ok(Some(post));
    }
    Ok(None)
}

#[inline]
pub fn get_post_w(txn: &WriteTransaction, id: u64) -> Result<Option<Post>, AppError> {
    let table = txn.open_table(db::POSTS_TABLE)?;
    if let Some(guard) = table.get(id)? {
        let post: Post = bincode::deserialize(guard.value())?;
        return Ok(Some(post));
    }
    Ok(None)
}

#[inline]
pub fn check_rate_limit(
    txn: &WriteTransaction,
    key: &str,
    window_seconds: u64,
    max_attempts: u32,
    now: u64,
) -> Result<(), AppError> {
    let mut table = txn.open_table(db::RATE_LIMITS_TABLE)?;

    let record_opt = { table.get(key)?.map(|g| g.value()) };

    if let Some((timestamp, count)) = record_opt {
        if now.saturating_sub(timestamp) < window_seconds {
            if count >= max_attempts {
                return Err(AppError::RateLimit);
            }
            table.insert(key, (timestamp, count + 1))?;
        } else {
            table.insert(key, (now, 1))?;
        }
    } else {
        table.insert(key, (now, 1))?;
    }
    Ok(())
}

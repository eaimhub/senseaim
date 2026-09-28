#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

pub mod repository;

use crate::error::AppError;
use rand::Rng;
use redb::{Database, MultimapTableDefinition, ReadableTableMetadata, TableDefinition};

pub const USERS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("users");
pub const USERS_BY_ID_TABLE: TableDefinition<u64, &str> = TableDefinition::new("users_by_id");
pub const INVITES_TABLE: TableDefinition<&str, u64> = TableDefinition::new("invites");
pub const SESSIONS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("sessions");
pub const RATE_LIMITS_TABLE: TableDefinition<&str, (u64, u32)> =
    TableDefinition::new("rate_limits");
pub const AUDIT_LOG_TABLE: TableDefinition<u64, &str> = TableDefinition::new("audit_log");

pub const INVITED_BY_INDEX: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("invited_by_index");
pub const INVITES_BY_OWNER_INDEX: MultimapTableDefinition<u64, &str> =
    MultimapTableDefinition::new("invites_by_owner_index");

pub const THREADS_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("threads");
pub const POSTS_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("posts");
pub const THREADS_BUMP_INDEX: TableDefinition<u64, u64> =
    TableDefinition::new("threads_bump_index");
pub const POSTS_BY_THREAD_INDEX: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("posts_by_thread_index");
pub const POSTS_BY_AUTHOR_INDEX: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("posts_by_author_index");
pub const THREADS_BY_AUTHOR_INDEX: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("threads_by_author_index");

pub const POLLS_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("polls");
pub const POLL_VOTES_TABLE: TableDefinition<u128, u8> = TableDefinition::new("poll_votes");
pub const POLL_VOTERS_BY_THREAD_INDEX: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("poll_voters_by_thread");

pub const THREAD_PLUS_TABLE: TableDefinition<u128, u8> = TableDefinition::new("thread_plus");
pub const PLUS_BY_THREAD_INDEX: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("plus_by_thread_index");

pub const THREAD_SOLUTIONS_TABLE: TableDefinition<u64, u64> =
    TableDefinition::new("thread_solutions");

pub const GIVEAWAY_PARTICIPANTS_TABLE: TableDefinition<u128, u8> =
    TableDefinition::new("giveaway_participants");
pub const GIVEAWAY_PARTICIPANTS_INDEX: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("giveaway_participants_index");

pub const THREAD_VIEWS_TABLE: TableDefinition<u128, u8> = TableDefinition::new("thread_views");
pub const VIEWS_BY_THREAD_INDEX: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("views_by_thread_index");

pub fn init_db() -> Result<Database, AppError> {
    let db = Database::create("senseaim.redb")?;
    let write_txn = db.begin_write()?;
    {
        let _ = write_txn.open_table(USERS_TABLE)?;
        let _ = write_txn.open_table(USERS_BY_ID_TABLE)?;
        let _ = write_txn.open_table(INVITES_TABLE)?;
        let _ = write_txn.open_table(SESSIONS_TABLE)?;
        let _ = write_txn.open_table(RATE_LIMITS_TABLE)?;
        let _ = write_txn.open_table(AUDIT_LOG_TABLE)?;
        let _ = write_txn.open_multimap_table(INVITED_BY_INDEX)?;
        let _ = write_txn.open_multimap_table(INVITES_BY_OWNER_INDEX)?;

        let _ = write_txn.open_table(THREADS_TABLE)?;
        let _ = write_txn.open_table(POSTS_TABLE)?;
        let _ = write_txn.open_table(THREADS_BUMP_INDEX)?;
        let _ = write_txn.open_multimap_table(POSTS_BY_THREAD_INDEX)?;
        let _ = write_txn.open_multimap_table(POSTS_BY_AUTHOR_INDEX)?;
        let _ = write_txn.open_multimap_table(THREADS_BY_AUTHOR_INDEX)?;

        let _ = write_txn.open_table(POLLS_TABLE)?;
        let _ = write_txn.open_table(POLL_VOTES_TABLE)?;
        let _ = write_txn.open_multimap_table(POLL_VOTERS_BY_THREAD_INDEX)?;

        let _ = write_txn.open_table(THREAD_PLUS_TABLE)?;
        let _ = write_txn.open_multimap_table(PLUS_BY_THREAD_INDEX)?;

        let _ = write_txn.open_table(THREAD_SOLUTIONS_TABLE)?;

        let _ = write_txn.open_table(GIVEAWAY_PARTICIPANTS_TABLE)?;
        let _ = write_txn.open_multimap_table(GIVEAWAY_PARTICIPANTS_INDEX)?;

        let _ = write_txn.open_table(THREAD_VIEWS_TABLE)?;
        let _ = write_txn.open_multimap_table(VIEWS_BY_THREAD_INDEX)?;
    }
    write_txn.commit()?;
    Ok(db)
}

pub fn run_first_time_setup(db: &Database) -> Result<(), AppError> {
    let write_txn = db.begin_write()?;
    {
        let mut invites = write_txn.open_table(INVITES_TABLE)?;
        let users = write_txn.open_table(USERS_TABLE)?;

        if users.is_empty()? && invites.is_empty()? {
            let charset: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            let mut rng = rand::thread_rng();
            let invite: String = (0..24)
                .map(|_| charset[rng.gen_range(0..charset.len())] as char)
                .collect();

            invites.insert(invite.as_str(), 1)?;

            let mut invites_by_owner = write_txn.open_multimap_table(INVITES_BY_OWNER_INDEX)?;
            invites_by_owner.insert(1, invite.as_str())?;

            tracing::info!("MASTER INVITE: {}", invite);
        }
    }
    write_txn.commit()?;
    Ok(())
}

#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Thread {
    pub id: u64,
    pub author_id: u64,
    pub content: String,
    pub created_at: u64,
    pub bumped_at: u64,
    pub replies_count: u32,
    pub views_count: u32,
    pub plus_count: u32,
    pub is_locked: bool,
    pub is_solved: bool,
    pub best_post_id: Option<u64>,
    pub giveaway_active: bool,
    pub giveaway_winners_count: u32,
    pub giveaway_target_participants: u32,
    pub giveaway_current_participants: u32,
    pub giveaway_winners: Vec<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Post {
    pub id: u64,
    pub thread_id: u64,
    pub author_id: u64,
    pub content: String,
    pub created_at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Poll {
    pub thread_id: u64,
    pub options: Vec<String>,
    pub votes: Vec<u32>,
    pub total_votes: u32,
}

#![deny(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::struct_excessive_bools)]

pub mod interactions;
pub mod posts;
pub mod threads;

use crate::models::board::{Post, Thread};
use askama::Template;

#[derive(Template)]
#[template(path = "board.html")]
pub struct BoardTemplate {
    pub threads: Vec<ThreadView>,
    pub total_threads: u64,
}

pub struct ThreadView {
    pub thread: Thread,
    pub op_role: u8,
    pub recent_posts: Vec<PostView>,
    pub omitted_posts: u32,
    pub is_pinned: bool,
    pub is_locked: bool,
    pub is_solved: bool,
    pub has_poll: bool,
}

#[derive(Template)]
#[template(path = "thread.html")]
pub struct ThreadTemplate {
    pub thread: Thread,
    pub op_role: u8,
    pub posts: Vec<PostView>,
    pub poll: Option<PollView>,
    pub user_has_plus: bool,
    pub user_participating: bool,
    pub is_pinned: bool,
    pub is_locked: bool,
    pub is_solved: bool,
}

pub struct PostView {
    pub post: Post,
    pub author_role: u8,
    pub is_best: bool,
}

pub struct PollView {
    pub options: Vec<PollOptionView>,
    pub total_votes: u32,
    pub user_voted: bool,
}

pub struct PollOptionView {
    pub text: String,
    pub votes: u32,
    pub percentage: u32,
    pub is_users_choice: bool,
    pub index: usize,
}

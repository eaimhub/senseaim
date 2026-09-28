#![deny(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::struct_excessive_bools)]

pub mod dashboard;
pub mod invites;
pub mod moderation;
pub mod users;

use askama::Template;

use crate::models::user::{ROLE_ADMIN, User};

#[derive(Template)]
#[template(path = "admin.html")]
pub struct AdminTemplate {
    pub users_count: u64,
    pub invites_count: u64,
    pub banned_count: u64,
    pub active_invites: Vec<ActiveInvite>,
    pub invites_page: usize,
    pub has_next_invites: bool,
    pub has_prev_invites: bool,
}

pub struct ActiveInvite {
    pub code: String,
    pub owner: String,
}

pub struct UserNode {
    pub id: u64,
    pub login: String,
    pub is_subscriber: bool,
    pub is_admin: bool,
    pub is_banned: bool,
    pub padding_rem: f64,
}

#[derive(Template)]
#[template(path = "admin_search.html")]
pub struct AdminSearchTemplate {
    pub query: String,
    pub user: Option<UserView>,
    pub active_invites: Vec<String>,
    pub tree: Vec<UserNode>,
}

pub struct UserView {
    pub id: u64,
    pub login: String,
    pub invited_by: u64,
    pub is_banned: bool,
    pub is_user: bool,
    pub is_subscriber: bool,
    pub is_admin: bool,
    pub post_count: u32,
    pub post_ids: Vec<u64>,
}

#[derive(Template)]
#[template(path = "admin_audit.html")]
pub struct AdminAuditTemplate {
    pub logs: Vec<LogEntry>,
    pub page: usize,
    pub has_next: bool,
    pub has_prev: bool,
}

pub struct LogEntry {
    pub timestamp: u64,
    pub action: String,
}

#[inline]
#[must_use]
pub const fn is_admin(user: &User) -> bool {
    (user.roles & ROLE_ADMIN) != 0
}

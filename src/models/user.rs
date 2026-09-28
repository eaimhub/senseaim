#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

use serde::{Deserialize, Serialize};

pub const ROLE_USER: u8 = 0b001;
pub const ROLE_SUBSCRIBER: u8 = 0b010;
pub const ROLE_ADMIN: u8 = 0b100;
pub const ROLE_ROOT: u8 = ROLE_USER | ROLE_SUBSCRIBER | ROLE_ADMIN;
pub const FLAG_BANNED: u8 = 0b1000;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct User {
    pub id: u64,
    pub login: String,
    pub password_hash: String,
    pub recovery_code: String,
    pub roles: u8,
    pub invited_by: u64,
    pub plus_count: u32,
    pub post_count: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Session {
    pub user_id: u64,
    pub expires_at: u64,
}

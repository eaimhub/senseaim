#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use rand::{Rng, RngCore};
use secrecy::{ExposeSecret, Secret};
use zeroize::Zeroize;

use crate::error::AppError;

#[inline]
pub fn hash_password(password: &Secret<String>) -> Result<String, AppError> {
    let params = Params::new(65536, 3, 4, None).map_err(|_| AppError::Hash)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = SaltString::generate(&mut OsRng);

    let mut password_bytes = password.expose_secret().as_bytes().to_vec();
    let hash = argon2
        .hash_password(&password_bytes, &salt)
        .map_err(|_| AppError::Hash)?
        .to_string();

    password_bytes.zeroize();
    Ok(hash)
}

#[inline]
pub fn verify_password(password: &Secret<String>, hash: &str) -> Result<(), AppError> {
    let parsed_hash = PasswordHash::new(hash).map_err(|_| AppError::Hash)?;
    let argon2 = Argon2::default();

    let mut password_bytes = password.expose_secret().as_bytes().to_vec();
    let result = argon2
        .verify_password(&password_bytes, &parsed_hash)
        .map_err(|_| AppError::InvalidCredentials);

    password_bytes.zeroize();
    result
}

#[inline]
pub fn verify_password_dummy() {
    let params = Params::new(65536, 3, 4, None).unwrap_or_default();
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = SaltString::generate(&mut OsRng);
    let mut dummy = b"dummy_timing_defense".to_vec();
    let _ = argon2.hash_password(&dummy, &salt);
    dummy.zeroize();
}

#[inline]
pub fn generate_alphanumeric_24() -> String {
    let charset: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    (0..24)
        .map(|_| charset[rng.gen_range(0..charset.len())] as char)
        .collect()
}

#[inline]
pub fn generate_session_id() -> String {
    let mut session_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut session_bytes);
    let result = hex::encode(session_bytes);
    session_bytes.zeroize();
    result
}

#[inline]
pub fn constant_time_compare_24(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();

    if a_bytes.len() != 24 || b_bytes.len() != 24 {
        return false;
    }

    let mut diff = 0u8;
    for i in 0..24 {
        diff |= a_bytes[i] ^ b_bytes[i];
    }

    diff == 0
}

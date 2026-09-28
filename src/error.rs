#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("DATABASE COMPROMISED")]
    Database,
    #[error("DATABASE CREATION FAULT")]
    DatabaseCreation,
    #[error("DATABASE TRANSACTION FAULT")]
    DatabaseTransaction,
    #[error("DATABASE TABLE FAULT")]
    DatabaseTable,
    #[error("DATABASE COMMIT FAULT")]
    DatabaseCommit,
    #[error("DATABASE STORAGE FAULT")]
    DatabaseStorage,
    #[error("TEMPLATE RENDERING FAULT")]
    Template,
    #[error("TASK SYNCHRONIZATION FAULT")]
    TaskJoin,
    #[error("FILESYSTEM IO FAULT")]
    Io,
    #[error("SERIALIZATION FAULT")]
    Bincode,
    #[error("TIME SYNCHRONIZATION FAULT")]
    SystemTime,
    #[error("CRYPTOGRAPHIC HASH FAULT")]
    Hash,
    #[error("RATE LIMIT EXCEEDED")]
    RateLimit,
    #[error("INVALID CREDENTIALS")]
    InvalidCredentials,
    #[error("INVALID CLEARANCE CODE")]
    InvalidInvite,
    #[error("IDENTITY ALREADY ESTABLISHED")]
    UsernameTaken,
    #[error("DATA VALIDATION FAULT")]
    Validation,
    #[error("CORE INTEGRITY COMPROMISED")]
    Integrity,
    #[error("ENTITY UNREACHABLE")]
    NotFound,
}

impl From<redb::Error> for AppError {
    fn from(_: redb::Error) -> Self {
        Self::Database
    }
}

impl From<redb::DatabaseError> for AppError {
    fn from(_: redb::DatabaseError) -> Self {
        Self::DatabaseCreation
    }
}

impl From<redb::TransactionError> for AppError {
    fn from(_: redb::TransactionError) -> Self {
        Self::DatabaseTransaction
    }
}

impl From<redb::TableError> for AppError {
    fn from(_: redb::TableError) -> Self {
        Self::DatabaseTable
    }
}

impl From<redb::CommitError> for AppError {
    fn from(_: redb::CommitError) -> Self {
        Self::DatabaseCommit
    }
}

impl From<redb::StorageError> for AppError {
    fn from(_: redb::StorageError) -> Self {
        Self::DatabaseStorage
    }
}

impl From<askama::Error> for AppError {
    fn from(_: askama::Error) -> Self {
        Self::Template
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(_: tokio::task::JoinError) -> Self {
        Self::TaskJoin
    }
}

impl From<std::io::Error> for AppError {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

impl From<bincode::Error> for AppError {
    fn from(_: bincode::Error) -> Self {
        Self::Bincode
    }
}

impl From<std::time::SystemTimeError> for AppError {
    fn from(_: std::time::SystemTimeError) -> Self {
        Self::SystemTime
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            Self::RateLimit => (StatusCode::TOO_MANY_REQUESTS, "RATE LIMIT EXCEEDED"),
            Self::InvalidCredentials => (StatusCode::UNAUTHORIZED, "INVALID CREDENTIALS"),
            Self::InvalidInvite => (StatusCode::BAD_REQUEST, "INVALID CLEARANCE CODE"),
            Self::UsernameTaken => (StatusCode::BAD_REQUEST, "IDENTITY ALREADY ESTABLISHED"),
            Self::Validation => (StatusCode::BAD_REQUEST, "DATA VALIDATION FAULT"),
            Self::NotFound => (StatusCode::NOT_FOUND, "ENTITY UNREACHABLE"),
            Self::Integrity => (StatusCode::FORBIDDEN, "CORE INTEGRITY COMPROMISED"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR"),
        };
        (status, msg).into_response()
    }
}

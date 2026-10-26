//! What the member and admin handlers share: a failure as the client reads
//! it, and a JSON body read into one. The app shows any failure but a 401
//! with its status and body (`docs/server-api.md`, "Status codes"), so the
//! body is always the reason in plain words: a missing row is a 404 naming
//! it, a malformed body a 400 saying what is wrong with it, and anything
//! the store or the lockbox files fail with a 500 whose body is one line.
//! No reason here ever carries a token or a key: the store holds tokens
//! only as hashes and keys only wrapped, so its errors cannot name one.

use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;

use crate::lockboxes;
use crate::store::StoreError;

#[derive(Debug)]
pub struct Failure {
    pub status: StatusCode,
    pub reason: String,
}

impl Failure {
    pub fn new(status: StatusCode, reason: impl Into<String>) -> Self {
        Self {
            status,
            reason: reason.into(),
        }
    }

    pub fn bad_request(reason: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, reason)
    }

    pub fn forbidden(reason: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, reason)
    }

    pub fn not_found(reason: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, reason)
    }

    pub fn conflict(reason: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, reason)
    }

    /// A 500 for `error`, logged for the operator; the body is its first
    /// line.
    pub fn internal(error: impl std::fmt::Display) -> Self {
        let text = error.to_string();
        log::error!("{text}");
        let line = text.lines().next().unwrap_or_default();
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the server failed: {line}"),
        )
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.status, self.reason).into_response()
    }
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound(what) => Self::not_found(format!("no {what}")),
            other => Self::internal(other),
        }
    }
}

impl From<lockboxes::Error> for Failure {
    fn from(error: lockboxes::Error) -> Self {
        match error {
            lockboxes::Error::NotFound(id) => Self::not_found(format!("no lockbox {id}")),
            lockboxes::Error::Store(error) => error.into(),
            other => Self::internal(other),
        }
    }
}

/// A JSON body, refused as a 400 with serde's reason when it does not read
/// — axum's own `Json` answers 415 or 422, and the contract says a missing
/// field is a 400.
pub struct JsonBody<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for JsonBody<T> {
    type Rejection = Failure;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(req, state)
            .await
            .map(|Json(body)| Self(body))
            .map_err(|rejection| Failure::bad_request(rejection.body_text()))
    }
}

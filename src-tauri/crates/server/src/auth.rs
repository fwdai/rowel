//! The bearer session behind a member call. One extractor for every
//! authenticated route, so "who is calling, from which device" is decided
//! in one place and a missing, expired or revoked token is a 401 everywhere
//! — the one status the app reads as "refresh once, then sign in again"
//! (`docs/server-api.md`).

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{header, StatusCode};

use crate::state::{now_ms, AppState};
use crate::store::{Device, Member, Role, Session};

/// A member's session as the store knows it, with the member and the device
/// the session is bound to (none until the session enrolled one).
pub struct Authed {
    pub session: Session,
    pub member: Member,
    pub device: Option<Device>,
}

impl Authed {
    pub fn is_admin(&self) -> bool {
        self.member.role == Role::Admin
    }
}

impl FromRequestParts<Arc<AppState>> for Authed {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                value
                    .strip_prefix("Bearer ")
                    .or(value.strip_prefix("bearer "))
            })
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let session = state
            .store
            .session_by_access_at(token, now_ms())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::UNAUTHORIZED)?;
        // A disabled member's sessions are over too, whatever their expiry.
        let member = state
            .store
            .member(&session.member_id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .filter(|member| member.disabled_at_ms.is_none())
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let device = match &session.device_id {
            Some(id) => state
                .store
                .device(id)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            None => None,
        };
        Ok(Self {
            session,
            member,
            device,
        })
    }
}

//! Rowel Server: the broker in server mode. One binary a company runs inside
//! its network, reached as `rowel serve --server`. It holds org lockboxes
//! whose random master it wraps under its root key and under each enrolled
//! device's key (`lockboxes`, `rootkey`), knows members through the company's
//! identity provider and agents through the tokens their platforms mint
//! (`identity`), decides with the same grant engine the app runs, routes an
//! ask to the members' Rowel apps, merges what devices push, and writes down
//! every decision for export. The metadata — tenant, members, devices,
//! sessions, lockboxes, agents, trust policies, rules, the audit mirror —
//! lives in one SQLite database (`store`); the lockboxes are SQLCipher files
//! beside it, the same files the app writes.
//!
//! What is deliberately not here: a second broker. The sealed channel, the
//! grant engine, policy, audit and the lockbox-side functions are
//! `rowel-broker`'s; the approvals queue and the WebSocket serving loop are
//! `rowel-serve`'s. This crate is the server's identity, storage and routing
//! around them.

// Desktop only, as `rowel-serve` is: on a phone the crate is empty and the
// workspace still builds for one.
#![cfg(not(any(target_os = "android", target_os = "ios")))]

pub mod config;
pub mod identity;
pub mod lockboxes;
pub mod rootkey;
pub mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

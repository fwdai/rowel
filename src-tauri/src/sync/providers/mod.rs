//! One directory per cloud provider: its REST surface, its [`super::VaultStore`]
//! and its OAuth client. Nothing in here decides which name to read or write —
//! that is the core's (see [`super::store`]).

pub mod gdrive;

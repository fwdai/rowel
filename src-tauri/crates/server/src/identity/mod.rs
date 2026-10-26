//! Who is asking (spec `02`), as the server answers it: an agent by the
//! workload token its platform minted (`workload`), a member by the sign-in
//! the company's identity provider completed on the server's behalf
//! (`members`), and under both the one thing a JWT verification needs, an
//! issuer's published keys and a strict reading of the standard claims
//! (`jwt`).
//!
//! Nothing here reaches the server's own store: a trust policy, a replay
//! cache and the clock come in as parameters, so the slice verifies on its
//! own and the store-backed pieces plug in where the traits are. A token is
//! a credential, and no error, log line or `Debug` output in this module
//! carries one.

pub mod jwt;
pub mod members;
pub mod workload;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

#[cfg(test)]
mod tests;

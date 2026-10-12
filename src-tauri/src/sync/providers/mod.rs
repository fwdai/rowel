//! Cloud storage providers the sync core can sit on. Each is a directory with
//! its REST client and OAuth constants; the rules about names, identity and
//! conflicts stay in the core.

pub mod dropbox;

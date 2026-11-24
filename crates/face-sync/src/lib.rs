//! The sync: a device's library and the shared database kept the same, both ways.
//! The files stay the working copy; what changed is found by comparing each row's digest with
//! the one kept from the last sync. Its parts: the digests (`digest.rs`), the labeling side's
//! rows (`local.rs`, `rows.rs`), what a device remembers (`state.rs`), sending and taking rows
//! (`server.rs`), taking them into the library (`apply.rs`), the private folder's tables (`home.rs`), what is only ever sent (`sent.rs`), and the
//! steps themselves (`sync.rs`: joining a database, sending what changed, taking what other devices changed).

pub mod apply;
pub mod digest;
pub mod home;
pub mod keys;
pub mod local;
pub mod rows;
pub mod sent;
pub mod server;
pub mod state;
pub mod sync;

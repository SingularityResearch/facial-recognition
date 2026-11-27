//! What the programs keep in the private folder, each in a file of its own fixed form: the alert
//! inbox (`alerts`), the presence log (`presence`), the pages' settings and each person's signals (`store`),
//! the expression log's waiting rows (`exprlog`), the photos' descriptions (`descriptions`) and where the device is
//! with the rows of its location history that wait (`location`), and the signal history's rows that wait (`siglog`). The servers answer their pages from these, and the
//! sync with the shared database reads and writes the same files through them.

// (this crate needs no unsafe code: the compiler holds it to that)
#![forbid(unsafe_code)]

// where things are kept and how files are written
pub use face_files::{filejson, paths};

pub mod alerts;
pub mod blocked;
pub mod describe;
pub mod descriptions;
pub mod exprlog;
pub mod location;
pub mod passphrase;
pub mod presence;
pub mod saved;
pub mod siglog;
pub mod store;
pub mod waiting;

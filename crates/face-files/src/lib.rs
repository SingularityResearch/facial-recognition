//! What every program shares about files: where things are kept (`paths`:
//! the folder with `models/` and `data/`, the private folder, a file written whole or not at all, a lock two
//! processes share) and JSON in the files' one spelling (`filejson`), so that a file is the same bytes
//! whichever device and whichever version of the programs wrote it.

pub mod filejson;
pub mod forgotten;
pub mod paths;
pub mod protect;

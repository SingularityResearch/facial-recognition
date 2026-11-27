//! What a computer needs beside the programs, fetched by the programs themselves: the models, the model runtime,
//! MediaPipe's web runtime, FFmpeg and the HEIC libraries. Every download is pinned by its size and SHA-256
//! (`fetch.rs`), and only named files are taken out of an archive (`zip.rs`, `parts.rs`).

// (this crate needs no unsafe code: the compiler holds it to that)
#![forbid(unsafe_code)]

pub mod database;
pub mod fetch;
pub mod first_run;
pub mod models;
pub mod needs;
pub mod parts;
#[cfg(test)]
mod testing;
pub mod zip;

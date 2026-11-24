//! A row as it travels between a device and the shared database.

use serde_json::Value;

/// A face's picture in a row.
#[derive(Debug, Clone, PartialEq)]
pub enum Picture {
    /// the face has none (or its print and picture were deleted)
    None,
    /// taking: the database's picture is the one this device has, so it wasn't fetched again
    Same,
    Jpeg(Vec<u8>),
}

/// A face: where it is, what it looks like, and what its photo's record says of it alone.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceRow {
    pub photo: String,
    pub file: String,
    /// x, y, width, height and the detector's score
    pub place: Vec<f32>,
    /// none where the face was never scored
    pub quality: Option<f32>,
    /// the recognition model the print was made with
    pub model: String,
    /// none for a face whose print was deleted
    pub print: Option<Vec<f32>>,
    pub picture: Picture,
    /// the detector's score, for a possible face
    pub possible: Option<f32>,
    /// the frame it was found in, for a face from a video
    pub frame: Option<i32>,
}

/// A row of one of the tables. A label, a Web search answer and a photo travel as the JSON they are kept as
/// (a photo, sent: its record with `_facts` and `_key`; taken: `name`, `size`, `fp`, `faces`, `dup_of`,
/// `video`, `fps`, `maybe_read`, `extra`, `facts`); a face has numbers and bytes of its own.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Plain(Value),
    Face(Box<FaceRow>),
}

/// Rows by their keys, in the order they are sent or were asked for.
pub type Rows = Vec<(String, Row)>;

/// A photo record's fields that are this device's own (where its file is, what it read) or have a column of
/// their own. They never go into the row's `extra` and are never taken from it: a row in the database
/// could otherwise give a photo a path on this device, which the peer service would show to every device
/// that asked.
pub const PHOTO_FIELDS: [&str; 13] =
    ["path", "size", "mtime", "fp", "faces", "name", "maybe", "dup_of", "video", "fps", "at", "_facts", "_key"];

/// A 32-bit number from the database as the number the files keep: the one its shortest decimal spelling
/// says (0.512), not the 32-bit value widened (0.5120000243186951). That is how every device has read these
/// columns so far, and a label's score or a possible face's would otherwise change its digits, and its
/// digest, on its way through the database.
pub fn short(v: f32) -> f64 {
    v.to_string().parse().unwrap_or(v as f64)
}

/// Bytes at most in a face's picture from the database: it is a 112x112 JPEG, a few thousand.
pub const MAX_PICTURE: usize = 1_000_000;

//! The app's upkeep of LM Studio, one round at a time, for a check of it.
//!
//!     lmstudio_try <private folder>
//!
//! For every line on standard input, one round of the upkeep (LM Studio's settings and server on all
//! interfaces, its announcement) and then LM Studio's state as Settings shows it, as one line of JSON. The
//! check gives it a made-up user folder with a stand-in `lms`, and `FACE_TEST_ANNOUNCE` names a file that
//! takes what would be announced: nothing is said on the network and the real LM Studio is never touched.

use std::{io::BufRead, path::PathBuf};

fn main() {
    let home = PathBuf::from(std::env::args().nth(1).expect("the private folder"));
    for _ in std::io::stdin().lock().lines().map_while(Result::ok) {
        println!("{}", face_server::lmstudio_round(&home));
    }
}

//! A folder imported into a library, to compare with another import of the same folder:
//!
//!     import_try <root: the folder with models/ and data/> <a private folder> <the folder to read>...
//!
//! Each folder is read in turn, here and now; after each, the import's last state is printed as a line of JSON.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use face_core::faces::REC_MODELS;
use face_import::importer::Importer;
use face_library::labels::LabelDb;

fn main() {
    let mut args = std::env::args().skip(1).map(PathBuf::from);
    let (root, home) = (args.next().expect("a root folder"), args.next().expect("a private folder"));
    let db = match LabelDb::open(&root.join("data"), &home, REC_MODELS[0], false, &mut |_| {}) {
        Ok(db) => Arc::new(Mutex::new(db)),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let importer = Importer::new(db.clone(), &root, &home);
    db.lock().unwrap().remake = Some(importer.remake());
    for folder in args {
        importer.run_now(&folder);
        println!("{}", importer.status());
    }
}

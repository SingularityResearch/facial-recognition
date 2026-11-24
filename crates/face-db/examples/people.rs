//! The people as the shared database gives them against the people file the labeling tool wrote on this
//! device: are they the same? Reads only, prints numbers only (no name).
//!
//!     people <private folder with database.json> <people.npz>

use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: people <private folder> <people.npz>".into());
    }
    let model = "opsis_1";
    let t = std::time::Instant::now();
    let (names, emb, dim) =
        face_db::people_now(std::path::Path::new(&args[1]), model)?.ok_or("no shared database on this device")?;
    let took = t.elapsed().as_secs_f64();
    let z = face_core::npy::read_npz(&std::fs::read(&args[2])?)?;
    let file_names = z["name"].text()?;
    let file_emb = z["emb"].f32()?;
    // a print as the bytes of its numbers: the same face has the same print in both places
    let key = |e: &[f32]| e.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    let mut from_db: BTreeMap<Vec<u8>, &str> = BTreeMap::new();
    for (i, n) in names.iter().enumerate() {
        from_db.insert(key(&emb[i * dim..(i + 1) * dim]), n);
    }
    let mut from_file: BTreeMap<Vec<u8>, &str> = BTreeMap::new();
    for (i, n) in file_names.iter().enumerate() {
        from_file.insert(key(&file_emb[i * dim..(i + 1) * dim]), n);
    }
    let people = |m: &BTreeMap<Vec<u8>, &str>| m.values().collect::<std::collections::BTreeSet<_>>().len();
    let both = from_db.iter().filter(|(k, n)| from_file.get(*k) == Some(n)).count();
    let other_name = from_db.iter().filter(|(k, n)| from_file.get(*k).is_some_and(|f| f != *n)).count();
    println!(
        "read from the shared database in {took:.2} s: {} prints of {} people ({dim} numbers each)",
        names.len(),
        people(&from_db)
    );
    println!("the people file on this device:        {} prints of {} people", file_names.len(), people(&from_file));
    println!("the same print with the same name in both: {both}; with another name: {other_name}");
    println!(
        "only in the database: {}; only in the file: {}",
        from_db.len() - both - other_name,
        from_file.len() - both - other_name
    );
    Ok(())
}

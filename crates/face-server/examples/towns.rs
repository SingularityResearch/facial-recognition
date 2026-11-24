//! Makes the list of towns the app carries (`crates/face-server/data/towns.tsv`, read by `towns.rs`) from
//! GeoNames' files, which are published under the Creative Commons Attribution 4.0 licence (`NOTICE`):
//!
//!     cargo run -p face-server --example towns -- <folder> crates/face-server/data/towns.tsv
//!
//! `<folder>` holds `cities5000.txt` (every populated place of 5,000 people and more), `admin1CodesASCII.txt`
//! (the states, provinces and regions) and `countryInfo.txt`, as downloaded from
//! download.geonames.org/export/dump. The list is three parts, a line a row, tabs between a row's fields:
//! the countries (code, name), the regions (country.code, name) and the towns (name, latitude, longitude,
//! country.region), the towns from the most people to the fewest, so that the first town of a name is the
//! largest of that name.

use std::{collections::BTreeSet, fmt::Write, path::Path};

fn read(folder: &Path, name: &str) -> String {
    std::fs::read_to_string(folder.join(name)).unwrap_or_else(|e| panic!("{name} can't be read: {e}"))
}

/// A name as one field of a row: no tab, no line end, no space at its ends.
fn field(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(folder), Some(out)) = (args.next(), args.next()) else {
        eprintln!("towns <folder with GeoNames' files> <the list to write>");
        std::process::exit(2);
    };
    let folder = Path::new(&folder);
    // (name, latitude, longitude, country.region, people)
    let mut towns: Vec<(String, f64, f64, String, u64)> = Vec::new();
    for line in read(folder, "cities5000.txt").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 15 {
            continue;
        }
        let (Ok(lat), Ok(lon)) = (f[4].parse::<f64>(), f[5].parse::<f64>()) else { continue };
        let (name, country) = (field(f[1]), field(f[8]));
        if name.is_empty() || country.len() != 2 || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            continue;
        }
        towns.push((name, lat, lon, format!("{country}.{}", field(f[10])), f[14].parse().unwrap_or(0)));
    }
    // the most people first; towns of one size by name and place, so that the list is the same every time
    towns.sort_by(|a, b| b.4.cmp(&a.4).then_with(|| a.0.cmp(&b.0)).then_with(|| a.1.total_cmp(&b.1)));
    let used_regions: BTreeSet<&str> = towns.iter().map(|t| t.3.as_str()).collect();
    let used_countries: BTreeSet<&str> = towns.iter().map(|t| &t.3[..2]).collect();

    let mut list = String::from(
        "# Towns of 5,000 people and more: GeoNames (geonames.org), CC BY 4.0. Made by examples/towns.rs.\n",
    );
    list.push_str("[countries]\n");
    for line in read(folder, "countryInfo.txt").lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() > 4 && used_countries.contains(f[0]) && !field(f[4]).is_empty() {
            let _ = writeln!(list, "{}\t{}", f[0], field(f[4]));
        }
    }
    list.push_str("[regions]\n");
    for line in read(folder, "admin1CodesASCII.txt").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() > 1 && used_regions.contains(f[0]) && !field(f[1]).is_empty() {
            let _ = writeln!(list, "{}\t{}", f[0], field(f[1]));
        }
    }
    list.push_str("[towns]\n");
    for (name, lat, lon, region, _) in &towns {
        let _ = writeln!(list, "{name}\t{lat:.4}\t{lon:.4}\t{region}");
    }
    std::fs::write(&out, &list).unwrap_or_else(|e| panic!("{out} can't be written: {e}"));
    println!("{} towns, {} bytes", towns.len(), list.len());
}

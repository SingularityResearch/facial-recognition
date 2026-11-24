//! The towns of the world, carried in the program: what a location is called in words ("Near Springfield,
//! Illinois, United States"), and where a town is that the owner typed. Nothing is looked up anywhere: the
//! pages load nothing from the internet, and a device's location is nobody else's to see.
//!
//! The list (`data/towns.tsv`, made by `examples/towns.rs`) is GeoNames' populated places of 5,000 people
//! and more with their states and countries, published under the Creative Commons Attribution 4.0 licence
//! (`NOTICE`). It is read where it lies, a line at a time, each time it is asked: the callers ask only when
//! a device has moved or a typed place changed.

use face_home::location::distance_m;

static LIST: &str = include_str!("../data/towns.tsv");

/// metres: this near a town's middle is "in" it
const IN: f64 = 3_000.0;
/// metres: this near is "near" it; further is so many km from it
const NEAR: f64 = 30_000.0;

/// A town of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Town {
    pub name: &'static str,
    /// its state, province or region; empty where the list has none
    pub region: &'static str,
    pub country: &'static str,
    pub lat: f64,
    pub lon: f64,
}

impl Town {
    /// "Springfield, Illinois, United States": the town, its state and its country. A name is said once: a
    /// city that is its own region ("Tokyo, Japan") or its own country ("Singapore").
    pub fn words(&self) -> String {
        let mut words = self.name.to_string();
        if !self.region.is_empty() && self.region != self.name && self.region != self.country {
            words.push_str(", ");
            words.push_str(self.region);
        }
        if !self.country.is_empty() && self.country != self.name {
            words.push_str(", ");
            words.push_str(self.country);
        }
        words
    }
}

/// The list's three parts: the countries, the regions, the towns.
fn parts() -> (&'static str, &'static str, &'static str) {
    static PARTS: std::sync::OnceLock<(&str, &str, &str)> = std::sync::OnceLock::new();
    *PARTS.get_or_init(|| {
        // (each part begins on the line after its mark and ends where the next one's mark begins; a line
        // ends as the file's lines end on the computer the program was built on)
        let at = |mark: &str| LIST.find(mark).unwrap_or(LIST.len());
        let part = |mark: &str, end: usize| {
            let begins = LIST.get(at(mark)..end).and_then(|rest| rest.find('\n')).map_or(end, |n| at(mark) + n + 1);
            LIST.get(begins..end).unwrap_or("")
        };
        (part("[countries]", at("[regions]")), part("[regions]", at("[towns]")), part("[towns]", LIST.len()))
    })
}

/// The name a part of the list has for a code ("US" -> "United States", "US.IL" -> "Illinois").
fn name_of(part: &'static str, code: &str) -> &'static str {
    part.lines().find_map(|line| line.split_once('\t').filter(|(c, _)| *c == code).map(|(_, name)| name)).unwrap_or("")
}

/// One row of the towns: (name, latitude as text, longitude as text, country.region).
fn row(line: &'static str) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    let mut fields = line.split('\t');
    Some((fields.next()?, fields.next()?, fields.next()?, fields.next()?))
}

fn town(name: &'static str, lat: f64, lon: f64, key: &'static str) -> Town {
    let (countries, regions, _) = parts();
    let code = key.split('.').next().unwrap_or("");
    let country = name_of(countries, code);
    Town { name, region: name_of(regions, key), country: if country.is_empty() { code } else { country }, lat, lon }
}

/// The town nearest to a point, and how many metres its middle is from it.
pub fn nearest(lat: f64, lon: f64) -> Option<(Town, f64)> {
    if !(lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)) {
        return None;
    }
    // (degrees of latitude are the same length everywhere: a town further north or south than the nearest
    // so far is not looked at any closer)
    let mut best: Option<(&'static str, f64, f64, &'static str, f64)> = None;
    for line in parts().2.lines() {
        let Some((name, at_lat, at_lon, key)) = row(line) else { continue };
        let Ok(at_lat) = at_lat.parse::<f64>() else { continue };
        if best.is_some_and(|b| (at_lat - lat).abs() * 111_195.0 >= b.4) {
            continue;
        }
        let Ok(at_lon) = at_lon.parse::<f64>() else { continue };
        let far = distance_m((lat, lon), (at_lat, at_lon));
        if best.is_none_or(|b| far < b.4) {
            best = Some((name, at_lat, at_lon, key, far));
        }
    }
    best.map(|(name, at_lat, at_lon, key, far)| (town(name, at_lat, at_lon, key), far))
}

/// A point in words: "Springfield, Illinois, United States" in the town, "Near ..." around it, "42 km from
/// ..." in the country. Empty for what is no point of the globe.
pub fn said(lat: f64, lon: f64) -> String {
    match nearest(lat, lon) {
        Some((town, far)) if far <= IN => town.words(),
        Some((town, far)) if far <= NEAR => format!("Near {}", town.words()),
        Some((town, far)) => format!("{} km from {}", (far / 1000.0).round(), town.words()),
        None => String::new(),
    }
}

/// Words compared as a person types them: capitals and the spaces around them don't count.
fn typed(text: &str) -> String {
    text.trim().to_lowercase()
}

/// The town the owner typed: "Springfield", or with what tells it apart, "Springfield, Illinois",
/// "Springfield, IL" or "Paris, France". Of several towns that fit, the one with the most people.
pub fn named(text: &str) -> Option<Town> {
    let mut asked = text.split(',').map(typed).filter(|p| !p.is_empty());
    let name = asked.next()?;
    let more: Vec<String> = asked.take(3).collect();
    if name.chars().count() > 80 {
        return None;
    }
    let (countries, regions, towns) = parts();
    // a word after the town fits a town whose state or country it names, in full or by its code
    let fits = |key: &'static str, word: &str| {
        let (code, region_code) = key.split_once('.').unwrap_or((key, ""));
        let word = match word {
            "usa" | "u.s." | "u.s.a." => "us",
            "uk" | "u.k." => "gb",
            other => other,
        };
        word == typed(code)
            || word == typed(region_code)
            || word == typed(name_of(countries, code))
            || word == typed(name_of(regions, key))
    };
    for line in towns.lines() {
        let Some((town_name, lat, lon, key)) = row(line) else { continue };
        if town_name.len() != name.len() && town_name.is_ascii() && name.is_ascii() {
            continue; // (most rows: told apart without making their small letters)
        }
        if typed(town_name) != name || !more.iter().all(|word| fits(key, word)) {
            continue;
        }
        let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) else { continue };
        return Some(town(town_name, lat, lon, key));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_whole_and_every_row_is_a_town_of_a_country_it_names() {
        let (countries, regions, towns) = parts();
        assert!(countries.lines().count() > 200 && regions.lines().count() > 3000, "the list's first two parts");
        let mut n = 0;
        for line in towns.lines() {
            let (name, lat, lon, key) = row(line).unwrap_or_else(|| panic!("not a row: {line}"));
            let (lat, lon): (f64, f64) = (lat.parse().unwrap(), lon.parse().unwrap());
            assert!(!name.is_empty() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon), "{line}");
            let code = key.split('.').next().unwrap();
            assert!(code.len() == 2 && key.as_bytes().get(2) == Some(&b'.'), "{line}");
            assert!(!name_of(countries, code).is_empty(), "no country for {line}");
            n += 1;
        }
        assert!(n > 60_000, "{n} towns");
        for part in [countries, regions] {
            assert!(part.lines().all(|l| l.split('\t').count() == 2), "a row of two fields");
        }
    }

    #[test]
    fn a_point_is_said_as_the_town_there_with_its_state_and_country() {
        let (town, far) = nearest(39.8017, -89.6437).unwrap();
        assert_eq!((town.name, town.region, town.country), ("Springfield", "Illinois", "United States"));
        assert!(far < 100.0, "{far}");
        assert_eq!(said(39.8017, -89.6437), "Springfield, Illinois, United States");
        // out of the middle, still near; far out in the country, so many km from the nearest town
        assert_eq!(said(39.88, -89.64), "Near Springfield, Illinois, United States");
        let ocean = said(30.0, -40.0);
        assert!(
            ocean.contains(" km from ") && ocean.split(' ').next().unwrap().parse::<f64>().unwrap() > 500.0,
            "{ocean}"
        );
        // other countries, each with its own kind of region
        assert_eq!(said(48.8534, 2.3488), "Paris, Île-de-France, France");
        assert_eq!(said(-33.8678, 151.2073), "Sydney, New South Wales, Australia");
        assert_eq!(said(35.6895, 139.6917), "Tokyo, Japan");
        assert_eq!(said(1.2897, 103.8501), "Singapore");
        // what is no point of the globe has no words
        for (lat, lon) in [(91.0, 0.0), (0.0, 181.0), (f64::NAN, 0.0)] {
            assert_eq!(said(lat, lon), "");
        }
    }

    #[test]
    fn a_typed_town_is_the_largest_of_its_name_unless_more_is_said() {
        let is = |text: &str, region: &str, country: &str| {
            let t = named(text).unwrap_or_else(|| panic!("{text} was not found"));
            assert_eq!((t.region, t.country), (region, country), "{text}");
        };
        is("Springfield", "Missouri", "United States");
        is("springfield, illinois", "Illinois", "United States");
        is(" Springfield , IL ", "Illinois", "United States");
        is("Springfield, IL, USA", "Illinois", "United States");
        is("Paris", "Île-de-France", "France");
        is("Paris, Texas", "Texas", "United States");
        is("Paris, US", "Texas", "United States");
        is("London, UK", "England", "United Kingdom");
        is("London, Canada", "Ontario", "Canada");
        is("Munich", "Bavaria", "Germany");
        for none in
            ["", " , ", "No Such Town Anywhere", "Springfield, France", "Paris, Texas, France", &"x".repeat(200)]
        {
            assert_eq!(named(none), None, "{none}");
        }
    }
}

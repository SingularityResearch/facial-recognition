//! The location history in the shared database (`location_log`, migration 15): where a device was, for
//! the devices whose owner switched that on. The devices only ever send rows; a row older than `KEEP_DAYS`
//! is deleted. What is read back (each device's newest row, a device's history) is read without trusting
//! its shape: whoever has the database's login can write a row.

use serde_json::{Value, json};
use tokio_postgres::Client;

use crate::Result;

/// Days a row is kept (the owner, 2026-11-24: "only keep the On a move for last 90 days").
pub const KEEP_DAYS: i32 = 90;
/// rows in one sending
const MOST: usize = 500;

/// A row as it is sent and as it is read: degrees north and east, the metres around them (nothing for a
/// place set by hand), when (milliseconds since 1970, UTC), who said it (system | hand) and the town in
/// words.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub lat: f64,
    pub lon: f64,
    pub accuracy: Option<f64>,
    pub at: i64,
    pub source: String,
    pub place: String,
}

impl Row {
    /// Out of {lat, lon, acc, at, source, place}; nothing for what is no location.
    pub fn of(v: &Value) -> Option<Row> {
        let (lat, lon) = (v.get("lat")?.as_f64()?, v.get("lon")?.as_f64()?);
        let accuracy = match v.get("acc") {
            None | Some(Value::Null) => None,
            Some(a) => Some(a.as_f64()?),
        };
        let row = Row {
            lat,
            lon,
            accuracy,
            at: v.get("at")?.as_i64()?,
            source: v.get("source")?.as_str()?.to_string(),
            place: v.get("place").and_then(Value::as_str).unwrap_or("").to_string(),
        };
        row.good().then_some(row)
    }

    /// What the table itself would take, and a place that is a line of text.
    fn good(&self) -> bool {
        self.lat.is_finite()
            && self.lon.is_finite()
            && (-90.0..=90.0).contains(&self.lat)
            && (-180.0..=180.0).contains(&self.lon)
            && self.accuracy.is_none_or(|m| m.is_finite() && m >= 0.0)
            && self.at > 0
            && matches!(self.source.as_str(), "system" | "hand")
    }

    pub fn json(&self) -> Value {
        json!({"lat": self.lat, "lon": self.lon, "acc": self.accuracy, "at": self.at, "source": self.source,
               "place": self.place})
    }
}

/// A place as a page can show it: a line of text, whoever wrote the row.
fn a_line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(200).collect::<String>().trim().to_string()
}

fn an_id(device: &str) -> Result<()> {
    uuid::Uuid::parse_str(device).map(|_| ()).map_err(|_| "that is not a device's id".to_string())
}

/// Put a device's waiting rows into the history. A row that is there already changes nothing, and what is
/// no location is left out. The times of the rows the database has now: those wait no longer.
pub async fn send(client: &Client, device: &str, rows: &[Value]) -> Result<Vec<i64>> {
    an_id(device)?;
    let said = |e: tokio_postgres::Error| format!("The location history couldn't be written: {e}");
    let insert = client
        .prepare(
            "INSERT INTO location_log (device, at, lat, lon, accuracy, source, data)
             VALUES ($1::text::uuid, 'epoch'::timestamptz + $2::bigint * interval '1 millisecond', $3, $4, $5::real,
                     $6, jsonb_build_object('place', $7::text))
             ON CONFLICT (device, at) DO NOTHING",
        )
        .await
        .map_err(said)?;
    let mut done = Vec::new();
    for v in rows.iter().take(MOST) {
        let at = v.get("at").and_then(Value::as_i64);
        let Some(row) = Row::of(v) else {
            done.extend(at); // (not a location: it never will be, so it waits no longer)
            continue;
        };
        let accuracy = row.accuracy.map(|m| m as f32);
        client
            .execute(&insert, &[&device, &row.at, &row.lat, &row.lon, &accuracy, &row.source, &a_line(&row.place)])
            .await
            .map_err(said)?;
        done.push(row.at);
    }
    Ok(done)
}

/// Delete every device's rows that are older than they are kept. How many went.
pub async fn prune(client: &Client) -> Result<u64> {
    client
        .execute("DELETE FROM location_log WHERE at < now() - make_interval(days => $1)", &[&KEEP_DAYS])
        .await
        .map_err(|e| format!("The location history's old rows couldn't be deleted: {e}"))
}

const COLUMNS: &str = "lat, lon, accuracy::float8, (extract(epoch FROM at) * 1000)::bigint, source, \
                       coalesce(data->>'place', '')";

fn row_of(r: &tokio_postgres::Row, from: usize) -> Option<Row> {
    let row = Row {
        lat: r.try_get(from).ok()?,
        lon: r.try_get(from + 1).ok()?,
        accuracy: r.try_get(from + 2).ok()?,
        at: r.try_get(from + 3).ok()?,
        source: r.try_get(from + 4).ok()?,
        place: a_line(&r.try_get::<_, String>(from + 5).ok()?),
    };
    row.good().then_some(row)
}

/// Each device's newest row: (the device's id, where it was).
pub async fn latest(client: &Client) -> Result<Vec<(String, Row)>> {
    let rows = client
        .query(
            &format!("SELECT DISTINCT ON (device) device::text, {COLUMNS} FROM location_log ORDER BY device, at DESC"),
            &[],
        )
        .await
        .map_err(|e| format!("The devices' locations couldn't be read: {e}"))?;
    Ok(rows.iter().filter_map(|r| Some((r.try_get::<_, String>(0).ok()?, row_of(r, 1)?))).collect())
}

/// A device's history, the newest row first, `most` rows at the most.
pub async fn history(client: &Client, device: &str, most: i64) -> Result<Vec<Row>> {
    an_id(device)?;
    let rows = client
        .query(
            &format!("SELECT {COLUMNS} FROM location_log WHERE device = $1::text::uuid ORDER BY at DESC LIMIT $2"),
            &[&device, &most.clamp(1, 1000)],
        )
        .await
        .map_err(|e| format!("The location history couldn't be read: {e}"))?;
    Ok(rows.iter().filter_map(|r| row_of(r, 0)).collect())
}

/// How many rows a device has in the history.
pub async fn count(client: &Client, device: &str) -> Result<i64> {
    an_id(device)?;
    client
        .query_one("SELECT count(*) FROM location_log WHERE device = $1::text::uuid", &[&device])
        .await
        .map_err(|e| format!("The location history couldn't be read: {e}"))?
        .try_get(0)
        .map_err(|e| format!("The location history couldn't be read: {e}"))
}

/// Delete a device's whole history, at its owner's word. How many rows went.
pub async fn forget(client: &Client, device: &str) -> Result<u64> {
    an_id(device)?;
    client
        .execute("DELETE FROM location_log WHERE device = $1::text::uuid", &[&device])
        .await
        .map_err(|e| format!("The location history couldn't be deleted: {e}"))
}

/// The same from Settings, on a connection made for it (the one a program keeps for a page's short
/// questions only reads).
pub fn forget_device(home: &std::path::Path, me: &crate::Device, device: &str) -> Result<u64> {
    an_id(device)?;
    let settings = crate::Settings::load(home)?.ok_or("This device has no shared database.")?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client = crate::connect(&settings, &settings.schema, me).await?;
        forget(&client, device).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_a_location_or_nothing() {
        let good = json!({"lat": 47.6062, "lon": -122.3321, "acc": 20.0, "at": 1_795_161_900_000i64, "source": "system",
                          "place": "Seattle, Washington, United States"});
        let row = Row::of(&good).unwrap();
        assert_eq!(row.json(), good);
        let mut by_hand = good.clone();
        by_hand["acc"] = Value::Null;
        by_hand["source"] = json!("hand");
        assert_eq!(Row::of(&by_hand).unwrap().accuracy, None);
        for (key, value) in [
            ("lat", json!(91)),
            ("lat", json!("47")),
            ("lon", json!(-181)),
            ("acc", json!(-5)),
            ("acc", json!("near")),
            ("at", json!(0)),
            ("at", json!("now")),
            ("source", json!("guess")),
            ("source", Value::Null),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert_eq!(Row::of(&bad), None, "{key}");
        }
        assert_eq!(Row::of(&json!([1, 2])), None);
        // a place is one line of text, whatever a row holds
        assert_eq!(a_line(" Seattle\n\u{7}, Washington "), "Seattle, Washington");
        assert_eq!(a_line(&"x".repeat(500)).len(), 200);
    }
}

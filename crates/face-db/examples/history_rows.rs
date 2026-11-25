//! What the signal history holds, read-only: for each device, how many of its rows were written in the last
//! minutes and when its newest one was, for its own camera (name "") and for the people in its view, by the name
//! the devices know it by. For a check that the devices' history writers are writing. Prints counts and times
//! only, never a reading or a person's name.
//!
//!     history_rows <private folder with database.json> [minutes, 10 if not given]

use std::path::PathBuf;

use face_db::Settings;

async fn run(args: &[String]) -> Result<String, String> {
    let home = PathBuf::from(args.first().ok_or("usage: history_rows <private folder> [minutes]")?);
    let minutes: i32 = args.get(1).map_or(Ok(10), |m| m.parse()).map_err(|_| "minutes: a whole number")?;
    let settings = Settings::load(&home)?.ok_or("no database.json in that folder")?;
    let client = face_db::connect_read_only(&settings).await?;
    let rows = client
        .query(
            &format!(
                "SELECT coalesce({}, '(not in the device list)'), count(*) FILTER (WHERE s.name = ''),
                        count(*) FILTER (WHERE s.name <> ''), max(s.at)::text
                   FROM signal_log s LEFT JOIN devices d ON d.id::text = s.device::text
                  WHERE s.source = 'camera' AND s.at > now() - make_interval(mins => $1)
                  GROUP BY 1 ORDER BY 1",
                face_db::DEVICE_NAME_D
            ),
            &[&minutes],
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut out = format!("signal history rows of the last {minutes} minutes, by device:\n");
    for r in &rows {
        let (name, own, people, newest): (String, i64, i64, Option<String>) = (r.get(0), r.get(1), r.get(2), r.get(3));
        out += &format!(
            "  {name:<24} its own {own:>4}, people in view {people:>4}, newest {}\n",
            newest.unwrap_or_default()
        );
    }
    Ok(out)
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    print!("{}", rt.block_on(run(&args))?);
    Ok(())
}

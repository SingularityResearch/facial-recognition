//! A schema of a check's own in the shared database's server: made and migrated, or dropped. For a check that
//! runs whole programs against a database (a device joining by a code), which must never work in the schema
//! the devices share.
//!
//!     test_schema make <private folder with database.json>     prints the new schema's name
//!     test_schema drop <private folder with database.json> <schema>
//!
//! Only a name of the kind this makes (`t_` and twelve hex digits) is ever made or dropped. The private
//! folder is a check's own copy: a device's identity is made in it when it has none.

use std::path::PathBuf;

use face_db::{Device, Settings};

fn a_checks_own(name: &str) -> bool {
    name.len() == 14 && name.starts_with("t_") && name[2..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

async fn run(args: &[String]) -> Result<String, String> {
    let usage = "usage: test_schema make <private folder> | test_schema drop <private folder> <schema>";
    let home = PathBuf::from(args.get(1).ok_or(usage)?);
    let settings = Settings::load(&home)?.ok_or("no database.json in that folder")?;
    let device = Device::load(&home, "a check")?;
    let said = |e: tokio_postgres::Error| e.to_string();
    match args.first().map(String::as_str) {
        Some("make") => {
            let schema = format!("t_{}", &face_db::new_id().replace('-', "")[..12]);
            let mut client = face_db::connect(&settings, &schema, &device).await?;
            client.batch_execute(&format!("CREATE SCHEMA {schema}")).await.map_err(said)?;
            face_db::migrate(&mut client).await?;
            // the server's own certificate authority, which its administrator puts there and a joining device
            // takes from there: the one this folder's settings check the server with
            let kept: serde_json::Value =
                serde_json::from_slice(&std::fs::read(home.join("database.json")).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let ca = kept.get("ca").and_then(|v| v.as_str()).filter(|pem| !pem.is_empty());
            let ca = ca.ok_or("that folder's settings have no certificate authority")?;
            client.execute("INSERT INTO server_ca (id, pem) VALUES (1, $1)", &[&ca]).await.map_err(said)?;
            Ok(schema)
        }
        Some("drop") => {
            let schema = args.get(2).filter(|name| a_checks_own(name));
            let schema = schema.ok_or("only a schema named t_ and twelve hex digits is dropped")?;
            let client = face_db::connect(&settings, schema, &device).await?;
            client.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")).await.map_err(said)?;
            Ok("dropped".to_string())
        }
        _ => Err(usage.to_string()),
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    println!("{}", rt.block_on(run(&args))?);
    Ok(())
}

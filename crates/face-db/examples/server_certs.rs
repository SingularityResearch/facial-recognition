//! The certificates for a new database server, made as Settings' "Make them" makes them (`servercert::make`):
//! for a check that sets up a server of its own to try Connect's preparing against.
//!     server_certs <private folder> <the server's address>...
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (home, names) = args.split_first().ok_or("server_certs <private folder> <address>...")?;
    if names.is_empty() {
        return Err("name the server's address".to_string());
    }
    let folder = face_db::servercert::make(std::path::Path::new(home), names)?;
    println!("{}", folder.display());
    Ok(())
}

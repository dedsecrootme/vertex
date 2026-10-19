use crawler_rs::database;

fn main() {
    if let Err(error) = run() {
        eprintln!("Vertex Indexer: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), rusqlite::Error> {
    let dir = database::default_db_dir();
    println!("Vertex Indexer v{}", env!("CARGO_PKG_VERSION"));
    println!("Database: {}", dir.join("vertex.db").display());
    let conn = database::init_in(&dir)?;
    database::optimize(&conn)?;
    println!("Index checked: {} pages", database::count_pages(&conn)?);
    Ok(())
}

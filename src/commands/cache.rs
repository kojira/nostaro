use anyhow::Result;

use crate::cache::CacheDb;

pub async fn clear() -> Result<()> {
    let db = CacheDb::open()?;
    db.clear()?;
    println!("Cache cleared.");
    Ok(())
}

pub async fn stats() -> Result<()> {
    let db = CacheDb::open()?;
    let (events, profiles) = db.stats()?;
    println!("Cache statistics:");
    println!("  Events:   {}", events);
    println!("  Profiles: {}", profiles);
    Ok(())
}

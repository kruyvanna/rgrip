//! Query history: every batch run from a console, kept per connection in
//! `history.jsonl` next to `connections.toml` (one JSON object per line, so
//! recording a run is an append).
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Entries kept per connection.
pub const KEEP: usize = 1000;
/// The file is only rewritten once a connection is this far over `KEEP`, so
/// most runs are a plain append.
const SLACK: usize = 100;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Connection id (not its name, which can be renamed).
    pub conn: String,
    /// Database the console was pointed at (`None`: the connection's own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub sql: String,
    /// Seconds since the Unix epoch, when the run started.
    pub at: u64,
    pub millis: u64,
    /// Rows returned or affected, summed over the batch; `None` on error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    #[serde(default)]
    pub error: bool,
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

pub fn history_path() -> Option<PathBuf> {
    crate::config::config_path().map(|p| p.with_file_name("history.jsonl"))
}

fn parse(text: &str) -> Vec<Entry> {
    // A line cut short by a crash is skipped, not fatal.
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn read(path: &Path) -> Vec<Entry> {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

fn write_all(path: &Path, entries: &[Entry]) -> Result<(), String> {
    let mut text = String::new();
    for e in entries {
        text.push_str(&serde_json::to_string(e).map_err(|e| e.to_string())?);
        text.push('\n');
    }
    // Rename over the old file: a crash leaves the old or the new, never half.
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Record a run. Skips it when it repeats the connection's previous entry
/// (same SQL and database); returns whether it was written.
pub fn record_in(path: &Path, entry: &Entry) -> Result<bool, String> {
    use std::io::Write;
    if entry.sql.trim().is_empty() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let last = text.lines().rev().filter_map(|l| serde_json::from_str::<Entry>(l).ok()).find(|e| e.conn == entry.conn);
    if last.is_some_and(|l| l.sql == entry.sql && l.database == entry.database) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
    // A file that doesn't end in a newline (cut short) would swallow this line.
    if !text.is_empty() && !text.ends_with('\n') {
        f.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    let mine = text.lines().filter(|l| l.contains(&entry.conn)).count() + 1;
    if mine > KEEP + SLACK {
        trim_in(path, KEEP)?;
    }
    Ok(true)
}

/// Keep only the newest `keep` entries of each connection.
pub fn trim_in(path: &Path, keep: usize) -> Result<(), String> {
    let entries = read(path);
    let mut seen = std::collections::HashMap::<&str, usize>::new();
    let mut kept: Vec<&Entry> = entries
        .iter()
        .rev()
        .filter(|e| {
            let n = seen.entry(&e.conn).or_default();
            *n += 1;
            *n <= keep
        })
        .collect();
    kept.reverse();
    if kept.len() == entries.len() {
        return Ok(());
    }
    write_all(path, &kept.into_iter().cloned().collect::<Vec<_>>())
}

/// A connection's entries, newest first (at most `KEEP`).
pub fn load_in(path: &Path, conn: &str) -> Vec<Entry> {
    let mut v: Vec<Entry> = read(path).into_iter().filter(|e| e.conn == conn).collect();
    v.reverse();
    v.truncate(KEEP);
    v
}

/// Forget a connection's history.
pub fn clear_in(path: &Path, conn: &str) -> Result<(), String> {
    let all = read(path);
    let rest: Vec<Entry> = all.iter().filter(|e| e.conn != conn).cloned().collect();
    if rest.len() == all.len() {
        return Ok(());
    }
    write_all(path, &rest)
}

/// Entries whose SQL contains `query` (case-insensitive; blank matches all).
pub fn search<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let q = query.trim().to_lowercase();
    entries.iter().filter(|e| q.is_empty() || e.sql.to_lowercase().contains(&q)).collect()
}

pub fn record(entry: &Entry) -> Result<bool, String> {
    record_in(&history_path().ok_or("no config directory")?, entry)
}

pub fn load(conn: &str) -> Vec<Entry> {
    history_path().map(|p| load_in(&p, conn)).unwrap_or_default()
}

pub fn clear(conn: &str) -> Result<(), String> {
    clear_in(&history_path().ok_or("no config directory")?, conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(conn: &str, sql: &str, at: u64) -> Entry {
        Entry { conn: conn.into(), database: None, sql: sql.into(), at, millis: 5, rows: Some(1), error: false }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rgrip-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("history.jsonl")
    }

    #[test]
    fn records_and_loads_newest_first_per_connection() {
        let p = temp("load");
        for (c, s, t) in [("a", "select 1", 1), ("b", "select 2", 2), ("a", "select 3", 3)] {
            assert!(record_in(&p, &entry(c, s, t)).unwrap());
        }
        let a = load_in(&p, "a");
        assert_eq!(a.iter().map(|e| e.sql.as_str()).collect::<Vec<_>>(), ["select 3", "select 1"]);
        assert_eq!(load_in(&p, "b").len(), 1);
        assert!(load_in(&p, "c").is_empty());
    }

    #[test]
    fn skips_consecutive_duplicates_only() {
        let p = temp("dup");
        assert!(record_in(&p, &entry("a", "select 1", 1)).unwrap());
        assert!(!record_in(&p, &entry("a", "select 1", 2)).unwrap());
        // Another connection's run in between doesn't make it new for "a".
        assert!(record_in(&p, &entry("b", "select 9", 3)).unwrap());
        assert!(!record_in(&p, &entry("a", "select 1", 4)).unwrap());
        assert!(record_in(&p, &entry("a", "select 2", 5)).unwrap());
        assert!(record_in(&p, &entry("a", "select 1", 6)).unwrap());
        let mut other_db = entry("a", "select 1", 7);
        other_db.database = Some("x".into());
        assert!(record_in(&p, &other_db).unwrap());
        assert!(!record_in(&p, &entry("a", "  ", 8)).unwrap());
        assert_eq!(load_in(&p, "a").len(), 4);
    }

    #[test]
    fn trims_to_the_cap_per_connection() {
        let p = temp("trim");
        record_in(&p, &entry("b", "keep me", 0)).unwrap();
        for i in 0..(KEEP + SLACK + 5) {
            record_in(&p, &entry("a", &format!("select {i}"), i as u64)).unwrap();
        }
        let a = load_in(&p, "a");
        assert_eq!(a.len(), KEEP);
        assert_eq!(a[0].sql, format!("select {}", KEEP + SLACK + 4));
        assert_eq!(load_in(&p, "b").len(), 1);
        // The file itself was cut down too, not just the view of it.
        assert!(read(&p).len() <= KEEP + SLACK);
    }

    #[test]
    fn clears_one_connection_and_survives_junk() {
        let p = temp("clear");
        record_in(&p, &entry("a", "select 1", 1)).unwrap();
        record_in(&p, &entry("b", "select 2", 2)).unwrap();
        // A line cut short by a crash.
        std::fs::write(&p, format!("{}{{\"conn\":\"a\",\"sq", std::fs::read_to_string(&p).unwrap())).unwrap();
        assert!(record_in(&p, &entry("b", "select 3", 3)).unwrap());
        assert_eq!(load_in(&p, "b").len(), 2);
        clear_in(&p, "a").unwrap();
        assert!(load_in(&p, "a").is_empty());
        assert_eq!(load_in(&p, "b").len(), 2);
    }

    #[test]
    fn searches_case_insensitively() {
        let v = vec![entry("a", "SELECT * FROM Users", 2), entry("a", "delete from logs", 1)];
        assert_eq!(search(&v, "users").len(), 1);
        assert_eq!(search(&v, "  ").len(), 2);
        assert!(search(&v, "nope").is_empty());
    }
}

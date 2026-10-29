//! The jump list: tabs the user bookmarked, with enough of their state to reopen them as they were.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_ITEMS: usize = 100;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JumpState {
    Console {
        sql: String,
        cursor_start: usize,
        cursor_end: usize,
        database: Option<String>,
        schema: Option<String>,
        more_schemas: Vec<String>,
    },
    Table {
        schema: String,
        table: String,
        filter: String,
        order_col: Option<String>,
        order_desc: bool,
        page: i64,
    },
    Diagram {
        schema: String,
        filter: String,
        show_inferred: bool,
    },
    Activity {
        filter: String,
        show_idle: bool,
        auto: bool,
        interval: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JumpItem {
    pub id: String,
    pub title: String,
    /// Connection id (`ConnConfig::id`).
    pub conn: String,
    /// Unix seconds.
    pub created: u64,
    pub state: JumpState,
}

#[derive(Default, Serialize, Deserialize)]
struct JumpFile {
    #[serde(default)]
    items: Vec<JumpItem>,
}

impl JumpItem {
    pub fn new(title: String, conn: String, state: JumpState) -> Self {
        let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        // Seconds alone collide when two tabs are added in the same second.
        Self { id: format!("{}-{}", since.as_secs(), since.subsec_nanos()), title, conn, created: since.as_secs(), state }
    }
}

/// Add `item` at the top. The same connection and state again replaces the
/// old entry (and takes the new title) rather than piling up duplicates.
pub fn add(items: &mut Vec<JumpItem>, item: JumpItem) {
    items.retain(|i| !(i.conn == item.conn && i.state == item.state));
    items.insert(0, item);
    items.truncate(MAX_ITEMS);
}

pub fn remove(items: &mut Vec<JumpItem>, id: &str) {
    items.retain(|i| i.id != id);
}

fn jumps_path() -> Option<PathBuf> {
    crate::config::config_path().map(|p| p.with_file_name("jumplist.toml"))
}

/// Missing or unreadable file: empty list.
pub fn load_in(path: &Path) -> Vec<JumpItem> {
    std::fs::read_to_string(path).ok().and_then(|s| toml::from_str::<JumpFile>(&s).ok()).map(|f| f.items).unwrap_or_default()
}

pub fn save_in(path: &Path, items: &[JumpItem]) -> Result<(), String> {
    crate::config::write_toml(path, &JumpFile { items: items.to_vec() })
}

pub fn load() -> Vec<JumpItem> {
    jumps_path().map(|p| load_in(&p)).unwrap_or_default()
}

pub fn save(items: &[JumpItem]) -> Result<(), String> {
    save_in(&jumps_path().ok_or("no config directory")?, items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console(sql: &str) -> JumpState {
        JumpState::Console { sql: sql.into(), cursor_start: 1, cursor_end: 3, database: Some("db".into()), schema: None, more_schemas: vec!["a".into()] }
    }

    fn all_states() -> Vec<JumpState> {
        vec![
            console("select 1\nfrom \"t\""),
            JumpState::Table { schema: "public".into(), table: "t".into(), filter: "id > 1".into(), order_col: Some("id".into()), order_desc: true, page: 2 },
            JumpState::Diagram { schema: "public".into(), filter: "x".into(), show_inferred: true },
            JumpState::Activity { filter: "".into(), show_idle: false, auto: true, interval: 5 },
        ]
    }

    #[test]
    fn round_trips_every_kind() {
        let dir = std::env::temp_dir().join(format!("rgrip-jumps-{}", std::process::id()));
        let path = dir.join("jumplist.toml");
        let items: Vec<_> = all_states().into_iter().enumerate().map(|(i, s)| JumpItem { id: i.to_string(), title: format!("t{i}"), conn: "c".into(), created: 1, state: s }).collect();
        save_in(&path, &items).unwrap();
        assert_eq!(load_in(&path), items);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn bad_or_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("rgrip-jumps-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_in(&dir.join("none.toml")).is_empty());
        std::fs::write(dir.join("bad.toml"), "items = 3 [[").unwrap();
        assert!(load_in(&dir.join("bad.toml")).is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn add_dedupes_and_caps() {
        let mut v = vec![];
        add(&mut v, JumpItem::new("a".into(), "c".into(), console("1")));
        add(&mut v, JumpItem::new("b".into(), "c".into(), console("2")));
        add(&mut v, JumpItem::new("a2".into(), "c".into(), console("1")));
        assert_eq!(v.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["a2", "b"]);
        // Same state on another connection is a different entry.
        add(&mut v, JumpItem::new("o".into(), "other".into(), console("1")));
        assert_eq!(v.len(), 3);
        for n in 0..MAX_ITEMS + 5 {
            add(&mut v, JumpItem::new("x".into(), "c".into(), console(&format!("q{n}"))));
        }
        assert_eq!(v.len(), MAX_ITEMS);
    }

    #[test]
    fn remove_by_id() {
        let mut v = vec![];
        add(&mut v, JumpItem::new("a".into(), "c".into(), console("1")));
        let id = v[0].id.clone();
        remove(&mut v, &id);
        assert!(v.is_empty());
    }
}

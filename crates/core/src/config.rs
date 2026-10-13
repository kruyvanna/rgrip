use crate::model::ConnectionConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Default, Serialize, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    connections: Vec<ConnectionConfig>,
}

pub fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "rgrip", "rgrip")
        .map(|d| d.config_dir().join("connections.toml"))
}

pub fn load_connections() -> Vec<ConnectionConfig> {
    let Some(path) = config_path() else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str::<ConfigFile>(&s).ok())
        .map(|f| f.connections)
        .unwrap_or_default()
}

pub fn save_connections(connections: &[ConnectionConfig]) -> Result<(), String> {
    let path = config_path().ok_or("no config directory")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = ConfigFile {
        connections: connections.to_vec(),
    };
    let text = toml::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    // Passwords live in this file, keep it private.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A named SQL snippet, optionally tied to the connection it was written for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    pub sql: String,
    #[serde(default)]
    pub connection: Option<String>,
}

impl SavedQuery {
    pub fn new(name: String, sql: String, connection: Option<String>) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        Self { id: format!("q{nanos:x}"), name, sql, connection }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct QueriesFile {
    #[serde(default)]
    queries: Vec<SavedQuery>,
}

fn queries_path() -> Option<PathBuf> {
    config_path().map(|p| p.with_file_name("queries.toml"))
}

pub fn load_queries() -> Vec<SavedQuery> {
    queries_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str::<QueriesFile>(&s).ok())
        .map(|f| f.queries)
        .unwrap_or_default()
}

pub fn save_queries(queries: &[SavedQuery]) -> Result<(), String> {
    let path = queries_path().ok_or("no config directory")?;
    write_toml(&path, &QueriesFile { queries: queries.to_vec() })
}

fn write_toml(path: &std::path::Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_queries_round_trip_through_toml() {
        let q = SavedQuery::new(
            "Khmer ✓".into(),
            "-- multi\nselect 'ភ្នំពេញ', $$x$$, \"q\"\nfrom t;\n".into(),
            Some("local".into()),
        );
        let file = QueriesFile { queries: vec![q.clone()] };
        let text = toml::to_string_pretty(&file).unwrap();
        let back: QueriesFile = toml::from_str(&text).unwrap();
        assert_eq!(back.queries, vec![q]);
    }
}

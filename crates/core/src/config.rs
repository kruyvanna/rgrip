use crate::model::ConnectionConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// A saved query: one `.sql` file in the queries folder. The file name is
/// the query's name; an optional first line `-- rgrip-connection: <id>`
/// remembers the connection it was written for.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedQuery {
    /// File stem; unique within the folder.
    pub id: String,
    pub name: String,
    pub sql: String,
    pub connection: Option<String>,
    pub path: PathBuf,
}

const CONNECTION_TAG: &str = "-- rgrip-connection: ";

pub fn queries_dir() -> Option<PathBuf> {
    config_path().map(|p| p.with_file_name("queries"))
}

fn parse_query(path: &Path) -> Option<SavedQuery> {
    let text = std::fs::read_to_string(path).ok()?;
    let name = path.file_stem()?.to_string_lossy().to_string();
    let (connection, sql) = match text.strip_prefix(CONNECTION_TAG) {
        Some(rest) => {
            let (id, body) = rest.split_once('\n').unwrap_or((rest, ""));
            (Some(id.trim().to_string()).filter(|s| !s.is_empty()), body.to_string())
        }
        None => (None, text),
    };
    Some(SavedQuery { id: name.clone(), name, sql, connection, path: path.to_path_buf() })
}

fn file_text(q: &SavedQuery) -> String {
    match &q.connection {
        Some(c) => format!("{CONNECTION_TAG}{c}\n{}", q.sql),
        None => q.sql.clone(),
    }
}

/// File-name-safe version of a query name.
fn file_stem(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '\0') || c.is_control() { '-' } else { c })
        .take(120)
        .collect();
    let cleaned = cleaned.trim_start_matches('.').trim().to_string();
    if cleaned.is_empty() { "query".into() } else { cleaned }
}

/// A path for `name` in `dir` that doesn't collide (except with `keep`).
fn free_path(dir: &Path, name: &str, keep: Option<&Path>) -> PathBuf {
    let stem = file_stem(name);
    let mut path = dir.join(format!("{stem}.sql"));
    let mut n = 2;
    while path.exists() && Some(path.as_path()) != keep {
        path = dir.join(format!("{stem} {n}.sql"));
        n += 1;
    }
    path
}

/// All `.sql` files in `dir`, sorted by name.
pub fn load_queries_in(dir: &Path) -> Vec<SavedQuery> {
    let mut out: Vec<SavedQuery> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .filter_map(|p| parse_query(&p))
        .collect();
    out.sort_by_key(|q| q.name.to_lowercase());
    out
}

pub fn create_query_in(dir: &Path, name: &str, sql: &str, connection: Option<String>) -> Result<SavedQuery, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = free_path(dir, name, None);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let q = SavedQuery { id: stem.clone(), name: stem, sql: sql.to_string(), connection, path };
    std::fs::write(&q.path, file_text(&q)).map_err(|e| e.to_string())?;
    Ok(q)
}

/// Write the query's SQL and connection back to its file.
pub fn update_query(q: &SavedQuery) -> Result<(), String> {
    std::fs::write(&q.path, file_text(q)).map_err(|e| format!("Cannot save {}: {e}", q.path.display()))
}

pub fn rename_query(q: &SavedQuery, new_name: &str) -> Result<SavedQuery, String> {
    let dir = q.path.parent().ok_or("query has no folder")?;
    let path = free_path(dir, new_name, Some(&q.path));
    std::fs::rename(&q.path, &path).map_err(|e| format!("Cannot rename: {e}"))?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    Ok(SavedQuery { id: stem.clone(), name: stem, path, ..q.clone() })
}

/// Move the file to the Trash (recoverable), falling back to deleting it.
pub fn delete_query(q: &SavedQuery) -> Result<(), String> {
    if let Some(trash) = std::env::var("HOME").ok().map(|h| Path::new(&h).join(".Trash")).filter(|t| t.is_dir()) {
        let dest = free_path(&trash, &q.name, None);
        if std::fs::rename(&q.path, &dest).is_ok() {
            return Ok(());
        }
    }
    std::fs::remove_file(&q.path).map_err(|e| format!("Cannot delete: {e}"))
}

/// Show the file selected in Finder.
pub fn reveal_in_finder(path: &Path) -> Result<(), String> {
    std::process::Command::new("open").arg("-R").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// One-time move from the old single `queries.toml` to `.sql` files.
fn migrate_queries_toml(dir: &Path) {
    #[derive(Deserialize)]
    struct Old {
        name: String,
        sql: String,
        #[serde(default)]
        connection: Option<String>,
    }
    #[derive(Deserialize)]
    struct OldFile {
        #[serde(default)]
        queries: Vec<Old>,
    }
    let Some(old_path) = config_path().map(|p| p.with_file_name("queries.toml")) else { return };
    let Some(old) = std::fs::read_to_string(&old_path).ok().and_then(|s| toml::from_str::<OldFile>(&s).ok()) else {
        return;
    };
    for q in old.queries {
        let _ = create_query_in(dir, &q.name, &q.sql, q.connection);
    }
    let _ = std::fs::rename(&old_path, old_path.with_extension("toml.migrated"));
}

pub fn load_queries() -> Vec<SavedQuery> {
    let Some(dir) = queries_dir() else { return Vec::new() };
    migrate_queries_toml(&dir);
    load_queries_in(&dir)
}

pub fn create_query(name: &str, sql: &str, connection: Option<String>) -> Result<SavedQuery, String> {
    create_query_in(&queries_dir().ok_or("no config directory")?, name, sql, connection)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    /// Follow macOS appearance.
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Palette names used in dark / light appearance.
    pub dark_theme: String,
    pub light_theme: String,
    /// Check GitHub for a newer release on launch.
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            dark_theme: "rgrip Dark".into(),
            light_theme: "rgrip Light".into(),
            check_updates: true,
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    config_path().map(|p| p.with_file_name("settings.toml"))
}

pub fn load_settings() -> Settings {
    settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &Settings) -> Result<(), String> {
    write_toml(&settings_path().ok_or("no config directory")?, settings)
}

fn commands_path() -> Option<PathBuf> {
    config_path().map(|p| p.with_file_name("commands.toml"))
}

#[derive(Serialize, Deserialize)]
struct CommandsFile {
    #[serde(default, rename = "command")]
    commands: Vec<crate::commands::SlashCommand>,
}

/// User slash commands; `None` until edited in Settings (so new built-ins
/// in later versions still show up for everyone else).
pub fn load_commands() -> Option<Vec<crate::commands::SlashCommand>> {
    let text = std::fs::read_to_string(commands_path()?).ok()?;
    toml::from_str::<CommandsFile>(&text).ok().map(|f| f.commands)
}

pub fn save_commands(commands: &[crate::commands::SlashCommand]) -> Result<(), String> {
    write_toml(&commands_path().ok_or("no config directory")?, &CommandsFile { commands: commands.to_vec() })
}

/// Back to the built-ins: forget the user file.
pub fn reset_commands() -> Result<(), String> {
    match commands_path() {
        Some(p) if p.exists() => std::fs::remove_file(p).map_err(|e| e.to_string()),
        _ => Ok(()),
    }
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
    fn settings_defaults_and_partial_files() {
        assert_eq!(Settings::default().theme, ThemeChoice::System);
        // Missing keys fall back to defaults (older settings files).
        let s: Settings = toml::from_str("theme = \"Light\"\ndark_theme = \"Dracula\"").unwrap();
        assert_eq!(
            s,
            Settings { theme: ThemeChoice::Light, dark_theme: "Dracula".into(), ..Settings::default() }
        );
        let back: Settings = toml::from_str(&toml::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn connection_color_compat_and_round_trip() {
        use crate::model::ConnColor;
        // Files written before colors existed still load.
        let old = "[[connections]]\nid = \"a\"\nname = \"Local\"\nhost = \"localhost\"\nport = 5432\ndatabase = \"postgres\"\nuser = \"u\"\n";
        let f: ConfigFile = toml::from_str(old).unwrap();
        assert_eq!(f.connections[0].color, ConnColor::None);

        let mut c = f.connections[0].clone();
        c.color = ConnColor::Red;
        let text = toml::to_string_pretty(&ConfigFile { connections: vec![c] }).unwrap();
        assert!(text.contains("color = \"red\""));
        let back: ConfigFile = toml::from_str(&text).unwrap();
        assert_eq!(back.connections[0].color, ConnColor::Red);
    }

    #[test]
    fn saved_query_files() {
        let dir = std::env::temp_dir().join(format!("rgrip-queries-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let sql = "-- note\nselect 'ភ្នំពេញ', $$x$$\nfrom t;\n";
        let a = create_query_in(&dir, "Khmer: report/2024", sql, Some("local".into())).unwrap();
        assert_eq!(a.name, "Khmer- report-2024");
        assert!(a.path.ends_with("Khmer- report-2024.sql"));
        let raw = std::fs::read_to_string(&a.path).unwrap();
        assert!(raw.starts_with("-- rgrip-connection: local\n-- note\n"));

        // Same name → numbered; no connection → plain SQL file.
        let b = create_query_in(&dir, "Khmer: report/2024", "select 1", None).unwrap();
        assert_eq!(b.name, "Khmer- report-2024 2");
        assert_eq!(std::fs::read_to_string(&b.path).unwrap(), "select 1");

        let all = load_queries_in(&dir);
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].sql.as_str(), all[0].connection.as_deref()), (sql, Some("local")));

        // Edit, rename, delete.
        let mut a2 = all[0].clone();
        a2.sql = "select 2".into();
        update_query(&a2).unwrap();
        let renamed = rename_query(&a2, "Daily").unwrap();
        assert!(!a2.path.exists() && renamed.path.ends_with("Daily.sql"));
        assert_eq!(load_queries_in(&dir).iter().find(|q| q.name == "Daily").unwrap().sql, "select 2");
        std::fs::remove_file(&renamed.path).unwrap();
        std::fs::remove_file(&b.path).unwrap();
        assert!(load_queries_in(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commands_file_round_trip() {
        let file = CommandsFile { commands: crate::commands::defaults() };
        let text = toml::to_string_pretty(&file).unwrap();
        assert!(text.contains("[[command]]"));
        let back: CommandsFile = toml::from_str(&text).unwrap();
        assert_eq!(back.commands, crate::commands::defaults());
        // `help` is optional.
        let one: CommandsFile = toml::from_str("[[command]]\nname = 'x'\ntemplate = 'select 1'\n").unwrap();
        assert_eq!(one.commands[0].help, "");
    }
}

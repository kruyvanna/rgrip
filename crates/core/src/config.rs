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

fn parse_file(path: &Path) -> Vec<ConnectionConfig> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str::<ConfigFile>(&s).ok())
        .map(|f| f.connections)
        .unwrap_or_default()
}

pub fn load_connections() -> Vec<ConnectionConfig> {
    config_path().map(|p| parse_file(&p)).unwrap_or_default()
}

/// Passwords are saved in the file too, so it is kept private (0600).
pub fn save_connections(connections: &[ConnectionConfig]) -> Result<(), String> {
    let path = config_path().ok_or("no config directory")?;
    save_connections_at(&path, connections)
}

fn save_connections_at(path: &Path, connections: &[ConnectionConfig]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string_pretty(&ConfigFile { connections: connections.to_vec() }).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A saved query: one `.sql` file in the queries folder, or in one of its
/// subfolders. The file name is the query's name; an optional first line
/// `-- rgrip-connection: <id>` remembers the connection it was written for.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedQuery {
    /// `folder/stem`, or just the stem at the top level.
    pub id: String,
    pub name: String,
    /// Subfolder of the queries folder (`None`: top level).
    pub folder: Option<String>,
    pub sql: String,
    pub connection: Option<String>,
    /// Database picked in the console (`None`: the connection's default).
    pub database: Option<String>,
    /// Schemas picked in the console, main one first (empty: the default).
    pub schemas: Vec<String>,
    pub path: PathBuf,
}

const CONNECTION_TAG: &str = "-- rgrip-connection: ";
const DATABASE_TAG: &str = "-- rgrip-database: ";
const SCHEMA_TAG: &str = "-- rgrip-schema: ";

pub fn queries_dir() -> Option<PathBuf> {
    config_path().map(|p| p.with_file_name("queries"))
}

fn query_id(folder: Option<&str>, stem: &str) -> String {
    match folder {
        Some(f) => format!("{f}/{stem}"),
        None => stem.to_string(),
    }
}

fn parse_query(path: &Path, folder: Option<&str>) -> Option<SavedQuery> {
    let text = std::fs::read_to_string(path).ok()?;
    let name = path.file_stem()?.to_string_lossy().to_string();
    let (mut connection, mut database, mut schemas) = (None, None, Vec::new());
    let mut sql = text.as_str();
    // Leading tag lines, in any order.
    loop {
        let (line, body) = sql.split_once('\n').unwrap_or((sql, ""));
        let value = |rest: &str| Some(rest.trim().to_string()).filter(|s| !s.is_empty());
        if let Some(rest) = line.strip_prefix(CONNECTION_TAG) {
            connection = value(rest);
        } else if let Some(rest) = line.strip_prefix(DATABASE_TAG) {
            database = value(rest);
        } else if let Some(rest) = line.strip_prefix(SCHEMA_TAG) {
            schemas = rest.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
        } else {
            break;
        }
        sql = body;
    }
    Some(SavedQuery {
        id: query_id(folder, &name),
        name,
        folder: folder.map(str::to_string),
        sql: sql.to_string(),
        connection,
        database,
        schemas,
        path: path.to_path_buf(),
    })
}

fn file_text(q: &SavedQuery) -> String {
    let mut out = String::new();
    if let Some(c) = &q.connection {
        out.push_str(&format!("{CONNECTION_TAG}{c}\n"));
    }
    if let Some(d) = &q.database {
        out.push_str(&format!("{DATABASE_TAG}{d}\n"));
    }
    if !q.schemas.is_empty() {
        out.push_str(&format!("{SCHEMA_TAG}{}\n", q.schemas.join(", ")));
    }
    out + &q.sql
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

fn sql_files(dir: &Path, folder: Option<&str>) -> impl Iterator<Item = SavedQuery> {
    let folder = folder.map(str::to_string);
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .filter_map(move |p| parse_query(&p, folder.as_deref()))
}

/// Subfolders of `dir` (one level, hidden ones skipped), sorted by name.
pub fn query_folders_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.'))
        .collect();
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// All `.sql` files in `dir` and its subfolders, sorted by folder (top
/// level first), then name.
pub fn load_queries_in(dir: &Path) -> Vec<SavedQuery> {
    let mut out: Vec<SavedQuery> = sql_files(dir, None).collect();
    for f in query_folders_in(dir) {
        out.extend(sql_files(&dir.join(&f), Some(&f)));
    }
    out.sort_by_key(|q| (q.folder.as_ref().map(|f| f.to_lowercase()), q.name.to_lowercase()));
    out
}

pub fn create_query_in(dir: &Path, name: &str, sql: &str, connection: Option<String>, database: Option<String>, schemas: Vec<String>) -> Result<SavedQuery, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = free_path(dir, name, None);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let q = SavedQuery { id: stem.clone(), name: stem, folder: None, sql: sql.to_string(), connection, database, schemas, path };
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
    Ok(SavedQuery { id: query_id(q.folder.as_deref(), &stem), name: stem, path, ..q.clone() })
}

/// Move the query into `folder` of the queries folder `root` (`None`: the
/// top level), renumbering on a name clash.
pub fn move_query_in(root: &Path, q: &SavedQuery, folder: Option<&str>) -> Result<SavedQuery, String> {
    let folder = folder.map(file_stem);
    let dir = folder.as_ref().map_or_else(|| root.to_path_buf(), |f| root.join(f));
    if q.path.parent() == Some(dir.as_path()) {
        return Ok(q.clone());
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = free_path(&dir, &q.name, None);
    std::fs::rename(&q.path, &path).map_err(|e| format!("Cannot move: {e}"))?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    Ok(SavedQuery { id: query_id(folder.as_deref(), &stem), name: stem, folder, path, ..q.clone() })
}

/// Make an empty folder; returns its (cleaned) name.
pub fn create_query_folder_in(root: &Path, name: &str) -> Result<String, String> {
    let name = file_stem(name);
    std::fs::create_dir_all(root.join(&name)).map_err(|e| format!("Cannot create folder: {e}"))?;
    Ok(name)
}

/// Rename a folder; returns the new (cleaned) name.
pub fn rename_query_folder_in(root: &Path, old: &str, new: &str) -> Result<String, String> {
    let new = file_stem(new);
    // A case-only rename finds itself on a case-insensitive disk.
    if !new.eq_ignore_ascii_case(old) && root.join(&new).exists() {
        return Err(format!("A folder named \"{new}\" already exists"));
    }
    std::fs::rename(root.join(old), root.join(&new)).map_err(|e| format!("Cannot rename: {e}"))?;
    Ok(new)
}

/// Remove a folder, moving its queries to the top level first. Returns
/// each moved query's old id with where it is now.
pub fn delete_query_folder_in(root: &Path, folder: &str) -> Result<Vec<(String, SavedQuery)>, String> {
    let dir = root.join(folder);
    let mut moved = Vec::new();
    for q in sql_files(&dir, Some(folder)).collect::<Vec<_>>() {
        moved.push((q.id.clone(), move_query_in(root, &q, None)?));
    }
    // Finder litter (`.DS_Store`) goes with it; anything else the user put
    // there stays, and so does the folder.
    let rest: Vec<PathBuf> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    if rest.iter().all(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) && p.is_file()) {
        for p in rest {
            let _ = std::fs::remove_file(p);
        }
    }
    std::fs::remove_dir(&dir).map_err(|e| format!("Cannot remove folder: {e}"))?;
    Ok(moved)
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
        let _ = create_query_in(dir, &q.name, &q.sql, q.connection, None, Vec::new());
    }
    let _ = std::fs::rename(&old_path, old_path.with_extension("toml.migrated"));
}

pub fn load_queries() -> Vec<SavedQuery> {
    let Some(dir) = queries_dir() else { return Vec::new() };
    migrate_queries_toml(&dir);
    load_queries_in(&dir)
}

pub fn query_folders() -> Vec<String> {
    queries_dir().map(|d| query_folders_in(&d)).unwrap_or_default()
}

pub fn move_query(q: &SavedQuery, folder: Option<&str>) -> Result<SavedQuery, String> {
    move_query_in(&queries_dir().ok_or("no config directory")?, q, folder)
}

pub fn create_query_folder(name: &str) -> Result<String, String> {
    create_query_folder_in(&queries_dir().ok_or("no config directory")?, name)
}

pub fn rename_query_folder(old: &str, new: &str) -> Result<String, String> {
    rename_query_folder_in(&queries_dir().ok_or("no config directory")?, old, new)
}

pub fn delete_query_folder(folder: &str) -> Result<Vec<(String, SavedQuery)>, String> {
    delete_query_folder_in(&queries_dir().ok_or("no config directory")?, folder)
}

pub fn create_query(name: &str, sql: &str, connection: Option<String>, database: Option<String>, schemas: Vec<String>) -> Result<SavedQuery, String> {
    create_query_in(&queries_dir().ok_or("no config directory")?, name, sql, connection, database, schemas)
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
    /// Explorer folders shown closed (row keys); folders start open.
    pub collapsed_folders: Vec<String>,
    /// Explorer rows pinned to the top of their list (row keys: connections
    /// and folders).
    pub pinned: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            dark_theme: "rgrip Dark".into(),
            light_theme: "rgrip Light".into(),
            check_updates: true,
            collapsed_folders: Vec::new(),
            pinned: Vec::new(),
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

fn diagrams_path() -> Option<PathBuf> {
    config_path().map(|p| p.with_file_name("diagrams.toml"))
}

/// Saved box positions of diagrams: diagram key → table → [x, y].
type DiagramFile = std::collections::BTreeMap<String, std::collections::BTreeMap<String, [f32; 2]>>;

/// Missing file: empty. A file that doesn't parse is an error, so saving
/// never overwrites other diagrams' positions.
fn read_diagrams() -> Result<DiagramFile, String> {
    let Some(path) = diagrams_path() else { return Ok(DiagramFile::new()) };
    match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).map_err(|e| format!("diagrams.toml: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DiagramFile::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Where the boxes of diagram `key` were left (table → [x, y]).
pub fn load_diagram(key: &str) -> std::collections::BTreeMap<String, [f32; 2]> {
    read_diagrams().ok().and_then(|mut d| d.remove(key)).unwrap_or_default()
}

pub fn save_diagram(key: &str, positions: std::collections::BTreeMap<String, [f32; 2]>) -> Result<(), String> {
    let mut all = read_diagrams()?;
    all.insert(key.to_string(), positions);
    write_toml(&diagrams_path().ok_or("no config directory")?, &all)
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
        let s = Settings { collapsed_folders: vec!["fc/Prod".into()], pinned: vec!["c/local".into(), "fq/Reports".into()], ..Settings::default() };
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
    fn connection_folder_compat_and_round_trip() {
        let old = "[[connections]]\nid = \"a\"\nname = \"Local\"\nhost = \"localhost\"\nport = 5432\ndatabase = \"postgres\"\nuser = \"u\"\n";
        let f: ConfigFile = toml::from_str(old).unwrap();
        assert_eq!(f.connections[0].folder, None);
        // No folder: nothing written.
        assert!(!toml::to_string_pretty(&f).unwrap().contains("folder"));

        let mut c = f.connections[0].clone();
        c.folder = Some("Prod".into());
        let text = toml::to_string_pretty(&ConfigFile { connections: vec![c] }).unwrap();
        assert!(text.contains("folder = \"Prod\""));
        let back: ConfigFile = toml::from_str(&text).unwrap();
        assert_eq!(back.connections[0].folder.as_deref(), Some("Prod"));
    }

    #[test]
    fn saved_query_folders() {
        let dir = std::env::temp_dir().join(format!("rgrip-query-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let a = create_query_in(&dir, "daily", "select 1", None, None, Vec::new()).unwrap();
        let b = create_query_in(&dir, "weekly", "select 2", None, None, Vec::new()).unwrap();
        assert_eq!(create_query_folder_in(&dir, "Empty").unwrap(), "Empty");
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();

        // Into a folder: id carries the folder; folders list includes empty ones.
        let a = move_query_in(&dir, &a, Some("Reports/2024")).unwrap();
        assert_eq!((a.id.as_str(), a.folder.as_deref()), ("Reports-2024/daily", Some("Reports-2024")));
        assert!(a.path.ends_with("Reports-2024/daily.sql"));
        assert_eq!(query_folders_in(&dir), ["Empty", "Reports-2024"]);
        let all = load_queries_in(&dir);
        assert_eq!(all.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(), ["weekly", "Reports-2024/daily"]);
        assert_eq!(all[1].sql, "select 1");

        // Renaming keeps the folder.
        let a = rename_query(&a, "daily sales").unwrap();
        assert_eq!(a.id, "Reports-2024/daily sales");

        // A clash on the way back is numbered.
        let clash = create_query_in(&dir, "daily sales", "select 3", None, None, Vec::new()).unwrap();
        let back = move_query_in(&dir, &a, None).unwrap();
        assert_eq!((back.id.as_str(), back.folder), ("daily sales 2", None));
        std::fs::remove_file(&clash.path).unwrap();

        // Rename a folder; refuse to clobber another one.
        let b = move_query_in(&dir, &b, Some("Reports-2024")).unwrap();
        assert!(rename_query_folder_in(&dir, "Reports-2024", "Empty").is_err());
        assert_eq!(rename_query_folder_in(&dir, "Reports-2024", "weekly").unwrap(), "weekly");
        assert_eq!(rename_query_folder_in(&dir, "weekly", "Weekly").unwrap(), "Weekly");
        assert_eq!(load_queries_in(&dir).iter().find(|q| q.name == "weekly").unwrap().id, format!("Weekly/{}", b.name));

        // Deleting a folder keeps its queries.
        std::fs::write(dir.join("Weekly/.DS_Store"), "").unwrap();
        let moved = delete_query_folder_in(&dir, "Weekly").unwrap();
        assert_eq!((moved[0].0.as_str(), moved[0].1.id.as_str()), ("Weekly/weekly", "weekly"));
        assert!(!dir.join("Weekly").exists());
        let all = load_queries_in(&dir);
        assert_eq!(all.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(), ["daily sales 2", "weekly"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_query_files() {
        let dir = std::env::temp_dir().join(format!("rgrip-queries-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let sql = "-- note\nselect 'ភ្នំពេញ', $$x$$\nfrom t;\n";
        let a = create_query_in(&dir, "Khmer: report/2024", sql, Some("local".into()), None, Vec::new()).unwrap();
        assert_eq!(a.name, "Khmer- report-2024");
        assert!(a.path.ends_with("Khmer- report-2024.sql"));
        let raw = std::fs::read_to_string(&a.path).unwrap();
        assert!(raw.starts_with("-- rgrip-connection: local\n-- note\n"));

        // Same name → numbered; no connection → plain SQL file.
        let b = create_query_in(&dir, "Khmer: report/2024", "select 1", None, None, Vec::new()).unwrap();
        assert_eq!(b.name, "Khmer- report-2024 2");
        assert_eq!(std::fs::read_to_string(&b.path).unwrap(), "select 1");

        let c = create_query_in(&dir, "scoped", "select 3", Some("local".into()), Some("gis".into()), vec!["land".into(), "public".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(&c.path).unwrap(), "-- rgrip-connection: local\n-- rgrip-database: gis\n-- rgrip-schema: land, public\nselect 3");
        let back = parse_query(&c.path, None).unwrap();
        assert_eq!((back.sql.as_str(), back.database.as_deref(), back.schemas.as_slice()), ("select 3", Some("gis"), &["land".to_string(), "public".to_string()][..]));
        delete_query(&c).unwrap();

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

    #[test]
    fn secrets_round_trip_through_private_file() {
        let dir = std::env::temp_dir().join(format!("rgrip-secrets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("connections.toml");
        let mut c = ConnectionConfig { id: "a".into(), ..Default::default() };
        c.password = "pw-secret".into();
        c.ssh.passphrase = "kp-secret".into();
        save_connections_at(&path, &[c]).unwrap();
        let back = parse_file(&path).remove(0);
        assert_eq!((back.password.as_str(), back.ssh.passphrase.as_str()), ("pw-secret", "kp-secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

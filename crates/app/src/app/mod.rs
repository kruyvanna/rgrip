use crate::grid::{self, GridEvent, GridState, Sort};
use crate::icons::Svg;
use crate::worker::{Cmd, ConnId, Ev, SourceTable, TabId, Worker};
use crate::widgets::{self, Icon, TreeRow};
use crate::{highlight, theme};
use eframe::egui::{
    self, Align, Button, Color32, FontId, Frame, Id, Key, Layout, Margin,
    Modifiers, RichText, Sense, Stroke, TextEdit, TextStyle, Ui,
};
use crate::completion::{self, Popup};
use crate::plan_view;
use rgrip_core::complete::{self, Catalog};
use rgrip_core::config::{SavedQuery, Settings, ThemeChoice};
use rgrip_core::update::{self, Release};
use rgrip_core::{config, export, sql, *};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod actions;
mod console;
mod demo;
mod dialog;
mod events;
mod explorer;
mod jumps;
mod modals;
mod tab_actions;
mod table;
mod tabs;
mod ui;
mod workspace;

use self::{console::*, demo::*, explorer::*, modals::*, table::*, tabs::*, ui::*};
pub(crate) use self::explorer::menu_layout;
pub(crate) use self::ui::plural;

const PAGE_SIZE: i64 = rgrip_core::postgres::PAGE_SIZE as i64;

enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> From<Result<T, String>> for Load<T> {
    fn from(r: Result<T, String>) -> Self {
        match r {
            Ok(v) => Load::Ready(v),
            Err(e) => Load::Failed(e),
        }
    }
}

enum Status {
    Disconnected,
    Connecting,
    Connected(String),
    Failed(String),
}

struct Conn {
    cfg: ConnectionConfig,
    status: Status,
    schemas: Option<Load<Vec<String>>>,
    tables: HashMap<String, Load<Vec<TableInfo>>>,
    columns: HashMap<(String, String), Load<Vec<ColumnInfo>>>,
    /// Every table and column, for autocompletion.
    catalog: Arc<Catalog>,
    /// Databases of the server, for the console's database picker.
    databases: Option<Load<Vec<String>>>,
    /// Schemas and catalog of the other databases consoles were pointed at.
    dbs: HashMap<String, DbCache>,
}

/// What a console needs to know about a database other than the connection's own.
#[derive(Default)]
struct DbCache {
    schemas: Option<Load<Vec<String>>>,
    catalog: Arc<Catalog>,
}

impl Conn {
    fn new(cfg: ConnectionConfig) -> Self {
        Self {
            cfg,
            status: Status::Disconnected,
            schemas: None,
            tables: HashMap::new(),
            columns: HashMap::new(),
            catalog: Arc::default(),
            databases: None,
            dbs: HashMap::new(),
        }
    }

    /// `database` as a key into `dbs`: `None` for the connection's own.
    fn other_db<'a>(&self, database: Option<&'a str>) -> Option<&'a str> {
        database.filter(|d| *d != self.cfg.database)
    }

    fn schemas_in(&self, database: Option<&str>) -> &Option<Load<Vec<String>>> {
        match self.other_db(database) {
            Some(d) => self.dbs.get(d).map(|db| &db.schemas).unwrap_or(&None),
            None => &self.schemas,
        }
    }

    fn catalog_in(&self, database: Option<&str>) -> Arc<Catalog> {
        match self.other_db(database) {
            Some(d) => self.dbs.get(d).map(|db| db.catalog.clone()).unwrap_or_default(),
            None => self.catalog.clone(),
        }
    }

    fn clear_cache(&mut self) {
        self.databases = None;
        self.dbs.clear();
        self.schemas = None;
        self.tables.clear();
        self.columns.clear();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Result(usize),
    /// Map of a result set's geometries.
    Map(usize),
    /// Grid and map side by side, selection linked.
    Split(usize),
    /// The EXPLAIN plan (`Console::plan`).
    Plan,
    Output,
}

/// What a table tab shows.
#[derive(Clone, Copy, PartialEq, Default)]
enum Pane {
    #[default]
    Grid,
    Map,
    Split,
}

struct Console {
    sql: String,
    /// Last edit, for autosaving consoles that belong to a saved query.
    last_edit: Option<Instant>,
    /// Sorted char range of the editor cursor/selection.
    cursor: std::ops::Range<usize>,
    focus: bool,
    /// Find and replace (⌘F); the current match is whichever one the editor has selected.
    find: FindBar,
    running: Option<Instant>,
    /// Query waiting for the connection to come up.
    pending: Option<String>,
    output: Option<Result<QueryOutput, String>>,
    view: View,
    grid: GridState,
    popup: Option<Popup>,
    /// The running query changes schema, so refresh the catalog afterwards.
    ddl: bool,
    /// Single read-only statement being paged server-side, and the current offset.
    paged: Option<String>,
    /// The statement as run, before a sort wrapped it.
    paged_base: Option<String>,
    /// Map area filter on the result (WHERE text), empty for none.
    area: String,
    /// Result sort: which result, column, ascending.
    order: Option<(usize, String, bool)>,
    /// The rows of an in-memory sorted result in their original order.
    unsorted: Option<(usize, Vec<Vec<Option<String>>>)>,
    offset: usize,
    map: crate::map::MapView,
    /// The saved query this console was opened from / saved as.
    saved_id: Option<String>,
    /// SQL of the last run (for "Copy for AI agent").
    last_sql: String,
    /// Schema picked in the toolbar; `None` = the default.
    schema: Option<String>,
    /// Schema the last run used (paging, export and queued runs reuse it).
    run_schema: Option<String>,
    /// More schemas searched after `schema`, in the order they were ticked.
    more_schemas: Vec<String>,
    /// Database picked in the toolbar; `None` = the connection's own.
    database: Option<String>,
    /// Database the last run used (like `run_schema`).
    run_database: Option<String>,
    /// Outgoing relations of the tables the result reads from, named after
    /// the result's columns (found after the run, once the server answers).
    relations: Vec<Relation>,
    /// What `relations` is computed from (again when the catalog arrives).
    sources: Option<(Vec<Option<rgrip_core::relations::Origin>>, Vec<SourceTable>)>,
    /// Related-row counts for the rows the row menu was opened on.
    probe: Option<Probe>,
    /// Related rows (and column lookups) shown in a popup over the tab.
    related: Option<Related>,
    /// Rows of the whole paged result, once counted.
    total: Option<i64>,
    /// The count was asked for (once per run, even if it fails).
    count_asked: bool,
    /// The column origins were asked for (once per run).
    origins_asked: bool,
    /// Pending inline edits to the result: (row, col) → new value.
    changes: std::collections::BTreeMap<(usize, usize), Option<String>>,
    editor: Option<grid::CellEditor>,
    /// Edits or deletes are being written (then the page reloads).
    saving: bool,
    /// The statements being saved delete rows of the result.
    deleting: bool,
    /// Bumped on every Run: answers from an older run are dropped.
    run: u64,
    /// The last EXPLAIN plan (from Explain, or a JSON EXPLAIN you ran).
    plan: Option<crate::plan_view::PlanState>,
    /// An Explain is running, in a transaction that is rolled back (or not).
    explaining: Option<bool>,
    /// The batch the user just ran (SQL, start time), until its answer is
    /// written to the query history. Pages and sorts never set it.
    history_run: Option<(String, u64)>,
}

/// Schemas a console can pick in `database`: the connection's, without system ones.
fn user_schemas(conn: &Conn, database: Option<&str>) -> Vec<String> {
    match conn.schemas_in(database) {
        Some(Load::Ready(list)) => list
            .iter()
            .filter(|s| !matches!(s.as_str(), "pg_catalog" | "information_schema") && !s.starts_with("pg_toast"))
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

impl Console {
    /// The schema unqualified names resolve in: the picked one, else `public`
    /// (or the first schema when there is no `public`).
    fn schema_or_default<'a>(&'a self, schemas: &'a [String]) -> &'a str {
        match &self.schema {
            Some(s) => s,
            None if schemas.is_empty() || schemas.iter().any(|s| s == "public") => "public",
            None => &schemas[0],
        }
    }

    /// The schema to send with a run. Not picked and not known yet: `None`,
    /// i.e. the server's own search_path.
    fn schema_for_run(&self, schemas: &[String]) -> Option<String> {
        (self.schema.is_some() || !schemas.is_empty()).then(|| self.search_list(schemas).join("\n"))
    }

    /// The picked schemas to save with a query, main one first.
    fn picked_schemas(&self) -> Vec<String> {
        self.schema.iter().chain(&self.more_schemas).cloned().collect()
    }

    /// Every schema this console searches, main one first.
    fn search_list(&self, schemas: &[String]) -> Vec<String> {
        let mut list = vec![self.schema_or_default(schemas).to_string()];
        list.extend(self.more_schemas.iter().filter(|s| !list.contains(s)).cloned().collect::<Vec<_>>());
        list
    }

    /// The paged statement with the map area filter applied (before sorting).
    fn filtered_base(&self) -> Option<String> {
        let base = self.paged_base.clone()?;
        Some(if self.area.is_empty() { base } else { sql::filtered_query(&base, &self.area).unwrap_or(base) })
    }

    /// `base` with the current result sort applied.
    fn sorted(&self, base: String) -> Option<String> {
        match &self.order {
            Some((_, name, asc)) => sql::sorted_query(&base, name, *asc).or(Some(base)),
            None => Some(base),
        }
    }
}

impl Console {
    fn new() -> Self {
        Self {
            sql: String::new(),
            last_edit: None,
            cursor: 0..0,
            focus: true,
            find: FindBar::default(),
            running: None,
            pending: None,
            output: None,
            view: View::Output,
            grid: GridState::default(),
            popup: None,
            ddl: false,
            paged: None,
            paged_base: None,
            area: String::new(),
            order: None,
            unsorted: None,
            offset: 0,
            map: Default::default(),
            saved_id: None,
            last_sql: String::new(),
            schema: None,
            run_schema: None,
            more_schemas: Vec::new(),
            database: None,
            run_database: None,
            relations: Vec::new(),
            sources: None,
            probe: None,
            related: None,
            total: None,
            count_asked: false,
            origins_asked: false,
            changes: Default::default(),
            editor: None,
            saving: false,
            deleting: false,
            run: 0,
            history_run: None,
            plan: None,
            explaining: None,
        }
    }
}

#[derive(Default)]
struct FindBar {
    open: bool,
    query: String,
    replace: String,
    show_replace: bool,
    case_sensitive: bool,
    whole_word: bool,
    /// Which field to focus (and select the text of) on the next frame.
    focus: Option<FindField>,
    /// Scroll the current match into view on the next frame.
    reveal: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum FindField {
    Find,
    Replace,
}

impl FindBar {
    fn matches(&self, text: &str) -> Vec<std::ops::Range<usize>> {
        let opts = rgrip_core::find::Options { case_sensitive: self.case_sensitive, whole_word: self.whole_word };
        rgrip_core::find::find_all(text, &self.query, opts)
    }
}

struct TableView {
    schema: String,
    table: String,
    filter: String,
    applied_filter: String,
    order: Option<(String, bool)>,
    page: i64,
    loading: bool,
    pending: bool,
    data: Option<Result<ResultSet, String>>,
    total: Option<i64>,
    grid: GridState,
    pane: Pane,
    map: crate::map::MapView,
    /// Primary key columns; empty means the table can't be edited inline.
    key: Vec<String>,
    /// Date/time columns by name, for the edit picker.
    temporal: HashMap<String, rgrip_core::datetime::TemporalKind>,
    /// Foreign keys from and to this table, then inferred relations.
    relations: Vec<Relation>,
    /// What the inferred relations are computed from (again when the
    /// catalog arrives after the page).
    columns: Vec<ColumnInfo>,
    fk_relations: Vec<Relation>,
    /// Related rows shown in a popup over the tab.
    related: Option<Related>,
    /// Related-row counts for the rows the row menu was opened on.
    probe: Option<Probe>,
    /// Pending inline edits: (row, col) → new value.
    changes: std::collections::BTreeMap<(usize, usize), Option<String>>,
    editor: Option<grid::CellEditor>,
    saving: bool,
    /// The statements being saved delete rows (for the toast).
    deleting: bool,
    /// Pending new rows: the last `added` rows of `data`. Their cells are
    /// edited through `changes` like any other; a cell that is still NULL
    /// is left out of the INSERT so its default applies.
    added: usize,
}

/// Match counts for each relation (and each of its alternative keys) of
/// some rows, from the database.
struct Probe {
    /// The rows counted for.
    rows: Vec<usize>,
    /// (relation, variant) per probe, in the order sent.
    map: Vec<(usize, usize)>,
    /// Filled in when the counts arrive.
    counts: Vec<Option<i64>>,
}

impl Probe {
    /// For `rows`, the best count per relation (for the menu).
    fn best(&self, rows: &[usize], relations: usize) -> Vec<Option<i64>> {
        let mut best = vec![None; relations];
        if self.rows != rows {
            return Vec::new();
        }
        for (&(ri, _), c) in self.map.iter().zip(&self.counts) {
            if let (Some(c), Some(b)) = (c, best.get_mut(ri)) {
                *b = Some(b.unwrap_or(0).max(*c));
            }
        }
        best
    }

    /// For `rows`, which variants of relation `ri` matched (empty: unknown).
    fn matched(&self, rows: &[usize], ri: usize) -> Vec<usize> {
        if self.rows != rows {
            return Vec::new();
        }
        self.map.iter().zip(&self.counts).filter(|((r, _), c)| *r == ri && c.is_some_and(|c| c > 0)).map(|((_, v), _)| *v).collect()
    }
}

/// What a related-rows popup shows: rows of `schema.table` matching `filter`.
#[derive(Clone)]
struct RelatedTarget {
    schema: String,
    table: String,
    filter: String,
    /// e.g. "← posts (user_id)".
    label: String,
}

/// The related-rows popup of a table tab. Read-only; it can follow further
/// relations, with Back to return.
struct Related {
    target: RelatedTarget,
    back: Vec<RelatedTarget>,
    data: Option<Result<ResultSet, String>>,
    total: Option<i64>,
    relations: Vec<Relation>,
    columns: Vec<ColumnInfo>,
    fk_relations: Vec<Relation>,
    grid: GridState,
    probe: Option<Probe>,
}

impl Related {
    fn query(&self) -> TableQuery {
        TableQuery {
            schema: self.target.schema.clone(),
            table: self.target.table.clone(),
            filter: self.target.filter.clone(),
            order: None,
            limit: PAGE_SIZE,
            offset: 0,
        }
    }
}

impl TableView {
    fn new(schema: String, table: String, filter: String) -> Self {
        Self {
            schema,
            table,
            filter: filter.clone(),
            applied_filter: filter,
            order: None,
            page: 0,
            loading: false,
            pending: false,
            data: None,
            total: None,
            grid: GridState::default(),
            pane: Pane::Grid,
            map: Default::default(),
            key: Vec::new(),
            temporal: HashMap::new(),
            relations: Vec::new(),
            columns: Vec::new(),
            fk_relations: Vec::new(),
            related: None,
            probe: None,
            changes: Default::default(),
            editor: None,
            saving: false,
            deleting: false,
            added: 0,
        }
    }

    /// Pending edits or new rows: a reload would drop them.
    fn dirty(&self) -> bool {
        !self.changes.is_empty() || self.added > 0
    }

    /// Drop pending edits and new rows.
    fn discard(&mut self) {
        self.changes.clear();
        self.editor = None;
        if let Some(Ok(rs)) = &mut self.data {
            rs.rows.truncate(rs.rows.len().saturating_sub(self.added));
        }
        self.added = 0;
    }

    /// Append a pending row: blank, or a copy of row `from` without its key
    /// columns (they keep their default / sequence).
    fn add_row(&mut self, from: Option<usize>) {
        let Some(Ok(rs)) = &mut self.data else { return };
        let mut row = match from.and_then(|r| rs.rows.get(r)) {
            Some(src) => src.clone(),
            None => vec![None; rs.columns.len()],
        };
        if from.is_some() {
            // Without a primary key, the unique column rows are found by.
            let pk: Vec<&str> = self.columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
            let keys = if pk.is_empty() { self.key.iter().map(String::as_str).collect() } else { pk };
            for (i, c) in rs.columns.iter().enumerate() {
                if keys.contains(&c.as_str()) {
                    row[i] = None;
                }
            }
        }
        rs.rows.push(row);
        self.added += 1;
        // Show it, and select it so it's clear where the typing goes.
        self.grid.reveal = Some(rs.rows.len() - 1);
        self.grid.selection = [rs.rows.len() - 1].into();
    }

    /// Drop the pending new rows among `rows` (indexes into the page), and
    /// the edits of the rows that move up.
    fn remove_added(&mut self, rows: &[usize]) {
        let Some(Ok(rs)) = &mut self.data else { return };
        let first = rs.rows.len() - self.added;
        let gone: Vec<usize> = rows.iter().copied().filter(|&r| r >= first && r < rs.rows.len()).collect();
        let mut keep = 0;
        let mut moved = std::collections::BTreeMap::new();
        for i in first..rs.rows.len() {
            if gone.contains(&i) {
                continue;
            }
            for c in 0..rs.columns.len() {
                if let Some(v) = self.changes.remove(&(i, c)) {
                    moved.insert((first + keep, c), v);
                }
            }
            keep += 1;
        }
        self.changes.retain(|(r, _), _| *r < first);
        self.changes.extend(moved);
        let mut i = 0;
        rs.rows.retain(|_| {
            i += 1;
            !gone.contains(&(i - 1))
        });
        self.added -= gone.len();
        self.editor = None;
        self.grid.selection.clear();
    }

    fn query(&self) -> TableQuery {
        TableQuery {
            schema: self.schema.clone(),
            table: self.table.clone(),
            filter: self.applied_filter.clone(),
            order: self.order.clone(),
            limit: PAGE_SIZE,
            offset: self.page * PAGE_SIZE,
        }
    }
}

enum TabKind {
    Console(Console),
    Table(TableView),
    Layout(Box<crate::layout_tab::LayoutTab>),
    Diagram(Box<crate::diagram_tab::DiagramTab>),
    Activity(Box<crate::activity_tab::ActivityTab>),
}

impl TabKind {
    /// The related-rows popup and row-menu counts of a console or table tab.
    fn related_slots(&mut self) -> Option<(&mut Option<Related>, &mut Option<Probe>)> {
        match self {
            TabKind::Console(c) => Some((&mut c.related, &mut c.probe)),
            TabKind::Table(t) => Some((&mut t.related, &mut t.probe)),
            TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_) => None,
        }
    }
}

struct Tab {
    id: TabId,
    conn: ConnId,
    title: String,
    kind: TabKind,
}

struct Dialog {
    cfg: ConnectionConfig,
    /// Pasted connection string, and the result of applying it.
    url: String,
    url_status: Option<Result<(), String>>,
    port: String,
    ssh_port: String,
    is_new: bool,
    testing: bool,
    test: Option<Result<String, String>>,
    tab: DialogTab,
    /// Explorer folder (empty: none).
    folder: String,
}

#[derive(Clone, Copy, PartialEq)]
enum DialogTab {
    General,
    Ssl,
    Ssh,
}

impl Dialog {
    fn new(cfg: ConnectionConfig, is_new: bool) -> Self {
        Self {
            port: cfg.port.to_string(),
            url: String::new(),
            url_status: None,
            ssh_port: cfg.ssh.port.to_string(),
            folder: cfg.folder.clone().unwrap_or_default(),
            cfg,
            is_new,
            testing: false,
            test: None,
            tab: DialogTab::General,
        }
    }
}

enum Action {
    NewConnection,
    /// New connection, pre-filed in the named folder.
    NewConnectionIn(String),
    Connect(usize),
    Disconnect(usize),
    Edit(usize),
    /// Copy the connection URL, password included.
    CopyUrlWithPassword(usize),
    /// Set a connection's tab color (explorer menu).
    SetColor(usize, rgrip_core::ConnColor),
    Delete(usize),
    Refresh(usize),
    NewConsole(usize),
    /// Open the query history of a connection.
    ShowHistory(usize),
    /// Bookmark a tab (with its state) in the jump list.
    AddJump(usize),
    OpenJumpList,
    /// Open (or switch to) the ER diagram of a schema.
    OpenDiagram(usize, String),
    /// Open (or switch to) the connection's Activity tab.
    OpenActivity(usize),
    OpenTable(usize, String, String),
    /// Open a new console with a SELECT of a table.
    OpenInConsole(usize, String, String),
    /// Import a file into a schema, or into one of its tables.
    Import(usize, String, Option<String>),
    /// Edit a table's structure (columns, types, key, name).
    EditTable(usize, String, String),
    /// Open a new map layout tab.
    NewLayout(usize),
    /// Add a table's geometry column to the connection's map layout.
    AddToLayout(usize, String, String, String),
    LoadSchemas(usize),
    LoadTables(usize, String),
    LoadColumns(usize, String, String),
    ActivateTab(usize),
    Close(CloseWhich),
    OpenSaved(usize),
    SetTheme(ThemeChoice),
    /// Pick a palette by name; `true` for the dark list.
    SetPalette(bool, &'static str),
    SetAutoUpdate(bool),
    /// Open the slash command editor (Settings).
    EditCommands,
    /// The slash command editor on a new command with this name (empty: a generated one).
    NewCommand(String),
    CheckUpdate,
    InstallUpdate,
    RenameSaved(usize),
    DeleteSaved(usize),
    RevealSaved(usize),
    OpenQueriesFolder,
    /// Put a connection (by id) in a folder (`None`: the top level).
    MoveConn(String, Option<String>),
    /// Move a saved query (by id) into a folder (`None`: the top level).
    MoveSaved(String, Option<String>),
    /// Ask for a folder name; with an item id, move it there.
    NewFolder(FolderKind, Option<String>),
    RenameFolder(FolderKind, String),
    /// Connections: ungroup them. Queries: move them to the top level.
    DeleteFolder(FolderKind, String),
    RevealQueryFolder(String),
    /// Pin or unpin a connection or folder row (by key) to the top.
    TogglePin(String),
    Toggle(String),
    Select(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum FolderKind {
    Conns,
    Queries,
}

impl FolderKind {
    /// Explorer row key of a folder; also what `collapsed_folders` stores.
    fn key(self, name: &str) -> String {
        match self {
            FolderKind::Conns => format!("fc/{name}"),
            FolderKind::Queries => format!("fq/{name}"),
        }
    }
}

/// What's being dragged in the explorer.
#[derive(Clone)]
enum DragItem {
    Conn(String),
    Saved(String),
}

/// Connection folders: the saved ones (empty ones too) and those the
/// connections name, sorted.
fn conn_folders(conns: &[Conn], saved: &[String]) -> Vec<String> {
    let mut out: Vec<String> = saved.iter().cloned().chain(conns.iter().filter_map(|c| c.cfg.folder.clone())).collect();
    out.sort_by_key(|f| f.to_lowercase());
    out.dedup();
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CloseWhich {
    One(usize),
    Others(usize),
    ToTheRight(usize),
    All,
}

/// Close tabs and keep `active` pointing at a sensible tab.
fn close_tabs<T>(tabs: &mut Vec<T>, active: &mut usize, which: CloseWhich) {
    match which {
        CloseWhich::One(i) if i < tabs.len() => {
            tabs.remove(i);
            if *active > i || *active >= tabs.len() {
                *active = active.saturating_sub(1);
            }
        }
        CloseWhich::Others(i) if i < tabs.len() => {
            let keep = tabs.remove(i);
            tabs.clear();
            tabs.push(keep);
            *active = 0;
        }
        CloseWhich::ToTheRight(i) => {
            tabs.truncate(i + 1);
            *active = (*active).min(tabs.len().saturating_sub(1));
        }
        CloseWhich::All => {
            tabs.clear();
            *active = 0;
        }
        _ => {}
    }
}

enum UpdateState {
    Idle,
    /// `manual`: report "up to date" and errors (launch checks stay quiet).
    Checking { manual: bool },
    Available(Release),
    Installing(Release),
}

/// A scripted demo (debug builds) for recording clips: `RGRIP_DEMO_STEPS`
/// is a `;`-separated list played after the scene settles. See
/// `App::demo_step` for the steps.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
struct DemoPlay {
    steps: Vec<String>,
    index: usize,
    /// When the current step started.
    since: Instant,
    /// Characters of the current `type:` / `keys:` step already sent.
    typed: usize,
}

struct CommandsEditor {
    list: Vec<rgrip_core::commands::SlashCommand>,
    selected: usize,
    /// Why Save didn't work.
    error: Option<String>,
    /// Arguments for the live preview.
    preview_args: String,
}

struct NamePrompt {
    name: String,
    target: NameTarget,
}

enum NameTarget {
    /// Save the console in this tab as a new query.
    NewFrom(TabId),
    /// Rename the saved query with this id.
    Rename(String),
    /// Name a new folder; with an item id, move it there.
    NewFolder(FolderKind, Option<String>),
    RenameFolder(FolderKind, String),
}

/// Statements waiting for the user's go-ahead (see `rgrip_core::safety`).
struct Confirm {
    tab: TabId,
    kind: ConfirmKind,
    title: String,
    /// Statement, why it is risky, and whether it is dangerous anywhere
    /// (rather than just a write on production).
    items: Vec<(String, String, bool)>,
    /// The keyboard focus was put on Cancel.
    focused: bool,
}

enum ConfirmKind {
    Run { all: bool },
    Explain { analyze: bool },
    Save,
    AddKey,
    Delete(Vec<usize>),
    /// Cancel the query of (`false`) or terminate (`true`) a backend.
    Backend(i64, bool),
}

const READ_ONLY_HINT: &str = "This connection is read-only: Postgres rejects writes. Untick it in the connection's settings to edit.";

/// The statements of `sql` that need a go-ahead: dangerous ones anywhere,
/// any write on production.
fn risky_items(sql: &str, production: bool) -> Vec<(String, String, bool)> {
    rgrip_core::safety::classify(sql)
        .into_iter()
        .filter(|f| f.danger.is_some() || production)
        .map(|f| match f.danger {
            Some(why) => (sql[f.range].to_string(), why.to_string(), true),
            None => (sql[f.range].to_string(), "Writes to a production connection".to_string(), false),
        })
        .collect()
}

enum TabAction {
    /// Load the connection's schemas (for the console's schema picker).
    LoadSchemas,
    /// Map layout: run these layer queries.
    LoadLayers(Vec<crate::layout_tab::Action>),
    /// Show a message in the status bar.
    Notify(String),
    /// `/new [name]` in the console: open the slash command editor on a new command.
    NewCommand(String),
    /// Wrap the console's query to add `ST_Transform(col, 4326)` and re-run.
    Reproject(String),
    /// Filter the console's result to a map area (WHERE text), or clear it.
    ResultArea(Option<String>),
    /// Save the console's SQL (asks for a name the first time).
    Save,
    /// Write every row of the current query/table to a file.
    ExportAll(export::Format, std::path::PathBuf),
    Run { all: bool },
    /// Open the query history of the tab's connection.
    History,
    /// Show the plan of the statement Run would run (`analyze`: run it for real timings).
    Explain { analyze: bool },
    /// Write a table tab's pending inline edits.
    SaveEdits,
    /// Delete these rows of a table tab (by primary key, as loaded).
    DeleteRows(Vec<usize>),
    /// Give a table without any key an identity primary key, so it can be
    /// edited (imports and shapefile loads often come without one).
    AddKey,
    /// Open `schema.table` filtered with `filter` (following a foreign key).
    OpenFiltered { schema: String, table: String, filter: String },
    /// Open `schema.table` (from a diagram box).
    OpenTable { schema: String, table: String },
    /// Diagram: load the schema again.
    ReloadDiagram,
    /// Activity: fetch the sessions again.
    RefreshActivity,
    /// Activity: ask before cancelling (`false`) or terminating (`true`) a backend.
    ConfirmBackend(i64, bool),
    /// Activity: signal the backend (after the confirmation).
    SignalBackend(i64, bool),
    /// Show related rows in the tab's popup (stacking on an open one).
    ShowRelated(RelatedTarget),
    /// Count related rows for the row menu (`modal`: the popup's rows).
    Probe { modal: bool, rows: Vec<usize>, probes: Vec<(String, String, String)>, map: Vec<(usize, usize)> },
    /// Popup: back to the previous related rows.
    RelatedBack,
    /// Sort a console result (result index, column index) by that column.
    SortResult(usize, usize),
    /// Next (`true`) or previous page of a paged console query.
    Page(bool),
    Cancel,
    Reload,
}

pub struct App {
    worker: Worker,
    conns: Vec<Conn>,
    /// Connection folders, kept even while empty (`connections.toml`).
    conn_folders: Vec<String>,
    tabs: Vec<Tab>,
    active: usize,
    next_tab: TabId,
    dialog: Option<Dialog>,
    filter: String,
    notice: Option<String>,
    saved: Vec<SavedQuery>,
    /// Subfolders of the queries folder (including empty ones).
    query_folders: Vec<String>,
    name_prompt: Option<NamePrompt>,
    confirm: Option<Confirm>,
    /// The next Run / Save was confirmed: don't ask again.
    skip_confirm: bool,
    settings: Settings,
    update: UpdateState,
    /// Close the window at the end of this frame (relaunching after an update).
    ctx_close: bool,
    was_focused: bool,
    egui_ctx: egui::Context,
    /// Transient status-bar message (text, color, when).
    toast: Option<(String, Color32, Instant)>,
    /// Keys of expanded explorer nodes.
    expanded: HashSet<String>,
    selected: Option<String>,
    /// A tree row was clicked last: arrow keys move the selection and
    /// typing jumps to a matching row.
    explorer_active: bool,
    /// Type-ahead buffer and when its last key came in.
    type_ahead: (String, Instant),
    /// The tree rows drawn last frame, in order.
    explorer_rows: Vec<NavRow>,
    /// A row to scroll into view on the next frame (`None`: just enough).
    scroll_to_row: Option<(String, Option<Align>)>,
    /// Slash command editor (Settings), while open.
    commands_editor: Option<CommandsEditor>,
    /// The Import data dialog, while open.
    import: Option<crate::import::ImportDialog>,
    table_editor: Option<crate::table_edit::TableEditor>,
    /// The Query history dialog, while open.
    history: Option<crate::history_view::HistoryView>,
    /// Bookmarked tabs (`jumplist.toml`) and their dialog, while open.
    jumps: Vec<rgrip_core::jumps::JumpItem>,
    jump_view: Option<crate::jump_view::JumpView>,
    /// Debug demo recordings: the scripted steps being played.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    demo_play: Option<DemoPlay>,
    /// Debug demo recordings: pointer events for the next frame's input.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    demo_input: Vec<egui::Event>,
    /// Debug screenshot scenes: when the scene first looked settled.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    demo_settled: Option<Instant>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = config::load_settings();
        // Debug builds: RGRIP_DEMO_THEME=light|dark and RGRIP_DEMO_PALETTE=<name>
        // for screenshots (not saved).
        let mut theme_choice = match std::env::var("RGRIP_DEMO_THEME").as_deref() {
            Ok("light") if cfg!(debug_assertions) => ThemeChoice::Light,
            Ok("dark") if cfg!(debug_assertions) => ThemeChoice::Dark,
            _ => settings.theme,
        };
        let (mut dark_name, mut light_name) = (settings.dark_theme.clone(), settings.light_theme.clone());
        if let Some(name) = std::env::var("RGRIP_DEMO_PALETTE").ok().filter(|_| cfg!(debug_assertions)) {
            if theme::DARK_THEMES.iter().any(|t| t.name == name) {
                (dark_name, theme_choice) = (name, ThemeChoice::Dark);
            } else if theme::LIGHT_THEMES.iter().any(|t| t.name == name) {
                (light_name, theme_choice) = (name, ThemeChoice::Light);
            }
        }
        theme::apply(&cc.egui_ctx, theme_choice, &dark_name, &light_name);
        if let Some(commands) = config::load_commands() {
            rgrip_core::commands::set_active(commands);
        }
        let worker = Worker::spawn(cc.egui_ctx.clone());
        // Launch checks only in release builds (dev runs restart a lot), or
        // when a test API is configured.
        let check_now =
            settings.check_updates && (!cfg!(debug_assertions) || std::env::var("RGRIP_UPDATE_API").is_ok());
        if check_now {
            worker.send(Cmd::CheckUpdate);
        }
        let (conns, conn_folders) = config::load_connections();
        Self {
            update: if check_now { UpdateState::Checking { manual: false } } else { UpdateState::Idle },
            ctx_close: false,
            was_focused: true,
            egui_ctx: cc.egui_ctx.clone(),
            settings,
            worker,
            conns: conns.into_iter().map(Conn::new).collect(),
            conn_folders,
            tabs: Vec::new(),
            active: 0,
            next_tab: 1,
            dialog: None,
            filter: String::new(),
            notice: None,
            saved: config::load_queries(),
            query_folders: config::query_folders(),
            name_prompt: None,
            confirm: None,
            skip_confirm: false,
            toast: None,
            expanded: HashSet::new(),
            selected: None,
            explorer_active: false,
            type_ahead: (String::new(), Instant::now()),
            explorer_rows: Vec::new(),
            scroll_to_row: None,
            demo_settled: None,
            demo_play: None,
            demo_input: Vec::new(),
            commands_editor: None,
            import: None,
            table_editor: None,
            history: None,
            jumps: rgrip_core::jumps::load(),
            jump_view: None,
        }
    }

    fn conn_index(&self, id: &str) -> Option<usize> {
        self.conns.iter().position(|c| c.cfg.id == id)
    }

    /// The catalog of a tab's connection; empty until it has loaded.
    fn tab_catalog(&self, id: TabId) -> Arc<Catalog> {
        let conn = self.tabs.iter().find(|t| t.id == id).map(|t| t.conn.clone());
        conn.and_then(|c| self.conn_index(&c)).map(|ci| self.conns[ci].catalog.clone()).unwrap_or_default()
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn save_config(&mut self) {
        let cfgs: Vec<_> = self.conns.iter().map(|c| c.cfg.clone()).collect();
        self.conn_folders = conn_folders(&self.conns, &self.conn_folders);
        if let Err(e) = config::save_connections(&cfgs, &self.conn_folders) {
            self.notice = Some(format!("Could not save connections: {e}"));
        }
    }

    // ---- actions ------------------------------------------------------

    /// Returns true when already connected; otherwise starts connecting.
    fn ensure_connected(&mut self, ci: usize) -> bool {
        match self.conns[ci].status {
            Status::Connected(_) => true,
            Status::Connecting => false,
            Status::Disconnected | Status::Failed(_) => {
                self.apply(Action::Connect(ci));
                false
            }
        }
    }

    fn save_settings(&mut self) {
        if let Err(e) = config::save_settings(&self.settings) {
            self.notice = Some(format!("Could not save settings: {e}"));
        }
    }

    /// Re-read the queries folder (files may be edited outside the app).
    fn reload_queries(&mut self) {
        self.saved = config::load_queries();
        self.query_folders = config::query_folders();
    }

    fn report(&mut self, result: Result<(), String>) {
        if let Err(e) = result {
            self.toast = Some((e, theme::ERROR(), Instant::now()));
        }
    }
}

impl eframe::App for App {
    #[cfg(debug_assertions)]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        raw_input.events.append(&mut self.demo_input);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::sync(ctx);
        // Pick up query files edited in another app when we come back.
        let focused = ctx.input(|i| i.focused);
        if focused && !self.was_focused {
            self.reload_queries();
        }
        self.was_focused = focused;
        #[cfg(debug_assertions)]
        self.demo_hook();
        self.handle_events();
        self.autosave();

        let mut actions = Vec::new();
        let mut tab_action = None;

        // ⌘N: new console on the current tab's connection (or the first one).
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::N)) && !self.conns.is_empty() {
            let ci = self.tabs.get(self.active).and_then(|t| self.conn_index(&t.conn)).unwrap_or(0);
            actions.push(Action::NewConsole(ci));
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::W)) && !self.tabs.is_empty() {
            actions.push(Action::Close(CloseWhich::One(self.active)));
        }

        // ⌘⇧K bookmarks the active tab, ⌘K shows the bookmarks (⌘J is the grid's "go to column").
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::K)) && !self.tabs.is_empty() {
            actions.push(Action::AddJump(self.active));
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K)) {
            actions.push(Action::OpenJumpList);
        }

        // Before the panels, or the editor's own ⌘↩ would take it.
        if self.confirm.is_some() && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
            self.accept_confirm();
        }

        self.title_bar(ctx, &mut actions);

        egui::TopBottomPanel::bottom("status")
            .exact_height(26.0)
            .frame(Frame::new().fill(theme::BG()).inner_margin(Margin::symmetric(12, 4)))
            .show(ctx, |ui| self.status_bar(ui, &mut actions));

        egui::SidePanel::left("explorer")
            .resizable(true)
            .default_width(270.0)
            .width_range(180.0..=560.0)
            .frame(Frame::new().fill(theme::BG()).inner_margin(Margin { left: 8, right: 8, top: 4, bottom: 8 }))
            .show(ctx, |ui| self.explorer(ui, &mut actions));

        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::SURFACE()))
            .show(ctx, |ui| {
                if self.tabs.is_empty() {
                    empty_state(ui, &mut actions, self.conns.is_empty());
                } else {
                    self.tab_bar(ui, &mut actions);
                    tab_action = self.tab_content(ui);
                }
            });

        self.dialog_ui(ctx);
        self.name_prompt_ui(ctx);
        self.confirm_ui(ctx);
        self.commands_editor_ui(ctx);
        self.import_ui(ctx);
        self.table_editor_ui(ctx);
        self.history_ui(ctx);
        self.jump_ui(ctx);

        if let Some(a) = tab_action {
            self.tab_action(self.active, a);
        }
        for a in actions {
            self.apply(a);
        }
        if self.ctx_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}


#[cfg(test)]
mod tests {
    //! Drives the real console UI headlessly with keyboard events.
    use super::*;

    #[test]
    fn risky_items_by_connection() {
        let sql = "select 1; insert into t values (1); delete from t";
        let dev = risky_items(sql, false);
        assert_eq!(dev.len(), 1, "only the unfiltered delete is dangerous off production");
        assert!(dev[0].2 && dev[0].0 == "delete from t");
        let prod = risky_items(sql, true);
        assert_eq!(prod.len(), 2);
        assert!(!prod[0].2 && prod[0].1.contains("production"));
        assert!(risky_items("select * from t", true).is_empty());
    }
}

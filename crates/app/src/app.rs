use crate::grid::{self, GridEvent, GridState, Sort};
use crate::worker::{Cmd, ConnId, Ev, TabId, Worker};
use crate::widgets::{self, Icon, TreeRow};
use crate::{highlight, theme};
use eframe::egui::{
    self, Align, Button, Color32, FontId, Frame, Id, Key, Layout, Margin,
    Modifiers, RichText, Sense, Stroke, TextEdit, TextStyle, Ui,
};
use crate::completion::{self, Popup};
use rgrip_core::complete::{self, Catalog};
use rgrip_core::config::{SavedQuery, Settings, ThemeChoice};
use rgrip_core::update::{self, Release};
use rgrip_core::{config, export, sql, *};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
        }
    }

    fn clear_cache(&mut self) {
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
    Output,
}

struct Console {
    sql: String,
    /// Sorted char range of the editor cursor/selection.
    cursor: std::ops::Range<usize>,
    focus: bool,
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
    offset: usize,
    map: crate::map::MapView,
    /// The saved query this console was opened from / saved as.
    saved_id: Option<String>,
    /// SQL of the last run (for "Copy for AI agent").
    last_sql: String,
}

impl Console {
    fn new() -> Self {
        Self {
            sql: String::new(),
            cursor: 0..0,
            focus: true,
            running: None,
            pending: None,
            output: None,
            view: View::Output,
            grid: GridState::default(),
            popup: None,
            ddl: false,
            paged: None,
            offset: 0,
            map: Default::default(),
            saved_id: None,
            last_sql: String::new(),
        }
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
    show_map: bool,
    map: crate::map::MapView,
}

impl TableView {
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
    Connect(usize),
    Disconnect(usize),
    Edit(usize),
    Delete(usize),
    Refresh(usize),
    NewConsole(usize),
    OpenTable(usize, String, String),
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
    CheckUpdate,
    InstallUpdate,
    RenameSaved(usize),
    DeleteSaved(usize),
    RevealSaved(usize),
    OpenQueriesFolder,
    Toggle(String),
    Select(String),
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

struct NamePrompt {
    name: String,
    target: NameTarget,
}

enum NameTarget {
    /// Save the console in this tab as a new query.
    NewFrom(TabId),
    /// Rename the saved query with this id.
    Rename(String),
}

enum TabAction {
    /// Show a message in the status bar.
    Notify(String),
    /// Wrap the console's query to add `ST_Transform(col, 4326)` and re-run.
    Reproject(String),
    /// Save the console's SQL (asks for a name the first time).
    Save,
    /// Write every row of the current query/table to a file.
    ExportAll(export::Format, std::path::PathBuf),
    Run { all: bool },
    /// Next (`true`) or previous page of a paged console query.
    Page(bool),
    Cancel,
    Reload,
}

pub struct App {
    worker: Worker,
    conns: Vec<Conn>,
    tabs: Vec<Tab>,
    active: usize,
    next_tab: TabId,
    dialog: Option<Dialog>,
    filter: String,
    notice: Option<String>,
    saved: Vec<SavedQuery>,
    name_prompt: Option<NamePrompt>,
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
        let worker = Worker::spawn(cc.egui_ctx.clone());
        // Launch checks only in release builds (dev runs restart a lot), or
        // when a test API is configured.
        let check_now =
            settings.check_updates && (!cfg!(debug_assertions) || std::env::var("RGRIP_UPDATE_API").is_ok());
        if check_now {
            worker.send(Cmd::CheckUpdate);
        }
        Self {
            update: if check_now { UpdateState::Checking { manual: false } } else { UpdateState::Idle },
            ctx_close: false,
            was_focused: true,
            egui_ctx: cc.egui_ctx.clone(),
            settings,
            worker,
            conns: config::load_connections().into_iter().map(Conn::new).collect(),
            tabs: Vec::new(),
            active: 0,
            next_tab: 1,
            dialog: None,
            filter: String::new(),
            notice: None,
            saved: config::load_queries(),
            name_prompt: None,
            toast: None,
            expanded: HashSet::new(),
            selected: None,
        }
    }

    fn conn_index(&self, id: &str) -> Option<usize> {
        self.conns.iter().position(|c| c.cfg.id == id)
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn save_config(&mut self) {
        let cfgs: Vec<_> = self.conns.iter().map(|c| c.cfg.clone()).collect();
        if let Err(e) = config::save_connections(&cfgs) {
            self.notice = Some(format!("Could not save connections: {e}"));
        }
    }

    // ---- worker events -------------------------------------------------

    fn handle_events(&mut self) {
        let events: Vec<Ev> = self.worker.poll().collect();
        for ev in events {
            match ev {
                Ev::Connected { conn, version } => {
                    let Some(ci) = self.conn_index(&conn) else { continue };
                    self.conns[ci].status = Status::Connected(version);
                    self.worker.send(Cmd::Catalog(conn.clone()));
                    self.flush_pending(&conn);
                    if !self.tabs.iter().any(|t| t.conn == conn) {
                        self.open_console(ci);
                    }
                }
                Ev::ConnectFailed { conn, error } => {
                    let Some(ci) = self.conn_index(&conn) else { continue };
                    self.conns[ci].status = Status::Failed(error.clone());
                    for tab in self.tabs.iter_mut().filter(|t| t.conn == conn) {
                        match &mut tab.kind {
                            TabKind::Console(c) if c.pending.is_some() => {
                                c.pending = None;
                                c.running = None;
                                c.output = Some(Err(error.clone()));
                            }
                            TabKind::Table(t) if t.pending => {
                                t.pending = false;
                                t.loading = false;
                                t.data = Some(Err(error.clone()));
                            }
                            _ => {}
                        }
                    }
                }
                Ev::TestResult(result) => {
                    if let Some(d) = &mut self.dialog {
                        d.testing = false;
                        d.test = Some(result);
                    }
                }
                Ev::Schemas { conn, result } => {
                    if let Ok(list) = &result {
                        if list.iter().any(|s| s == "public") {
                            self.expanded.insert(format!("s/{conn}/public"));
                        }
                    }
                    if let Some(ci) = self.conn_index(&conn) {
                        self.conns[ci].schemas = Some(result.into());
                    }
                }
                Ev::Tables { conn, schema, result } => {
                    if let Some(ci) = self.conn_index(&conn) {
                        self.conns[ci].tables.insert(schema, result.into());
                    }
                }
                Ev::Columns { conn, schema, table, result } => {
                    if let Some(ci) = self.conn_index(&conn) {
                        self.conns[ci].columns.insert((schema, table), result.into());
                    }
                }
                Ev::Catalog { conn, result } => {
                    if let (Some(ci), Ok(catalog)) = (self.conn_index(&conn), result) {
                        self.conns[ci].catalog = Arc::new(catalog);
                    }
                }
                Ev::QueryDone { tab, result } => {
                    let mut refresh = None;
                    if let Some(Tab { kind: TabKind::Console(c), conn, .. }) = self.tab_mut(tab) {
                        if std::mem::take(&mut c.ddl) && result.is_ok() {
                            refresh = Some(conn.clone());
                        }
                        c.running = None;
                        c.grid = GridState::default();
                        c.view = match &result {
                            Ok(out) => match out.statements.iter().filter(|s| s.result.is_some()).count() {
                                0 => View::Output,
                                n => View::Result(n - 1),
                            },
                            Err(_) => View::Output,
                        };
                        c.output = Some(result);
                    }
                    if let Some(conn) = refresh {
                        self.worker.send(Cmd::Catalog(conn));
                    }
                }
                Ev::TablePage { tab, result } => {
                    if let Some(Tab { kind: TabKind::Table(t), .. }) = self.tab_mut(tab) {
                        t.loading = false;
                        t.grid = GridState::default();
                        t.data = Some(result);
                    }
                }
                Ev::Exported(result) => {
                    self.toast = Some(match result {
                        Ok((path, rows)) => (
                            format!("Exported {} to {}", plural(rows, "row"), path.display()),
                            theme::OK(),
                            Instant::now(),
                        ),
                        Err(e) => (format!("Export failed: {e}"), theme::ERROR(), Instant::now()),
                    });
                }
                Ev::UpdateChecked(result) => {
                    let manual = matches!(self.update, UpdateState::Checking { manual: true });
                    self.update = match result {
                        Ok(Some(release)) => UpdateState::Available(release),
                        Ok(None) => {
                            if manual {
                                self.toast = Some((format!("rgrip {} is up to date", update::CURRENT), theme::OK(), Instant::now()));
                            }
                            UpdateState::Idle
                        }
                        Err(e) => {
                            if manual {
                                self.toast = Some((e, theme::ERROR(), Instant::now()));
                            }
                            UpdateState::Idle
                        }
                    };
                }
                Ev::UpdateInstalled(result) => match result {
                    Ok(app) => match update::relaunch(&app) {
                        Ok(()) => self.ctx_close = true,
                        Err(e) => self.toast = Some((e, theme::ERROR(), Instant::now())),
                    },
                    Err(e) => {
                        if let UpdateState::Installing(r) = std::mem::replace(&mut self.update, UpdateState::Idle) {
                            self.update = UpdateState::Available(r);
                        }
                        self.toast = Some((e, theme::ERROR(), Instant::now()));
                    }
                },
                Ev::TableCount { tab, result } => {
                    if let Some(Tab { kind: TabKind::Table(t), .. }) = self.tab_mut(tab) {
                        t.total = result.ok();
                    }
                }
            }
        }
    }

    /// Send work that was queued while the connection was coming up.
    fn flush_pending(&mut self, conn: &str) {
        for tab in self.tabs.iter_mut().filter(|t| t.conn == conn) {
            match &mut tab.kind {
                TabKind::Console(c) => {
                    if let Some(sql) = c.pending.take() {
                        let page = c.paged.is_some().then_some(c.offset);
                        self.worker.send(Cmd::Query { conn: tab.conn.clone(), tab: tab.id, sql, page });
                    }
                }
                TabKind::Table(t) if t.pending => {
                    t.pending = false;
                    self.worker.send(Cmd::TableData {
                        conn: tab.conn.clone(),
                        tab: tab.id,
                        query: t.query(),
                    });
                }
                _ => {}
            }
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

    fn open_console(&mut self, ci: usize) {
        let conn = self.conns[ci].cfg.id.clone();
        let n = self
            .tabs
            .iter()
            .filter(|t| t.conn == conn && matches!(t.kind, TabKind::Console(_)))
            .count();
        let name = self.conns[ci].cfg.display_name();
        let title = if n == 0 { name } else { format!("{name} ({})", n + 1) };
        self.push_tab(
            conn,
            title,
            TabKind::Console(Console::new()),
        );
        self.ensure_connected(ci);
    }

    fn open_table(&mut self, ci: usize, schema: String, table: String) {
        let conn = self.conns[ci].cfg.id.clone();
        if let Some(i) = self.tabs.iter().position(|t| {
            t.conn == conn && matches!(&t.kind, TabKind::Table(v) if v.schema == schema && v.table == table)
        }) {
            self.active = i;
            return;
        }
        let title = if schema == "public" { table.clone() } else { format!("{schema}.{table}") };
        self.push_tab(
            conn,
            title,
            TabKind::Table(TableView {
                schema,
                table,
                filter: String::new(),
                applied_filter: String::new(),
                order: None,
                page: 0,
                loading: false,
                pending: false,
                data: None,
                total: None,
                grid: GridState::default(),
                show_map: false,
                map: Default::default(),
            }),
        );
        self.tab_action(self.active, TabAction::Reload);
    }

    fn push_tab(&mut self, conn: ConnId, title: String, kind: TabKind) {
        self.tabs.push(Tab { id: self.next_tab, conn, title, kind });
        self.next_tab += 1;
        self.active = self.tabs.len() - 1;
    }

    fn close_tab(&mut self, i: usize) {
        close_tabs(&mut self.tabs, &mut self.active, CloseWhich::One(i));
    }

    fn save_settings(&mut self) {
        if let Err(e) = config::save_settings(&self.settings) {
            self.notice = Some(format!("Could not save settings: {e}"));
        }
    }

    /// Re-read the queries folder (files may be edited outside the app).
    fn reload_queries(&mut self) {
        self.saved = config::load_queries();
    }

    fn report(&mut self, result: Result<(), String>) {
        if let Err(e) = result {
            self.toast = Some((e, theme::ERROR(), Instant::now()));
        }
    }

    /// Point consoles linked to saved query `old` at `new` (None = unlink).
    fn relink(&mut self, old: &str, new: Option<(&str, &str)>) {
        for tab in &mut self.tabs {
            if let TabKind::Console(c) = &mut tab.kind {
                if c.saved_id.as_deref() == Some(old) {
                    c.saved_id = new.map(|(id, _)| id.to_string());
                    if let Some((_, name)) = new {
                        tab.title = name.to_string();
                    }
                }
            }
        }
    }

    fn open_saved(&mut self, i: usize) {
        let Some(q) = self.saved.get(i).cloned() else { return };
        let linked = self.tabs.iter().position(|t| matches!(&t.kind, TabKind::Console(c) if c.saved_id.as_ref() == Some(&q.id)));
        if let Some(t) = linked {
            self.active = t;
            return;
        }
        let current = self.tabs.get(self.active).map(|t| t.conn.clone());
        let ci = q
            .connection
            .as_deref()
            .and_then(|id| self.conn_index(id))
            .or_else(|| current.and_then(|id| self.conn_index(&id)))
            .unwrap_or(0);
        if self.conns.is_empty() {
            return;
        }
        self.open_console(ci);
        if let Some(tab) = self.tabs.last_mut() {
            tab.title = q.name.clone();
            if let TabKind::Console(c) = &mut tab.kind {
                c.sql = q.sql.clone();
                c.saved_id = Some(q.id.clone());
            }
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::NewConnection => {
                self.dialog = Some(Dialog::new(ConnectionConfig::default(), true));
            }
            Action::Connect(ci) => {
                let c = &mut self.conns[ci];
                c.status = Status::Connecting;
                c.clear_cache();
                self.worker.send(Cmd::Connect(c.cfg.clone()));
            }
            Action::Disconnect(ci) => {
                let c = &mut self.conns[ci];
                self.worker.send(Cmd::Disconnect(c.cfg.id.clone()));
                c.status = Status::Disconnected;
                c.clear_cache();
            }
            Action::Edit(ci) => {
                self.dialog = Some(Dialog::new(self.conns[ci].cfg.clone(), false));
            }
            Action::Delete(ci) => {
                let id = self.conns[ci].cfg.id.clone();
                self.worker.send(Cmd::Disconnect(id.clone()));
                while let Some(i) = self.tabs.iter().position(|t| t.conn == id) {
                    self.close_tab(i);
                }
                self.conns.remove(ci);
                self.save_config();
            }
            Action::Refresh(ci) => {
                self.conns[ci].clear_cache();
                self.worker.send(Cmd::Catalog(self.conns[ci].cfg.id.clone()));
            }
            Action::NewConsole(ci) => self.open_console(ci),
            Action::OpenTable(ci, s, t) => self.open_table(ci, s, t),
            Action::LoadSchemas(ci) => {
                let c = &mut self.conns[ci];
                c.schemas = Some(Load::Loading);
                self.worker.send(Cmd::Schemas(c.cfg.id.clone()));
            }
            Action::LoadTables(ci, schema) => {
                let c = &mut self.conns[ci];
                c.tables.insert(schema.clone(), Load::Loading);
                self.worker.send(Cmd::Tables(c.cfg.id.clone(), schema));
            }
            Action::LoadColumns(ci, schema, table) => {
                let c = &mut self.conns[ci];
                c.columns.insert((schema.clone(), table.clone()), Load::Loading);
                self.worker.send(Cmd::Columns(c.cfg.id.clone(), schema, table));
            }
            Action::Toggle(key) => {
                if !self.expanded.remove(&key) {
                    self.expanded.insert(key);
                }
            }
            Action::Select(key) => self.selected = Some(key),
            Action::ActivateTab(i) => self.active = i,
            Action::Close(which) => close_tabs(&mut self.tabs, &mut self.active, which),
            Action::OpenSaved(i) => self.open_saved(i),
            Action::SetTheme(choice) => {
                self.settings.theme = choice;
                theme::set_choice(&self.egui_ctx, choice);
                self.save_settings();
            }
            Action::SetPalette(dark, name) => {
                if dark {
                    self.settings.dark_theme = name.into();
                } else {
                    self.settings.light_theme = name.into();
                }
                theme::set_themes(&self.egui_ctx, &self.settings.dark_theme, &self.settings.light_theme);
                // Show the chosen theme right away.
                let wanted = if dark { ThemeChoice::Dark } else { ThemeChoice::Light };
                if self.settings.theme != ThemeChoice::System || theme::is_dark() != dark {
                    self.settings.theme = wanted;
                    theme::set_choice(&self.egui_ctx, wanted);
                }
                self.save_settings();
            }
            Action::SetAutoUpdate(on) => {
                self.settings.check_updates = on;
                self.save_settings();
            }
            Action::CheckUpdate => {
                if !matches!(self.update, UpdateState::Checking { .. } | UpdateState::Installing(_)) {
                    self.update = UpdateState::Checking { manual: true };
                    self.worker.send(Cmd::CheckUpdate);
                }
            }
            Action::InstallUpdate => {
                if let UpdateState::Available(release) = &self.update {
                    match update::current_bundle() {
                        Some(app) => {
                            let release = release.clone();
                            self.worker.send(Cmd::InstallUpdate { release: release.clone(), app });
                            self.update = UpdateState::Installing(release);
                        }
                        // Running from source: send the user to the release page.
                        None => self.egui_ctx.open_url(egui::OpenUrl::new_tab(&release.page_url)),
                    }
                }
            }
            Action::RenameSaved(i) => {
                if let Some(q) = self.saved.get(i) {
                    self.name_prompt = Some(NamePrompt { name: q.name.clone(), target: NameTarget::Rename(q.id.clone()) });
                }
            }
            Action::DeleteSaved(i) => {
                if let Some(q) = self.saved.get(i).cloned() {
                    let result = config::delete_query(&q);
                    if result.is_ok() {
                        self.relink(&q.id, None);
                        self.toast = Some((format!("Moved “{}” to the Trash", q.name), theme::MUTED(), Instant::now()));
                    }
                    self.report(result);
                    self.reload_queries();
                }
            }
            Action::RevealSaved(i) => {
                if let Some(path) = self.saved.get(i).map(|q| q.path.clone()) {
                    let result = config::reveal_in_finder(&path);
                    self.report(result);
                }
            }
            Action::OpenQueriesFolder => {
                if let Some(dir) = config::queries_dir() {
                    let _ = std::fs::create_dir_all(&dir);
                    self.egui_ctx.open_url(egui::OpenUrl::same_tab(format!("file://{}", dir.display())));
                }
            }
        }
    }

    fn tab_action(&mut self, ti: usize, action: TabAction) {
        let Some(ci) = self.conn_index(&self.tabs[ti].conn) else { return };
        let connected = matches!(self.conns[ci].status, Status::Connected(_));
        let mut needs_connect = false;
        let catalog = self.conns[ci].catalog.clone();
        let tab = &mut self.tabs[ti];
        match (action, &mut tab.kind) {
            (TabAction::Run { all }, TabKind::Console(c)) => {
                if c.running.is_some() {
                    return;
                }
                let at = sql::char_to_byte(&c.sql, c.cursor.start);
                let text = if all {
                    c.sql.clone()
                } else if c.cursor.start != c.cursor.end {
                    let b = sql::char_to_byte(&c.sql, c.cursor.end);
                    c.sql[at..b].to_string()
                } else if let Some((_, line)) = rgrip_core::commands::command_line_at(&c.sql, at) {
                    // A command line is its own statement (it has no `;`).
                    line.to_string()
                } else {
                    sql::statement_at(&c.sql, at).map(|r| c.sql[r].to_string()).unwrap_or_default()
                };
                // Slash commands are ours, not SQL: expand before sending.
                let text = match rgrip_core::commands::expand_all(&text, &catalog) {
                    Ok(t) => t,
                    Err(e) => {
                        self.toast = Some((e, theme::WARN(), Instant::now()));
                        return;
                    }
                };
                if text.trim().is_empty() {
                    return;
                }
                c.running = Some(Instant::now());
                c.popup = None;
                let lower = text.to_lowercase();
                c.ddl = ["create", "alter", "drop", "truncate", "comment"]
                    .iter()
                    .any(|k| lower.split(|ch: char| !ch.is_alphanumeric()).any(|w| w == *k));
                c.offset = 0;
                c.last_sql = text.clone();
                c.paged = sql::paged_query(&text, 1, 0).map(|_| text.clone());
                let page = c.paged.is_some().then_some(0);
                if connected {
                    self.worker.send(Cmd::Query { conn: tab.conn.clone(), tab: tab.id, sql: text, page });
                } else {
                    c.pending = Some(text);
                    needs_connect = true;
                }
            }
            (TabAction::Page(forward), TabKind::Console(c)) => {
                let Some(sql) = c.paged.clone().filter(|_| c.running.is_none() && connected) else { return };
                let size = PAGE_SIZE as usize;
                c.offset = if forward { c.offset + size } else { c.offset.saturating_sub(size) };
                c.running = Some(Instant::now());
                self.worker.send(Cmd::Query { conn: tab.conn.clone(), tab: tab.id, sql, page: Some(c.offset) });
            }
            (TabAction::Save, TabKind::Console(c)) => {
                let existing = c.saved_id.as_ref().and_then(|id| self.saved.iter_mut().find(|q| &q.id == id));
                match existing {
                    Some(q) => {
                        q.sql = c.sql.clone();
                        q.connection = Some(tab.conn.clone());
                        let (name, result) = (q.name.clone(), config::update_query(q));
                        if result.is_ok() {
                            self.toast = Some((format!("Saved “{name}”"), theme::OK(), Instant::now()));
                        }
                        self.report(result);
                    }
                    None => {
                        let name = default_query_name(&c.sql, self.saved.len() + 1);
                        self.name_prompt = Some(NamePrompt { name, target: NameTarget::NewFrom(tab.id) });
                    }
                }
            }
            (TabAction::ExportAll(format, path), kind) => {
                let (sql, table) = match kind {
                    TabKind::Console(c) => match &c.paged {
                        Some(sql) => (sql.clone(), "query_result".to_string()),
                        None => return,
                    },
                    TabKind::Table(t) => (
                        rgrip_core::postgres::table_select_sql(&t.query()),
                        format!("{}.{}", t.schema, t.table),
                    ),
                };
                self.toast = Some((format!("Exporting to {}…", path.display()), theme::MUTED(), Instant::now()));
                self.worker.send(Cmd::ExportAll { conn: tab.conn.clone(), sql, format, table, path });
            }
            (TabAction::Reproject(col), TabKind::Console(c)) => {
                let Some(query) = c.paged.clone() else { return };
                let alias = format!("{col}_4326");
                c.sql = reproject_sql(&query, &col, &alias);
                c.map.prefer = Some(alias);
                c.view = View::Map(0);
                c.cursor = 0..0;
                return self.tab_action(ti, TabAction::Run { all: true });
            }
            (TabAction::Notify(text), _) => self.toast = Some((text, theme::WARN(), Instant::now())),
            (TabAction::Cancel, TabKind::Console(c)) => {
                if c.pending.take().is_some() {
                    c.running = None;
                } else if c.running.is_some() {
                    self.worker.send(Cmd::Cancel(tab.conn.clone()));
                }
            }
            (TabAction::Reload, TabKind::Table(t)) => {
                t.loading = true;
                t.total = None;
                if connected {
                    self.worker.send(Cmd::TableData {
                        conn: tab.conn.clone(),
                        tab: tab.id,
                        query: t.query(),
                    });
                } else {
                    t.pending = true;
                    needs_connect = true;
                }
            }
            _ => {}
        }
        if needs_connect {
            self.ensure_connected(ci);
        }
    }
}

impl App {
    /// Debug builds only: `RGRIP_DEMO_SQL` opens a console on the first
    /// connection and runs it; `RGRIP_DEMO_TABLE=schema.table` opens a table.
    /// Used to exercise and screenshot the UI without clicking.
    #[cfg(debug_assertions)]
    fn demo_hook(&mut self) {
        if let Ok(name) = std::env::var("RGRIP_DEMO_CONN") {
            if let Some(i) = self.conns.iter().position(|c| c.cfg.display_name() == name).filter(|&i| i > 0) {
                self.conns.swap(0, i);
            }
        }
        // `RGRIP_DEMO_COMPLETE='select u.| from users u'`: `|` is the cursor.
        if let (Ok(text), Some(tab)) = (std::env::var("RGRIP_DEMO_COMPLETE"), self.tabs.first_mut()) {
            let catalog = self.conns[0].catalog.clone();
            let plain = text.replace('|', "");
            if let TabKind::Console(c) = &mut tab.kind {
                if c.popup.is_none() && !catalog.tables.is_empty() && (c.sql.is_empty() || c.sql == plain) {
                    let at = text.find('|').unwrap_or(text.len());
                    let chars = plain[..at].chars().count();
                    c.sql = plain;
                    c.cursor = chars..chars;
                    c.focus = true;
                    let id = Id::new(("editor", tab.id));
                    let ctx = self.egui_ctx.clone();
                    let mut state = TextEdit::load_state(&ctx, id).unwrap_or_default();
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(chars))));
                    state.store(&ctx, id);
                    c.popup = complete::complete(&c.sql, at, &catalog, false).map(Popup::new);
                }
            }
        }
        if std::env::var("RGRIP_DEMO_VIEW").as_deref() == Ok("map") {
            if let Some(TabKind::Console(c)) = self.tabs.first_mut().map(|t| &mut t.kind) {
                if matches!(c.output, Some(Ok(_))) && c.view == View::Result(0) {
                    c.view = View::Map(0);
                }
            }
        }
        if self.conns.is_empty() || self.next_tab != 1 {
            return;
        }
        if std::env::var("RGRIP_DEMO_COMPLETE").is_ok() {
            self.expanded.insert(format!("c/{}", self.conns[0].cfg.id));
            self.open_console(0);
        }
        if let Ok(sql) = std::env::var("RGRIP_DEMO_SQL") {
            self.expanded.insert(format!("c/{}", self.conns[0].cfg.id));
            self.open_console(0);
            if let Some(TabKind::Console(c)) = self.tabs.last_mut().map(|t| &mut t.kind) {
                c.sql = sql.replace("\\n", "\n");
            }
            self.tab_action(self.active, TabAction::Run { all: true });
        }
        if let Ok(tab) = std::env::var("RGRIP_DEMO_DIALOG") {
            self.apply(Action::Edit(0));
            if let Some(d) = &mut self.dialog {
                d.tab = match tab.as_str() {
                    "ssl" => DialogTab::Ssl,
                    "ssh" => DialogTab::Ssh,
                    _ => DialogTab::General,
                };
                d.cfg.ssh.enabled |= tab == "ssh";
            }
        }
        if let Ok(t) = std::env::var("RGRIP_DEMO_TABLE") {
            if let Some((schema, table)) = t.split_once('.') {
                self.open_table(0, schema.into(), table.into());
            }
        }
    }
}

impl eframe::App for App {
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

        title_bar(ctx);

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

// ---- explorer ---------------------------------------------------------

impl App {
    fn explorer(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Explorer").size(12.0).color(theme::MUTED()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.settings_menu(ui, actions);
                if icon_button(ui, "+").on_hover_text("New connection").clicked() {
                    actions.push(Action::NewConnection);
                }
            });
        });
        ui.add_space(6.0);
        ui.add(
            TextEdit::singleline(&mut self.filter)
                .hint_text(hint("Search tables"))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(9, 6)),
        );
        ui.add_space(8.0);

        if self.conns.is_empty() {
            widgets::tree_note(ui, 0, "No connections yet.", theme::MUTED());
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                if ui.link("Add a connection").clicked() {
                    actions.push(Action::NewConnection);
                }
            });
            return;
        }

        let filter = self.filter.trim().to_lowercase();
        let tree = Tree { expanded: &self.expanded, selected: self.selected.as_deref(), filter: &filter };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (ci, conn) in self.conns.iter().enumerate() {
                tree.conn(ui, ci, conn, actions);
            }
            self.saved_section(ui, &filter, actions);
        });
    }

    fn settings_menu(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        const WIDTH: f32 = 260.0;
        let button = icon_button(ui, "⚙").on_hover_text("Settings");
        let mut popup = egui::Popup::menu(&button);
        // Debug builds: RGRIP_DEMO_MENU=settings opens this menu (screenshots).
        if cfg!(debug_assertions) && std::env::var("RGRIP_DEMO_MENU").as_deref() == Ok("settings") {
            popup = popup.open(true);
        }
        popup.show(|ui| {
            ui.set_width(WIDTH);
            menu_heading(ui, "Appearance");
            if let Some(choice) = appearance_switch(ui, self.settings.theme, WIDTH) {
                actions.push(Action::SetTheme(choice));
            }
            ui.add_space(4.0);
            for (dark, label, themes, current) in [
                (true, "Dark theme", theme::DARK_THEMES, &self.settings.dark_theme),
                (false, "Light theme", theme::LIGHT_THEMES, &self.settings.light_theme),
            ] {
                ui.menu_button(format!("{label}:  {current}"), |ui| {
                    ui.set_width(220.0);
                    for t in themes {
                        if theme_row(ui, t, t.name == current.as_str(), 220.0).clicked() {
                            actions.push(Action::SetPalette(dark, t.name));
                        }
                    }
                });
            }
            ui.separator();
            menu_heading(ui, "Updates");
            let busy = matches!(self.update, UpdateState::Checking { .. } | UpdateState::Installing(_));
            if ui.add_enabled(!busy, Button::new("Check for updates…")).clicked() {
                actions.push(Action::CheckUpdate);
                ui.close();
            }
            let mut auto = self.settings.check_updates;
            if ui.checkbox(&mut auto, "Check automatically").changed() {
                actions.push(Action::SetAutoUpdate(auto));
            }
            ui.separator();
            ui.label(RichText::new(format!("rgrip {}", update::CURRENT)).size(11.5).color(theme::FAINT()));
        });
    }

    fn saved_section(&self, ui: &mut Ui, filter: &str, actions: &mut Vec<Action>) {
        if self.saved.is_empty() {
            return;
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Saved queries").size(12.0).color(theme::MUTED()));
            ui.label(RichText::new(self.saved.len().to_string()).size(11.0).color(theme::FAINT()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, "↗").on_hover_text("Open queries folder in Finder").clicked() {
                    actions.push(Action::OpenQueriesFolder);
                }
            });
        });
        ui.add_space(4.0);
        let active_saved = self.tabs.get(self.active).and_then(|t| match &t.kind {
            TabKind::Console(c) => c.saved_id.clone(),
            TabKind::Table(_) => None,
        });
        for (i, q) in self.saved.iter().enumerate() {
            if !filter.is_empty() && !q.name.to_lowercase().contains(filter) {
                continue;
            }
            let conn = q.connection.as_deref().and_then(|id| self.conn_index(id)).map(|ci| self.conns[ci].cfg.display_name());
            let resp = widgets::tree_row(
                ui,
                TreeRow {
                    depth: 0,
                    open: None,
                    icon: Icon::Query,
                    label: &q.name,
                    label_color: theme::TEXT(),
                    detail: conn.as_deref(),
                    selected: active_saved.as_ref() == Some(&q.id),
                },
            );
            if resp.clicked() {
                actions.push(Action::OpenSaved(i));
            }
            let preview: String = q.sql.lines().take(12).collect::<Vec<_>>().join("\n");
            resp.on_hover_text(RichText::new(preview).monospace().size(11.5)).context_menu(|ui| {
                if ui.button("Open").clicked() {
                    actions.push(Action::OpenSaved(i));
                    ui.close();
                }
                if ui.button("Rename…").clicked() {
                    actions.push(Action::RenameSaved(i));
                    ui.close();
                }
                if ui.button("Reveal in Finder").clicked() {
                    actions.push(Action::RevealSaved(i));
                    ui.close();
                }
                ui.separator();
                if ui.button(RichText::new("Move to Trash").color(theme::ERROR())).clicked() {
                    actions.push(Action::DeleteSaved(i));
                    ui.close();
                }
            });
        }
    }

    fn status_bar(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let toast = self.toast.as_ref().filter(|(_, _, at)| at.elapsed() < Duration::from_secs(8));
            if let Some((text, color, _)) = toast {
                ui.label(RichText::new(text).size(11.5).color(*color));
                ui.ctx().request_repaint_after(Duration::from_secs(1));
            } else if let Some(n) = &self.notice {
                ui.label(RichText::new(n).size(11.5).color(theme::ERROR()));
            } else if let Some(c) = self.tabs.get(self.active).and_then(|t| self.conn_index(&t.conn)).map(|ci| &self.conns[ci]) {
                let (color, text) = status_style(&c.status);
                widgets::status_dot(ui, color);
                ui.label(
                    RichText::new(format!("{}   {}:{}/{}   {text}", c.cfg.display_name(), c.cfg.host, c.cfg.port, c.cfg.database))
                        .size(11.5)
                        .color(theme::MUTED()),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match &self.update {
                    UpdateState::Available(r) => {
                        let label = if update::current_bundle().is_some() { "Install & restart" } else { "Download" };
                        if ui
                            .add(Button::new(RichText::new(label).size(11.5).color(theme::BG())).fill(theme::ACCENT()).small())
                            .on_hover_text(format!("rgrip {} → {}", update::CURRENT, r.version))
                            .clicked()
                        {
                            actions.push(Action::InstallUpdate);
                        }
                        ui.label(RichText::new(format!("Update {} available", r.version)).size(11.5).color(theme::ACCENT()));
                        ui.add_space(12.0);
                    }
                    UpdateState::Installing(r) => {
                        ui.label(RichText::new(format!("Installing {}…", r.version)).size(11.5).color(theme::MUTED()));
                        ui.add(egui::Spinner::new().size(11.0));
                        ui.add_space(12.0);
                    }
                    UpdateState::Checking { manual: true } => {
                        ui.label(RichText::new("Checking for updates…").size(11.5).color(theme::MUTED()));
                        ui.add_space(12.0);
                    }
                    _ => {}
                }
                for (keys, what) in [("⌘W", "close tab"), ("⇧⌘↩", "run all"), ("⌘↩", "run")] {
                    ui.label(RichText::new(what).size(11.5).color(theme::FAINT()));
                    ui.label(RichText::new(keys).size(11.5).color(theme::MUTED()));
                    ui.add_space(8.0);
                }
            });
        });
    }
}

fn menu_heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(theme::FAINT()));
}

/// System / Light / Dark segmented control with icons, filling `width`.
fn appearance_switch(ui: &mut Ui, current: ThemeChoice, width: f32) -> Option<ThemeChoice> {
    let options = [
        (ThemeChoice::System, Icon::System, "System"),
        (ThemeChoice::Light, Icon::Sun, "Light"),
        (ThemeChoice::Dark, Icon::Moon, "Dark"),
    ];
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 30.0), Sense::hover());
    ui.painter().rect_filled(rect, 7.0, theme::HOVER());
    let seg_w = (width - 4.0) / options.len() as f32;
    let mut picked = None;
    for (i, (choice, icon, label)) in options.into_iter().enumerate() {
        let seg = egui::Rect::from_min_size(rect.min + egui::vec2(2.0 + i as f32 * seg_w, 2.0), egui::vec2(seg_w, 26.0));
        let resp = ui.interact(seg, Id::new(("appearance", i)), Sense::click());
        let selected = current == choice;
        if selected {
            ui.painter().rect_filled(seg, 5.0, theme::SURFACE());
            ui.painter().rect_stroke(seg, 5.0, Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(seg, 5.0, theme::SELECTED().gamma_multiply(0.5));
        }
        let text_color = if selected { theme::TEXT() } else { theme::MUTED() };
        let galley = ui.painter().layout_no_wrap(label.into(), FontId::proportional(12.5), text_color);
        let total = 16.0 + 6.0 + galley.size().x;
        let left = seg.center().x - total / 2.0;
        let tint = (!selected).then(theme::MUTED);
        widgets::paint_icon_tinted(ui.painter(), egui::pos2(left + 8.0, seg.center().y), icon, tint);
        ui.painter().galley(egui::pos2(left + 22.0, seg.center().y - galley.size().y / 2.0), galley, text_color);
        if resp.clicked() && !selected {
            picked = Some(choice);
        }
    }
    picked
}

/// A theme entry: swatch (background, accent, text), name, check when active.
fn theme_row(ui: &mut Ui, t: &theme::Palette, selected: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 26.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 5.0, theme::HOVER());
    }
    let swatch = egui::Rect::from_min_size(egui::pos2(rect.left() + 6.0, rect.center().y - 7.0), egui::vec2(26.0, 14.0));
    p.rect_filled(swatch, 4.0, t.bg);
    p.rect_stroke(swatch, 4.0, Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
    p.circle_filled(swatch.center() - egui::vec2(4.5, 0.0), 3.5, t.accent);
    p.circle_filled(swatch.center() + egui::vec2(4.5, 0.0), 3.5, t.text);
    let color = if selected { theme::TEXT() } else { theme::MUTED() };
    p.text(egui::pos2(swatch.right() + 9.0, rect.center().y), egui::Align2::LEFT_CENTER, t.name, FontId::proportional(13.0), color);
    if selected {
        p.text(egui::pos2(rect.right() - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, "✓", FontId::proportional(13.0), theme::ACCENT());
    }
    resp
}

fn status_style(s: &Status) -> (Color32, String) {
    match s {
        Status::Disconnected => (theme::FAINT(), "disconnected".into()),
        Status::Connecting => (theme::WARN(), "connecting…".into()),
        Status::Connected(v) => (theme::OK(), v.clone()),
        Status::Failed(_) => (theme::ERROR(), "connection failed".into()),
    }
}

/// Read-only view of explorer state; interactions are returned as actions.
struct Tree<'a> {
    expanded: &'a HashSet<String>,
    selected: Option<&'a str>,
    filter: &'a str,
}

impl Tree<'_> {
    fn row(&self, ui: &mut Ui, key: &str, depth: usize, expandable: bool, icon: Icon, label: &str, detail: Option<&str>, actions: &mut Vec<Action>) -> (egui::Response, bool) {
        let open = self.expanded.contains(key);
        let resp = widgets::tree_row(
            ui,
            TreeRow {
                depth,
                open: expandable.then_some(open),
                icon,
                label,
                label_color: theme::TEXT(),
                detail,
                selected: self.selected == Some(key),
            },
        );
        if resp.clicked() {
            actions.push(Action::Select(key.to_string()));
            if expandable {
                actions.push(Action::Toggle(key.to_string()));
            }
        }
        (resp, open)
    }

    fn conn(&self, ui: &mut Ui, ci: usize, conn: &Conn, actions: &mut Vec<Action>) {
        let key = format!("c/{}", conn.cfg.id);
        let (dot, _) = status_style(&conn.status);
        let detail = match conn.status {
            Status::Connecting => Some("connecting…"),
            _ => None,
        };
        let name = conn.cfg.display_name();
        let (resp, open) = self.row(ui, &key, 0, true, Icon::Database(dot), &name, detail, actions);
        if resp.clicked() && !open && matches!(conn.status, Status::Disconnected | Status::Failed(_)) {
            actions.push(Action::Connect(ci));
        }
        resp.on_hover_text(format!("{}@{}:{}/{}", conn.cfg.user, conn.cfg.host, conn.cfg.port, conn.cfg.database))
            .context_menu(|ui| conn_menu(ui, ci, conn, actions));
        if !open {
            return;
        }
        match &conn.status {
            Status::Disconnected | Status::Connecting => {}
            Status::Failed(e) => widgets::tree_note(ui, 1, e, theme::ERROR()),
            Status::Connected(_) => match &conn.schemas {
                None => actions.push(Action::LoadSchemas(ci)),
                Some(Load::Loading) => widgets::tree_note(ui, 1, "loading…", theme::FAINT()),
                Some(Load::Failed(e)) => widgets::tree_note(ui, 1, e, theme::ERROR()),
                Some(Load::Ready(list)) => {
                    for schema in list {
                        self.schema(ui, ci, conn, schema, actions);
                    }
                }
            },
        }
    }

    fn schema(&self, ui: &mut Ui, ci: usize, conn: &Conn, schema: &str, actions: &mut Vec<Action>) {
        let key = format!("s/{}/{schema}", conn.cfg.id);
        let count = match conn.tables.get(schema) {
            Some(Load::Ready(t)) => Some(t.len().to_string()),
            _ => None,
        };
        let (_, open) = self.row(ui, &key, 1, true, Icon::Schema, schema, count.as_deref(), actions);
        if !open {
            return;
        }
        match conn.tables.get(schema) {
            None => actions.push(Action::LoadTables(ci, schema.to_string())),
            Some(Load::Loading) => widgets::tree_note(ui, 2, "loading…", theme::FAINT()),
            Some(Load::Failed(e)) => widgets::tree_note(ui, 2, e, theme::ERROR()),
            Some(Load::Ready(tables)) => {
                let mut shown = 0;
                for t in tables.iter().filter(|t| self.filter.is_empty() || t.name.to_lowercase().contains(self.filter)) {
                    shown += 1;
                    self.table(ui, ci, conn, schema, t, actions);
                }
                if shown == 0 {
                    let msg = if tables.is_empty() { "no tables" } else { "no matches" };
                    widgets::tree_note(ui, 2, msg, theme::FAINT());
                }
            }
        }
    }

    fn table(&self, ui: &mut Ui, ci: usize, conn: &Conn, schema: &str, t: &TableInfo, actions: &mut Vec<Action>) {
        let key = format!("t/{}/{schema}/{}", conn.cfg.id, t.name);
        let icon = if matches!(t.kind, TableKind::Table | TableKind::Partitioned) { Icon::Table } else { Icon::View };
        let (resp, open) = self.row(ui, &key, 2, true, icon, &t.name, t.kind.tag(), actions);
        if resp.double_clicked() {
            actions.push(Action::OpenTable(ci, schema.to_string(), t.name.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button("Open data").clicked() {
                actions.push(Action::OpenTable(ci, schema.to_string(), t.name.clone()));
                ui.close();
            }
            if ui.button("Copy name").clicked() {
                ui.ctx().copy_text(format!("{}.{}", sql::quote_ident(schema), sql::quote_ident(&t.name)));
                ui.close();
            }
        });
        if !open {
            return;
        }
        match conn.columns.get(&(schema.to_string(), t.name.clone())) {
            None => actions.push(Action::LoadColumns(ci, schema.to_string(), t.name.clone())),
            Some(Load::Loading) => widgets::tree_note(ui, 3, "loading…", theme::FAINT()),
            Some(Load::Failed(e)) => widgets::tree_note(ui, 3, e, theme::ERROR()),
            Some(Load::Ready(cols)) => {
                for c in cols {
                    let key = format!("{key}/{}", c.name);
                    let icon = if c.primary_key { Icon::Key } else { Icon::Column };
                    let mut ty = sql::short_type(&c.data_type);
                    if !c.nullable {
                        ty += " · not null";
                    }
                    let (resp, _) = self.row(ui, &key, 3, false, icon, &c.name, Some(&ty), actions);
                    if let Some(d) = &c.default {
                        resp.on_hover_text(format!("default {d}"));
                    }
                }
            }
        }
    }
}

fn conn_menu(ui: &mut Ui, ci: usize, conn: &Conn, actions: &mut Vec<Action>) {
    if ui.button("New console").clicked() {
        actions.push(Action::NewConsole(ci));
        ui.close();
    }
    ui.menu_button("Copy connection URL", |ui| {
        if ui.button("Without password").clicked() {
            ui.ctx().copy_text(rgrip_core::conn_string::to_url(&conn.cfg, false));
            ui.close();
        }
        if ui.button("With password").clicked() {
            ui.ctx().copy_text(rgrip_core::conn_string::to_url(&conn.cfg, true));
            ui.close();
        }
    });
    if matches!(conn.status, Status::Connected(_)) {
        if ui.button("Refresh").clicked() {
            actions.push(Action::Refresh(ci));
            ui.close();
        }
        if ui.button("Disconnect").clicked() {
            actions.push(Action::Disconnect(ci));
            ui.close();
        }
    } else if ui.button("Connect").clicked() {
        actions.push(Action::Connect(ci));
        ui.close();
    }
    ui.separator();
    if ui.button("Edit…").clicked() {
        actions.push(Action::Edit(ci));
        ui.close();
    }
    if ui.button(RichText::new("Delete").color(theme::ERROR())).clicked() {
        actions.push(Action::Delete(ci));
        ui.close();
    }
}

// ---- tabs -------------------------------------------------------------

impl App {
    fn tab_bar(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        Frame::new()
            .fill(theme::BG())
            .inner_margin(Margin { left: 10, right: 10, top: 6, bottom: 0 })
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("tabs")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for (i, tab) in self.tabs.iter().enumerate() {
                                let dirty = match &tab.kind {
                                    TabKind::Console(c) => c.saved_id.as_ref().is_some_and(|id| {
                                        self.saved.iter().find(|q| &q.id == id).is_some_and(|q| q.sql != c.sql)
                                    }),
                                    TabKind::Table(_) => false,
                                };
                                tab_button(ui, i, tab, i == self.active, dirty, self.tabs.len(), actions);
                            }
                        });
                    });
            });
        let rect = ui.available_rect_before_wrap();
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, theme::BORDER()));
    }

    fn tab_content(&mut self, ui: &mut Ui) -> Option<TabAction> {
        let catalog = {
            let conn = &self.tabs.get(self.active)?.conn;
            self.conn_index(conn).map(|ci| self.conns[ci].catalog.clone()).unwrap_or_default()
        };
        let tab = self.tabs.get_mut(self.active)?;
        let id = tab.id;
        match &mut tab.kind {
            TabKind::Console(c) => console_ui(ui, id, c, &catalog),
            TabKind::Table(t) => table_ui(ui, id, t),
        }
    }
}

/// One tab in the tab bar. Returns the tab's rect.
fn tab_button(
    ui: &mut Ui,
    i: usize,
    tab: &Tab,
    active: bool,
    dirty: bool,
    count: usize,
    actions: &mut Vec<Action>,
) -> egui::Rect {
    const PAD_L: f32 = 10.0;
    const ICON: f32 = 14.0;
    const GAP: f32 = 7.0;
    const CLOSE: f32 = 18.0;
    const PAD_R: f32 = 6.0;

    let color = if active { theme::TEXT() } else { theme::MUTED() };
    // A dot marks unsaved changes to a saved query.
    let title = if dirty { format!("{} •", tab.title) } else { tab.title.clone() };
    let galley = ui.painter().layout_no_wrap(title, TextStyle::Body.resolve(ui.style()), color);
    let size = egui::vec2(PAD_L + ICON + GAP + galley.size().x + GAP + CLOSE + PAD_R, 30.0);

    // The tab background is registered first and the close button last, so
    // the close button sits on top and receives its own clicks.
    let (rect, tab_resp) = ui.allocate_exact_size(size, Sense::click());
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - PAD_R - CLOSE / 2.0, rect.center().y),
        egui::vec2(CLOSE, CLOSE),
    );
    let close_resp = ui.interact(close_rect, Id::new(("tab-close", tab.id)), Sense::click()).on_hover_text("Close  ⌘W");

    let painter = ui.painter();
    let fill = if active {
        theme::SURFACE()
    } else if tab_resp.hovered() {
        theme::HOVER()
    } else {
        theme::BG()
    };
    painter.rect_filled(rect, egui::CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, fill);
    let cy = rect.center().y;
    let icon = match tab.kind {
        TabKind::Console(_) => Icon::Console,
        TabKind::Table(_) => Icon::Table,
    };
    widgets::paint_icon(painter, egui::pos2(rect.left() + PAD_L + ICON / 2.0, cy), icon);
    painter.galley(egui::pos2(rect.left() + PAD_L + ICON + GAP, cy - galley.size().y / 2.0), galley, color);
    if close_resp.hovered() {
        painter.rect_filled(close_rect, 4.0, theme::BORDER());
    }
    let x_color = if close_resp.hovered() { theme::TEXT() } else if active || tab_resp.hovered() { theme::MUTED() } else { theme::FAINT() };
    painter.text(close_rect.center(), egui::Align2::CENTER_CENTER, "×", FontId::proportional(14.0), x_color);

    if close_resp.clicked() || tab_resp.middle_clicked() {
        actions.push(Action::Close(CloseWhich::One(i)));
    } else if tab_resp.clicked() {
        actions.push(Action::ActivateTab(i));
    }
    tab_resp.context_menu(|ui| {
        let mut item = |ui: &mut Ui, label: &str, enabled: bool, which: CloseWhich| {
            if ui.add_enabled(enabled, Button::new(label)).clicked() {
                actions.push(Action::Close(which));
                ui.close();
            }
        };
        item(ui, "Close", true, CloseWhich::One(i));
        item(ui, "Close others", count > 1, CloseWhich::Others(i));
        item(ui, "Close tabs to the right", i + 1 < count, CloseWhich::ToTheRight(i));
        ui.separator();
        item(ui, "Close all", true, CloseWhich::All);
    });
    rect
}

fn console_ui(ui: &mut Ui, tab_id: TabId, c: &mut Console, catalog: &Catalog) -> Option<TabAction> {
    let mut action = None;
    let editor_id = Id::new(("editor", tab_id));
    let keys = completion::handle_keys(ui, &mut c.popup);
    if keys.dismissed {
        ui.memory_mut(|m| m.request_focus(editor_id));
    }
    if let Some(i) = keys.accept {
        if let Some(p) = c.popup.take() {
            c.cursor = completion::accept(ui.ctx(), editor_id, &mut c.sql, &p.completion, i);
        }
    }
    // Slash commands: Tab expands the command on the cursor's line; ⌘↩
    // expands and runs it.
    let tab = c.popup.is_none() && ui.input(|i| i.key_pressed(Key::Tab) && i.modifiers.is_none());
    let run = ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers.command && !i.modifiers.shift);
    if tab || run {
        let byte = sql::char_to_byte(&c.sql, c.cursor.start);
        if let Some((range, line)) = rgrip_core::commands::command_line_at(&c.sql, byte) {
            if tab {
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));
            }
            match rgrip_core::commands::expand(line, catalog) {
                Ok(expanded) => {
                    let indent: String = line.chars().take_while(|ch| ch.is_whitespace()).collect();
                    let text = format!("{indent}{expanded}");
                    c.cursor = completion::replace_text(ui.ctx(), editor_id, &mut c.sql, range, &text);
                    c.popup = None;
                }
                Err(e) => {
                    if tab {
                        return Some(TabAction::Notify(e));
                    }
                }
            }
        }
    }
    let mut format = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F));
    if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Enter)) {
        action = Some(TabAction::Run { all: true });
    } else if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
        action = Some(TabAction::Run { all: false });
    } else if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
        action = Some(TabAction::Save);
    }

    toolbar(ui, |ui| {
        if c.running.is_some() {
            if ui.add(Button::new(RichText::new("■  Stop").color(theme::ERROR()))).clicked() {
                action = Some(TabAction::Cancel);
            }
        } else {
            if ui
                .add(Button::new(RichText::new("▶  Run").color(theme::BG())).fill(theme::ACCENT()))
                .on_hover_text("Run statement at cursor  ⌘↩")
                .clicked()
            {
                action = Some(TabAction::Run { all: false });
            }
            if ui.button("Run all").on_hover_text("⇧⌘↩").clicked() {
                action = Some(TabAction::Run { all: true });
            }
            if ghost_button(ui, "Format").on_hover_text("Format selection or all  ⇧⌘F").clicked() {
                format = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if c.saved_id.is_some() { "Save" } else { "Save as…" };
                if ghost_button(ui, label).on_hover_text("⌘S").clicked() {
                    action = Some(TabAction::Save);
                }
            });
        }
    });

    if format && c.running.is_none() {
        // The selection if there is one, else the whole editor.
        let range = if c.cursor.start != c.cursor.end {
            sql::char_to_byte(&c.sql, c.cursor.start)..sql::char_to_byte(&c.sql, c.cursor.end)
        } else {
            0..c.sql.len()
        };
        let formatted = rgrip_core::format::format_sql(&c.sql[range.clone()]);
        c.cursor = completion::replace_text(ui.ctx(), editor_id, &mut c.sql, range, &formatted);
        c.popup = None;
        ui.memory_mut(|m| m.request_focus(editor_id));
    }

    let max_h = (ui.available_height() - 120.0).max(120.0);
    egui::TopBottomPanel::top(Id::new(("editor-panel", tab_id)))
        .resizable(true)
        .default_height(240.0)
        .height_range(80.0..=max_h)
        .frame(Frame::new().fill(theme::SURFACE()).inner_margin(Margin { left: 0, right: 14, top: 10, bottom: 8 }))
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("editor-scroll", tab_id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                        let mut job = highlight::sql(buf.as_str(), TextStyle::Monospace.resolve(ui.style()));
                        job.wrap.max_width = wrap;
                        ui.fonts_mut(|f| f.layout_job(job))
                    };
                    let out = TextEdit::multiline(&mut c.sql)
                        .id(editor_id)
                        .code_editor()
                        .frame(false)
                        .lock_focus(true)
                        .desired_width(f32::INFINITY)
                        .desired_rows(12)
                        .hint_text(hint("select * from …"))
                        .margin(Margin { left: 48, right: 0, top: 0, bottom: 0 })
                        .layouter(&mut layouter)
                        .show(ui);
                    paint_line_numbers(ui, &out);
                    let previous = c.cursor.clone();
                    if let Some(r) = out.cursor_range {
                        c.cursor = r.as_sorted_char_range();
                    }
                    update_popup(c, catalog, out.response.changed(), keys.force, previous);
                    if let Some(p) = &mut c.popup {
                        let focused = out.response.has_focus();
                        match completion::show(ui, Id::new(("completion", tab_id)), p, &out, &c.sql, focused) {
                            completion::Outcome::Accept(i) => {
                                let p = c.popup.take().unwrap();
                                c.cursor = completion::accept(ui.ctx(), editor_id, &mut c.sql, &p.completion, i);
                                ui.memory_mut(|m| m.request_focus(editor_id));
                            }
                            completion::Outcome::Close => c.popup = None,
                            completion::Outcome::None => {}
                        }
                    }
                    if c.focus {
                        out.response.request_focus();
                        c.focus = false;
                    }
                });
        });

    let Console { running, output, view, grid, paged, offset, map, last_sql, .. } = c;
    if let Some(started) = running {
        centered(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0));
                ui.label(
                    RichText::new(format!("Running…  {:.1}s", started.elapsed().as_secs_f32()))
                        .color(theme::MUTED()),
                );
            });
        });
        ui.ctx().request_repaint_after(Duration::from_millis(100));
        return action;
    }

    match output {
        None => centered(ui, |ui| muted(ui, "Run a query to see results here")),
        Some(Err(e)) => {
            Frame::new().inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.label(RichText::new(e.as_str()).monospace().color(theme::ERROR()));
                });
            });
        }
        Some(Ok(out)) => {
            let sets: Vec<&ResultSet> = out.statements.iter().filter_map(|s| s.result.as_ref()).collect();
            toolbar(ui, |ui| {
                for (n, rs) in sets.iter().enumerate() {
                    if segment(ui, *view == View::Result(n), &format!("Result {}", n + 1)) {
                        *view = View::Result(n);
                        *grid = GridState::default();
                    }
                    if !rs.geometry_columns().is_empty()
                        && segment(ui, *view == View::Map(n), if sets.len() > 1 { "Map ↑" } else { "Map" })
                    {
                        *view = View::Map(n);
                    }
                }
                if segment(ui, *view == View::Output, "Output") {
                    *view = View::Output;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ms = out.elapsed.as_secs_f64() * 1000.0;
                    let result = match *view {
                        View::Result(n) | View::Map(n) => sets.get(n).copied(),
                        View::Output => None,
                    };
                    if let Some(rs) = result {
                        if let Some(a) = export_menu(ui, rs, "query_result", paged.is_some()) {
                            action = Some(a);
                        }
                        ui.add_space(6.0);
                    }
                    let summary = match result {
                        Some(rs) if paged.is_some() => {
                            // Paged: `truncated` means another page exists.
                            if ui.add_enabled(rs.truncated, Button::new("›")).on_hover_text("Next 1,000").clicked() {
                                action = Some(TabAction::Page(true));
                            }
                            if ui.add_enabled(*offset > 0, Button::new("‹")).on_hover_text("Previous 1,000").clicked() {
                                action = Some(TabAction::Page(false));
                            }
                            let to = *offset + rs.rows.len();
                            let from = if rs.rows.is_empty() { 0 } else { *offset + 1 };
                            let more = if rs.truncated { " +" } else { "" };
                            format!("rows {from}–{to}{more}  ·  {ms:.0} ms")
                        }
                        Some(rs) => {
                            let more = if rs.truncated { " (first 1,000 shown)" } else { "" };
                            format!("{}{more}  ·  {ms:.0} ms", plural(rs.rows.len(), "row"))
                        }
                        None => format!("{}  ·  {ms:.0} ms", plural(out.statements.len(), "statement")),
                    };
                    ui.label(RichText::new(summary).small().color(theme::MUTED()));
                });
            });
            match *view {
                View::Result(n) if n < sets.len() => {
                    let row_offset = if paged.is_some() { *offset } else { 0 };
                    let source = grid::Source { table: "query_result", sql: Some(last_sql.as_str()) };
                    grid::show(ui, Id::new(("grid", tab_id, n)), sets[n], grid, None, row_offset, &source);
                }
                View::Map(n) if n < sets.len() => {
                    let cols = sets[n].geometry_columns().to_vec();
                    if cols.is_empty() {
                        centered(ui, |ui| muted(ui, "No geometry columns in this result"));
                    } else {
                        if let Some(col) = map.show(ui, sets[n], &cols, paged.is_some()) {
                            action = Some(TabAction::Reproject(col));
                        }
                    }
                }
                _ => output_log(ui, out),
            }
        }
    }
    action
}

/// Open, refresh or close the completion popup after the editor ran.
fn update_popup(c: &mut Console, catalog: &Catalog, changed: bool, force: bool, previous: std::ops::Range<usize>) {
    if c.cursor.start != c.cursor.end {
        c.popup = None;
        return;
    }
    let byte = sql::char_to_byte(&c.sql, c.cursor.start);
    let before = c.sql[..byte].chars().next_back();
    let in_word = before.is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '.' | '"' | '/'));
    if force || (changed && in_word) {
        c.popup = complete::complete(&c.sql, byte, catalog, force).map(Popup::new);
    } else if changed || previous != c.cursor {
        // Typed a separator, or moved the cursor away.
        c.popup = None;
    }
}

/// Export menu for a result grid: copy or save the rows shown, or (when
/// `can_export_all`) write every row of the underlying query to a file.
fn export_menu(ui: &mut Ui, rs: &ResultSet, table: &str, can_export_all: bool) -> Option<TabAction> {
    let has_geometry = !rs.geometry_columns().is_empty();
    let formats: Vec<export::Format> = export::Format::ALL
        .into_iter()
        .filter(|f| *f != export::Format::GeoJson || has_geometry)
        .collect();
    let file_name = table.rsplit('.').next().unwrap_or("export").to_string();
    let save_path = |f: export::Format| {
        rfd::FileDialog::new()
            .set_file_name(format!("{file_name}.{}", f.extension()))
            .add_filter(f.label(), &[f.extension()])
            .save_file()
    };
    let mut action = None;
    ui.menu_button(RichText::new("Export").color(theme::MUTED()), |ui| {
        ui.set_min_width(190.0);
        ui.menu_button("Copy as", |ui| {
            for f in &formats {
                if ui.button(f.label()).clicked() {
                    ui.ctx().copy_text(export::export(rs, *f, table));
                    ui.close();
                }
            }
        });
        ui.menu_button(format!("Save {} as", plural(rs.rows.len(), "row")), |ui| {
            for f in &formats {
                if ui.button(format!("{}…", f.label())).clicked() {
                    ui.close();
                    if let Some(path) = save_path(*f) {
                        let _ = std::fs::write(&path, export::export(rs, *f, table));
                    }
                }
            }
        });
        if can_export_all {
            ui.menu_button("Export all rows as", |ui| {
                for f in &formats {
                    if ui.button(format!("{}…", f.label())).clicked() {
                        ui.close();
                        if let Some(path) = save_path(*f) {
                            action = Some(TabAction::ExportAll(*f, path));
                        }
                    }
                }
            });
        }
    });
    action
}

fn paint_line_numbers(ui: &Ui, out: &egui::text_edit::TextEditOutput) {
    let cursor_line = out.cursor_range.map(|r| {
        out.galley.text()[..sql::char_to_byte(out.galley.text(), r.primary.index)].matches('\n').count() + 1
    });
    let font = FontId::monospace(11.5);
    let mut line = 1;
    let mut new_line = true;
    for row in &out.galley.rows {
        if new_line {
            let y = out.galley_pos.y + row.rect().center().y;
            let color = if cursor_line == Some(line) { theme::MUTED() } else { theme::FAINT() };
            ui.painter().text(
                egui::pos2(out.galley_pos.x - 16.0, y),
                egui::Align2::RIGHT_CENTER,
                line.to_string(),
                font.clone(),
                color,
            );
            line += 1;
        }
        new_line = row.row.ends_with_newline;
    }
}

fn output_log(ui: &mut Ui, out: &QueryOutput) {
    Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, s) in out.statements.iter().enumerate() {
                let msg = match &s.result {
                    Some(rs) => format!("{} retrieved", plural(rs.rows.len(), "row")),
                    None if s.affected > 0 => format!("{} affected", plural(s.affected as usize, "row")),
                    None => "completed".to_string(),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("#{}", i + 1)).monospace().color(theme::FAINT()));
                    ui.label(RichText::new(msg).monospace().color(theme::OK()));
                });
            }
            ui.label(
                RichText::new(format!("finished in {:.0} ms", out.elapsed.as_secs_f64() * 1000.0))
                    .monospace()
                    .color(theme::MUTED()),
            );
        });
    });
}

fn table_ui(ui: &mut Ui, tab_id: TabId, t: &mut TableView) -> Option<TabAction> {
    let mut action = None;
    toolbar(ui, |ui| {
        ui.label(RichText::new(format!("{}.{}", t.schema, t.table)).strong());
        ui.add_space(8.0);
        let filter = ui.add(
            TextEdit::singleline(&mut t.filter)
                .hint_text(hint("WHERE …   (enter to apply)"))
                .font(TextStyle::Monospace)
                .desired_width(320.0)
                .margin(Margin::symmetric(8, 4)),
        );
        if filter.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            t.applied_filter = t.filter.clone();
            t.page = 0;
            action = Some(TabAction::Reload);
        }
        if ghost_button(ui, "↻").on_hover_text("Reload").clicked() {
            action = Some(TabAction::Reload);
        }
        if let Some(Ok(rs)) = &t.data {
            if !rs.geometry_columns().is_empty() {
                ui.add_space(6.0);
                if segment(ui, !t.show_map, "Grid") {
                    t.show_map = false;
                }
                if segment(ui, t.show_map, "Map") {
                    t.show_map = true;
                }
            }
        }
        if t.loading {
            ui.add(egui::Spinner::new().size(12.0));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(Ok(rs)) = &t.data {
                if let Some(a) = export_menu(ui, rs, &format!("{}.{}", t.schema, t.table), true) {
                    action = Some(a);
                }
                ui.add_space(6.0);
            }
            let rows = match &t.data {
                Some(Ok(rs)) => rs.rows.len() as i64,
                _ => 0,
            };
            let last_page = match t.total {
                Some(total) => (t.page + 1) * PAGE_SIZE >= total,
                None => rows < PAGE_SIZE,
            };
            if ui.add_enabled(!last_page && !t.loading, Button::new("›")).clicked() {
                t.page += 1;
                action = Some(TabAction::Reload);
            }
            if ui.add_enabled(t.page > 0 && !t.loading, Button::new("‹")).clicked() {
                t.page -= 1;
                action = Some(TabAction::Reload);
            }
            let from = if rows == 0 { 0 } else { t.page * PAGE_SIZE + 1 };
            let range = format!("{from}–{}", t.page * PAGE_SIZE + rows);
            let text = match t.total {
                Some(total) => format!("{range} of {total}"),
                None => range,
            };
            ui.label(RichText::new(text).small().color(theme::MUTED()));
        });
    });

    match &t.data {
        None => centered(ui, |ui| muted(ui, "loading…")),
        Some(Err(e)) => {
            Frame::new().inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
                ui.label(RichText::new(e.as_str()).monospace().color(theme::ERROR()));
            });
        }
        Some(Ok(rs)) if t.show_map && !rs.geometry_columns().is_empty() => {
            let cols = rs.geometry_columns().to_vec();
            t.map.show(ui, rs, &cols, false);
        }
        Some(Ok(rs)) => {
            let (table_name, table_sql) = (format!("{}.{}", t.schema, t.table), rgrip_core::postgres::table_select_sql(&t.query()));
            let sort = t.order.as_ref().map(|(c, asc)| Sort { column: c, ascending: *asc });
            let offset = (t.page * PAGE_SIZE) as usize;
            if let Some(GridEvent::HeaderClicked(i)) =
                grid::show(
                    ui,
                    Id::new(("table-grid", tab_id)),
                    rs,
                    &mut t.grid,
                    sort,
                    offset,
                    &grid::Source { table: &table_name, sql: Some(&table_sql) },
                )
            {
                let col = rs.columns[i].clone();
                // Cycle: ascending → descending → unsorted.
                t.order = match t.order.take() {
                    Some((c, true)) if c == col => Some((c, false)),
                    Some((c, false)) if c == col => None,
                    _ => Some((col, true)),
                };
                t.page = 0;
                action = Some(TabAction::Reload);
            }
        }
    }
    action
}

// ---- saved query name prompt -------------------------------------------

/// `query` wrapped to add `col` reprojected to WGS84 as `alias`.
fn reproject_sql(query: &str, col: &str, alias: &str) -> String {
    let id = |s: &str| if complete::needs_quotes(s) { sql::quote_ident(s) } else { s.to_string() };
    let body = query.trim().trim_end_matches(';').trim_end();
    format!("select q.*, ST_Transform(q.{}, 4326) as {}\nfrom (\n{body}\n) q;", id(col), id(alias))
}

/// First meaningful line of SQL, shortened, as a default query name.
fn default_query_name(sql: &str, n: usize) -> String {
    let line = sql
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("--"))
        .unwrap_or_default();
    let line: String = line.chars().take(40).collect();
    if line.is_empty() { format!("Query {n}") } else { line }
}

impl App {
    fn name_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &mut self.name_prompt else { return };
        let title = match prompt.target {
            NameTarget::NewFrom(_) => "Save query",
            NameTarget::Rename(_) => "Rename query",
        };
        let mut done = false;
        let mut cancel = false;
        let modal = egui::Modal::new(Id::new("name-prompt"))
            .frame(
                Frame::new()
                    .fill(theme::SURFACE())
                    .stroke(Stroke::new(1.0, theme::BORDER()))
                    .corner_radius(12)
                    .inner_margin(Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(RichText::new(title).heading().color(theme::TEXT()));
                ui.add_space(12.0);
                let edit = ui.add(
                    TextEdit::singleline(&mut prompt.name)
                        .hint_text(hint("Name"))
                        .desired_width(340.0 - 16.0)
                        .margin(Margin::symmetric(8, 6)),
                );
                if !edit.has_focus() && !edit.lost_focus() {
                    edit.request_focus();
                }
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    done = true;
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(Button::new(RichText::new("Save").color(theme::BG())).fill(theme::ACCENT())).clicked() {
                            done = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            });

        let name = prompt.name.trim().to_string();
        if done && !name.is_empty() {
            let Some(prompt) = self.name_prompt.take() else { return };
            let result = match prompt.target {
                NameTarget::NewFrom(tab_id) => {
                    let tab = self.tabs.iter_mut().find(|t| t.id == tab_id);
                    match tab.and_then(|t| match &mut t.kind {
                        TabKind::Console(c) => Some((&mut t.title, c, t.conn.clone())),
                        TabKind::Table(_) => None,
                    }) {
                        Some((title, c, conn)) => config::create_query(&name, &c.sql, Some(conn)).map(|q| {
                            c.saved_id = Some(q.id.clone());
                            *title = q.name.clone();
                            q.name
                        }),
                        None => Err("The console was closed".into()),
                    }
                }
                NameTarget::Rename(id) => match self.saved.iter().find(|q| q.id == id).cloned() {
                    Some(q) => config::rename_query(&q, &name).map(|r| {
                        self.relink(&id, Some((&r.id, &r.name)));
                        r.name
                    }),
                    None => Err("The query no longer exists".into()),
                },
            };
            match result {
                Ok(saved_as) => self.toast = Some((format!("Saved “{saved_as}”"), theme::OK(), Instant::now())),
                Err(e) => self.toast = Some((e, theme::ERROR(), Instant::now())),
            }
            self.reload_queries();
        } else if cancel || modal.should_close() {
            self.name_prompt = None;
        }
    }
}

// ---- connection dialog ------------------------------------------------

impl App {
    fn dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.dialog else { return };
        let mut test = false;
        let mut save = false;
        let mut cancel = false;

        let modal = egui::Modal::new(Id::new("conn-dialog"))
            .frame(
                Frame::new()
                    .fill(theme::SURFACE())
                    .stroke(Stroke::new(1.0, theme::BORDER()))
                    .corner_radius(12)
                    .inner_margin(Margin::same(22)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(DIALOG_WIDTH);
                ui.set_max_width(DIALOG_WIDTH);
                ui.label(
                    RichText::new(if d.is_new { "New connection" } else { "Edit connection" })
                        .heading()
                        .color(theme::TEXT()),
                );
                ui.label(RichText::new("PostgreSQL").small().color(theme::MUTED()));
                ui.add_space(14.0);

                ui.horizontal(|ui| {
                    for (tab, label) in [(DialogTab::General, "General"), (DialogTab::Ssl, "SSL"), (DialogTab::Ssh, "SSH")] {
                        let mut text = label.to_string();
                        if tab == DialogTab::Ssh && d.cfg.ssh.enabled {
                            text += " ●";
                        }
                        if segment(ui, d.tab == tab, &text) {
                            d.tab = tab;
                        }
                    }
                });
                ui.add_space(10.0);
                // Fixed-size body (fits the tallest tab) so the modal doesn't
                // jump around when switching tabs or when a test result shows.
                let body = egui::vec2(DIALOG_WIDTH, 350.0);
                ui.allocate_ui_with_layout(body, Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(body);
                    match d.tab {
                        DialogTab::General => general_form(ui, d),
                        DialogTab::Ssl => ssl_form(ui, d),
                        DialogTab::Ssh => ssh_form(ui, d),
                    }
                });
                let status = egui::vec2(DIALOG_WIDTH, 44.0);
                ui.allocate_ui_with_layout(status, Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(status);
                    status_line(ui, d);
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!d.testing, Button::new("Test connection")).clicked() {
                        test = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(Button::new(RichText::new("Save").color(theme::BG())).fill(theme::ACCENT()))
                            .clicked()
                        {
                            save = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            });

        if test || save {
            match (d.port.trim().parse::<u16>(), d.ssh_port.trim().parse::<u16>()) {
                (Ok(p), Ok(sp)) => {
                    d.cfg.port = p;
                    d.cfg.ssh.port = sp;
                }
                _ => {
                    d.test = Some(Err("Ports must be numbers between 1 and 65535".into()));
                    return;
                }
            }
        }
        if test {
            d.testing = true;
            d.test = None;
            self.worker.send(Cmd::Test(d.cfg.clone()));
        }
        if save {
            let cfg = d.cfg.clone();
            self.dialog = None;
            match self.conn_index(&cfg.id) {
                Some(ci) => {
                    let was_connected = !matches!(self.conns[ci].status, Status::Disconnected);
                    self.conns[ci].cfg = cfg;
                    if was_connected {
                        self.apply(Action::Disconnect(ci));
                        self.apply(Action::Connect(ci));
                    }
                }
                None => self.conns.push(Conn::new(cfg)),
            }
            self.save_config();
        } else if cancel || modal.should_close() {
            self.dialog = None;
        }
    }
}

const DIALOG_WIDTH: f32 = 400.0;

/// Test result under the form; long errors wrap within the reserved space.
fn status_line(ui: &mut Ui, d: &Dialog) {
    match &d.test {
        _ if d.testing => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                muted(ui, "testing…");
            });
        }
        Some(Ok(v)) => {
            ui.label(RichText::new(format!("✓  Connected · {v}")).color(theme::OK()));
        }
        Some(Err(e)) => {
            egui::ScrollArea::vertical().max_height(44.0).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(e).small().color(theme::ERROR())).wrap());
            });
        }
        None => {}
    }
}

const LABEL_WIDTH: f32 = 96.0;
const FIELD_WIDTH: f32 = DIALOG_WIDTH - LABEL_WIDTH - 14.0;
/// Horizontal text-field padding; `desired_width` excludes it.
const FIELD_PAD: i8 = 8;

fn form_grid(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([14.0, 9.0])
        .min_col_width(LABEL_WIDTH)
        .max_col_width(FIELD_WIDTH)
        .show(ui, add);
}

fn field(ui: &mut Ui, label: &str, edit: TextEdit<'_>) {
    ui.label(RichText::new(label).color(theme::MUTED()));
    let inner = FIELD_WIDTH - 2.0 * FIELD_PAD as f32;
    ui.add(edit.desired_width(inner).margin(Margin::symmetric(FIELD_PAD, 5)));
    ui.end_row();
}

/// Path field with a "…" button that opens a native file picker.
fn file_field(ui: &mut Ui, label: &str, path: &mut String, placeholder: &str) {
    ui.label(RichText::new(label).color(theme::MUTED()));
    ui.horizontal(|ui| {
        let button_w = 28.0;
        let w = FIELD_WIDTH - button_w - ui.spacing().item_spacing.x - 2.0 * FIELD_PAD as f32;
        ui.add(
            TextEdit::singleline(path)
                .hint_text(hint(placeholder))
                .desired_width(w)
                .margin(Margin::symmetric(FIELD_PAD, 5)),
        );
        if ui.add_sized([button_w, 26.0], Button::new("…")).on_hover_text("Choose file").clicked() {
            let mut dialog = rfd::FileDialog::new();
            if let Some(dir) = std::path::Path::new(path.as_str()).parent().filter(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            if let Some(file) = dialog.pick_file() {
                *path = file.display().to_string();
            }
        }
    });
    ui.end_row();
}

fn note(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(theme::FAINT())).wrap());
}

fn general_form(ui: &mut Ui, d: &mut Dialog) {
    form_grid(ui, "conn-url", |ui| {
        let edit = TextEdit::singleline(&mut d.url).hint_text(hint("postgres://user:pass@host:5432/db"));
        ui.label(RichText::new("URL").color(theme::MUTED()));
        let inner = FIELD_WIDTH - 2.0 * FIELD_PAD as f32;
        let resp = ui.add(edit.desired_width(inner).margin(Margin::symmetric(FIELD_PAD, 5)));
        ui.end_row();
        if resp.changed() {
            d.url_status = (!d.url.trim().is_empty()).then(|| {
                let mut cfg = d.cfg.clone();
                rgrip_core::conn_string::apply(&mut cfg, &d.url).map(|()| {
                    d.port = cfg.port.to_string();
                    d.cfg = cfg;
                })
            });
        }
    });
    match &d.url_status {
        Some(Ok(())) => note(ui, "Filled in from the connection string."),
        Some(Err(e)) => {
            ui.label(RichText::new(e).size(11.5).color(theme::ERROR()));
        }
        None => note(ui, "Paste a postgres:// URL, key=value string or jdbc:postgresql:// URL."),
    }
    ui.add_space(6.0);
    form_grid(ui, "conn-general", |ui| {
        let placeholder = format!("{}@{}", d.cfg.database, d.cfg.host);
        field(ui, "Name", TextEdit::singleline(&mut d.cfg.name).hint_text(hint(&placeholder)));
        field(ui, "Host", TextEdit::singleline(&mut d.cfg.host));
        field(ui, "Port", TextEdit::singleline(&mut d.port));
        field(ui, "Database", TextEdit::singleline(&mut d.cfg.database));
        field(ui, "User", TextEdit::singleline(&mut d.cfg.user));
        field(ui, "Password", TextEdit::singleline(&mut d.cfg.password).password(true));
    });
    if d.cfg.ssh.enabled {
        ui.add_space(6.0);
        note(ui, "Host and port are as seen from the SSH server.");
    }
}

fn ssl_form(ui: &mut Ui, d: &mut Dialog) {
    form_grid(ui, "conn-ssl", |ui| {
        ui.label(RichText::new("Mode").color(theme::MUTED()));
        egui::ComboBox::from_id_salt("ssl-mode")
            .selected_text(d.cfg.ssl.label())
            .width(160.0)
            .show_ui(ui, |ui| {
                for m in SslMode::ALL {
                    ui.selectable_value(&mut d.cfg.ssl, m, m.label());
                }
            });
        ui.end_row();
        file_field(ui, "CA certificate", &mut d.cfg.ssl_root_cert, "system roots");
        file_field(ui, "Client cert", &mut d.cfg.ssl_client_cert, "optional (.crt / .pem)");
        file_field(ui, "Client key", &mut d.cfg.ssl_client_key, "optional (.key / .pem)");
    });
    ui.add_space(8.0);
    note(
        ui,
        match d.cfg.ssl {
            SslMode::Disable => "No encryption.",
            SslMode::Prefer => "Encrypt if the server supports it; the certificate is not checked.",
            SslMode::Require => "Always encrypt; the certificate is not checked.",
            SslMode::VerifyCa => "Encrypt and check the certificate is signed by the CA (host name not checked).",
            SslMode::VerifyFull => "Encrypt, check the CA and that the certificate matches the host name.",
        },
    );
}

fn ssh_form(ui: &mut Ui, d: &mut Dialog) {
    let ssh = &mut d.cfg.ssh;
    ui.checkbox(&mut ssh.enabled, "Connect through an SSH tunnel");
    ui.add_space(8.0);
    ui.add_enabled_ui(ssh.enabled, |ui| {
        form_grid(ui, "conn-ssh", |ui| {
            field(ui, "SSH host", TextEdit::singleline(&mut ssh.host).hint_text(hint("bastion.example.com")));
            field(ui, "Port", TextEdit::singleline(&mut d.ssh_port));
            field(ui, "User", TextEdit::singleline(&mut ssh.user));
            ui.label(RichText::new("Auth").color(theme::MUTED()));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut ssh.auth, SshAuth::Key, "Key file");
                ui.selectable_value(&mut ssh.auth, SshAuth::Password, "Password");
            });
            ui.end_row();
            match ssh.auth {
                SshAuth::Password => {
                    field(ui, "Password", TextEdit::singleline(&mut ssh.password).password(true));
                }
                SshAuth::Key => {
                    file_field(ui, "Private key", &mut ssh.key_path, "~/.ssh/id_ed25519");
                    field(
                        ui,
                        "Passphrase",
                        TextEdit::singleline(&mut ssh.passphrase).password(true).hint_text(hint("if the key has one")),
                    );
                }
            }
        });
        ui.add_space(8.0);
        note(ui, "Host keys are checked against ~/.ssh/known_hosts; a changed key is refused.");
    });
}

// ---- small widgets ----------------------------------------------------

fn empty_state(ui: &mut Ui, actions: &mut Vec<Action>, no_connections: bool) {
    centered(ui, |ui| {
        ui.label(RichText::new("rgrip").size(28.0).color(theme::FAINT()));
        ui.add_space(6.0);
        if no_connections {
            muted(ui, "Add a PostgreSQL connection to get started.");
            ui.add_space(10.0);
            if ui
                .add(Button::new(RichText::new("New connection").color(theme::BG())).fill(theme::ACCENT()))
                .clicked()
            {
                actions.push(Action::NewConnection);
            }
        } else {
            muted(ui, "Open a connection on the left to start a console.");
            muted(ui, "Double-click a table to browse its data.");
        }
    });
}

fn toolbar(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .inner_margin(Margin::symmetric(12, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(24.0);
                add(ui);
            });
        });
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, theme::BORDER()));
}

fn segment(ui: &mut Ui, selected: bool, text: &str) -> bool {
    let color = if selected { theme::TEXT() } else { theme::MUTED() };
    let r = ui.add(
        Button::new(RichText::new(text).color(color))
            .fill(if selected { theme::SELECTED() } else { Color32::TRANSPARENT })
            .frame_when_inactive(selected),
    );
    r.clicked()
}

/// Native-feeling title strip for the full-size-content macOS window.
fn title_bar(ctx: &egui::Context) {
    egui::TopBottomPanel::top("titlebar")
        .exact_height(30.0)
        .frame(Frame::new().fill(theme::BG()))
        .show(ctx, |ui| {
            let resp = ui.interact(ui.max_rect(), Id::new("titlebar-drag"), Sense::click_and_drag());
            if resp.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if resp.double_clicked() {
                let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
            // Painted, not a Label: a (selectable) label would take the
            // double-click and select its text instead of zooming the window.
            ui.painter().text(
                ui.max_rect().center(),
                egui::Align2::CENTER_CENTER,
                "rgrip",
                FontId::proportional(12.0),
                theme::FAINT(),
            );
        });
}

/// Small borderless button that brightens on hover.
fn icon_button(ui: &mut Ui, text: &str) -> egui::Response {
    let resp = ui.add(
        Button::new(RichText::new(text).size(14.0).color(theme::MUTED()))
            .frame(false)
            .min_size(egui::vec2(18.0, 18.0)),
    );
    if resp.hovered() {
        ui.painter().rect_filled(resp.rect, 4.0, theme::HOVER());
        ui.painter().text(resp.rect.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(14.0), theme::TEXT());
    }
    resp
}

fn ghost_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(Button::new(RichText::new(text).color(theme::MUTED())).frame(false))
}

fn centered(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.with_layout(Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 60.0).max(0.0));
            add(ui);
        });
    });
}

fn hint(text: &str) -> RichText {
    RichText::new(text).color(theme::FAINT())
}

pub(crate) fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

fn muted(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::MUTED()));
}


#[cfg(test)]
mod tests {
    //! Drives the real console UI headlessly with keyboard events.
    use super::*;
    use rgrip_core::complete::CatalogTable;

    struct Harness {
        ctx: egui::Context,
        console: Console,
        catalog: Catalog,
    }

    impl Harness {
        fn new() -> Self {
            let table = |name: &str, cols: &[&str]| CatalogTable {
                schema: "public".into(),
                name: name.into(),
                kind: TableKind::Table,
                columns: cols.iter().map(|c| (c.to_string(), "text".into())).collect(),
            };
            let mut h = Self {
                ctx: egui::Context::default(),
                console: Console::new(),
                catalog: Catalog {
                    tables: vec![table("users", &["id", "email"]), table("orders", &["id", "total"])],
                },
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events,
                ..Default::default()
            };
            let (console, catalog) = (&mut self.console, &self.catalog);
            let _ = self.ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    console_ui(ui, 1, console, catalog);
                });
            });
        }

        /// One frame per character, like real typing.
        fn type_text(&mut self, text: &str) {
            for ch in text.chars() {
                self.frame(vec![egui::Event::Text(ch.to_string())]);
            }
        }

        fn key(&mut self, key: Key, modifiers: Modifiers) {
            self.frame(vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
        }

        fn labels(&self) -> Vec<String> {
            self.console
                .popup
                .as_ref()
                .map(|p| p.completion.items.iter().map(|i| i.label.clone()).collect())
                .unwrap_or_default()
        }
    }

    fn tab(id: TabId) -> Tab {
        Tab { id, conn: "c".into(), title: format!("tab {id}"), kind: TabKind::Console(Console::new()) }
    }

    /// Run the tab bar for one frame with `events`; returns tab rects and actions.
    fn tab_frame(ctx: &egui::Context, tabs: &[Tab], events: Vec<egui::Event>) -> (Vec<egui::Rect>, Vec<Action>) {
        let mut rects = Vec::new();
        let mut actions = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 600.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (i, t) in tabs.iter().enumerate() {
                        rects.push(tab_button(ui, i, t, i == 0, false, tabs.len(), &mut actions));
                    }
                });
            });
        });
        (rects, actions)
    }

    fn click(ctx: &egui::Context, tabs: &[Tab], pos: egui::Pos2, button: egui::PointerButton) -> Vec<Action> {
        let press = |pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE };
        tab_frame(ctx, tabs, vec![egui::Event::PointerMoved(pos)]);
        let mut actions = tab_frame(ctx, tabs, vec![press(true)]).1;
        actions.extend(tab_frame(ctx, tabs, vec![press(false)]).1);
        actions.extend(tab_frame(ctx, tabs, vec![]).1);
        actions
    }

    fn closes(actions: &[Action]) -> Vec<CloseWhich> {
        actions.iter().filter_map(|a| if let Action::Close(w) = a { Some(*w) } else { None }).collect()
    }

    #[test]
    fn tab_close_button_closes_that_tab() {
        let ctx = egui::Context::default();
        let tabs = [tab(1), tab(2)];
        let (rects, _) = tab_frame(&ctx, &tabs, vec![]);
        tab_frame(&ctx, &tabs, vec![]);

        // The × sits at the right end of each tab.
        let close_of = |r: egui::Rect| egui::pos2(r.right() - 6.0 - 9.0, r.center().y);
        assert_eq!(closes(&click(&ctx, &tabs, close_of(rects[0]), egui::PointerButton::Primary)), [CloseWhich::One(0)]);
        assert_eq!(closes(&click(&ctx, &tabs, close_of(rects[1]), egui::PointerButton::Primary)), [CloseWhich::One(1)]);

        // Clicking the tab body activates it instead; middle click closes.
        let body = egui::pos2(rects[1].left() + 30.0, rects[1].center().y);
        let actions = click(&ctx, &tabs, body, egui::PointerButton::Primary);
        assert!(closes(&actions).is_empty());
        assert!(actions.iter().any(|a| matches!(a, Action::ActivateTab(1))));
        assert_eq!(closes(&click(&ctx, &tabs, body, egui::PointerButton::Middle)), [CloseWhich::One(1)]);
    }

    #[test]
    fn close_variants() {
        let run = |which, active: usize| {
            let mut tabs = vec![0, 1, 2, 3];
            let mut active = active;
            close_tabs(&mut tabs, &mut active, which);
            (tabs, active)
        };
        assert_eq!(run(CloseWhich::One(1), 3), (vec![0, 2, 3], 2));
        assert_eq!(run(CloseWhich::One(3), 3), (vec![0, 1, 2], 2));
        assert_eq!(run(CloseWhich::Others(2), 0), (vec![2], 0));
        assert_eq!(run(CloseWhich::ToTheRight(1), 3), (vec![0, 1], 1));
        assert_eq!(run(CloseWhich::ToTheRight(1), 0), (vec![0, 1], 0));
        assert_eq!(run(CloseWhich::All, 2), (vec![], 0));
        assert_eq!(run(CloseWhich::One(9), 1), (vec![0, 1, 2, 3], 1));
    }

    #[test]
    fn reprojection_query() {
        assert_eq!(
            reproject_sql("select * from parcels where id < 10;\n", "geom", "geom_4326"),
            "select q.*, ST_Transform(q.geom, 4326) as geom_4326\nfrom (\nselect * from parcels where id < 10\n) q;"
        );
        assert!(reproject_sql("select 1", "Geom Col", "Geom Col_4326").contains("q.\"Geom Col\""));
    }

    #[test]
    fn query_names() {
        assert_eq!(default_query_name("-- note\n\n  select * from users where id = 1 and name like 'x%'", 1), "select * from users where id = 1 and nam");
        assert_eq!(default_query_name("  \n-- only a comment", 3), "Query 3");
    }

    #[test]
    fn typing_shows_and_accepts_suggestions() {
        let mut h = Harness::new();
        h.type_text("select * from us");
        assert_eq!(h.labels().first().map(String::as_str), Some("users"));

        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.console.sql, "select * from users");
        assert!(h.console.popup.is_none());

        // Typing continues at the new cursor; Tab accepts too.
        h.type_text(" u where u.em");
        assert_eq!(h.labels(), ["email"]);
        h.key(Key::Tab, Modifiers::NONE);
        assert_eq!(h.console.sql, "select * from users u where u.email");

        h.type_text(" = 'x'");
        assert_eq!(h.console.sql, "select * from users u where u.email = 'x'");
    }

    #[test]
    fn slash_command_tab_expands() {
        let mut h = Harness::new();
        h.type_text("/con_4326 geom");
        h.key(Key::Tab, Modifiers::NONE);
        assert_eq!(h.console.sql, "select t.*, ST_Transform(t.geom, 4326) as geom_4326\nfrom table_name t;");
        h.console.sql.clear();
        h.frame(vec![]);
        h.type_text("/con_4326_2 users");
        h.key(Key::Tab, Modifiers::NONE);
        assert_eq!(h.console.sql, "select t.*, ST_SetSRID(st_astext(t.shape)::text, 4326) as shape_4326\nfrom users t;");
        // Typing just "/" lists the commands.
        h.console.sql.clear();
        h.frame(vec![]);
        h.type_text("/");
        assert!(h.labels().iter().any(|l| l.starts_with("/con_4326")));
    }

    #[test]
    fn format_shortcut_formats_the_editor() {
        let mut h = Harness::new();
        h.type_text("select a, b from t where a = 1");
        h.key(Key::F, Modifiers::COMMAND | Modifiers::SHIFT);
        assert_eq!(h.console.sql, "select a, b\nfrom t\nwhere a = 1");
    }

    #[test]
    fn arrows_escape_and_separators() {
        let mut h = Harness::new();
        h.type_text("select o.");
        // Columns of `o` aren't known until FROM exists: nothing to show.
        assert!(h.console.popup.is_none());

        h.type_text("x from orders o");
        h.console.sql.clear();
        h.frame(vec![]);

        h.type_text("select * from ");
        assert!(h.console.popup.is_none(), "space alone shouldn't open the popup");
        h.key(Key::Space, Modifiers::CTRL);
        assert_eq!(h.labels()[..2], ["users", "orders"]);
        h.key(Key::ArrowDown, Modifiers::NONE);
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.console.sql, "select * from orders");

        h.type_text(" wh");
        assert!(h.console.popup.is_some());
        h.key(Key::Escape, Modifiers::NONE);
        assert!(h.console.popup.is_none());
        // Enter now inserts a newline as usual.
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.console.sql, "select * from orders wh\n");
    }
}

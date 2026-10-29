use super::*;

impl App {
    /// Debug builds only: `RGRIP_DEMO_SQL` opens a console on the first
    /// connection and runs it; `RGRIP_DEMO_TABLE=schema.table` opens a table.
    /// Used to exercise and screenshot the UI without clicking.
    #[cfg(debug_assertions)]
    /// Per-frame screenshot scene setup (debug builds), applied once the
    /// active tab has data:
    /// - `RGRIP_DEMO_EXPAND=demo,public`: explorer schemas to expand
    /// - `RGRIP_DEMO_FREEZE=a,b`: frozen columns
    /// - `RGRIP_DEMO_SELECT=0,2`: selected rows
    /// - `RGRIP_DEMO_TABLE_VIEW=map` (+ `RGRIP_DEMO_FEATURE=<row>`): map view
    /// - `RGRIP_DEMO_EDITS='0:phone=+855 12 345 678;1:verified=t'`: pending edits
    /// - `RGRIP_DEMO_EDITOR=2:registered_on`: open cell editor (picker)
    /// - `RGRIP_DEMO_RELATED=parcels`: related-rows popup for the selection
    /// - `RGRIP_DEMO_COPY_IMAGE=<path>`: write "Copy as image" PNG of the selection
    /// - `RGRIP_DEMO_SCROLL_X=320`: scroll the grid sideways (once)
    /// - `RGRIP_DEMO_ONLY=1`: show just the demo connection, no saved queries
    /// - `RGRIP_DEMO_READY=<path>`: touch when settled (for the capture script)
    fn demo_scene(&mut self) {
        let env = |k: &str| std::env::var(k).ok();
        if env("RGRIP_DEMO_ONLY").is_some() {
            if let Some(name) = env("RGRIP_DEMO_CONN") {
                self.conns.retain(|c| c.cfg.display_name() == name);
            }
            self.saved.clear();
        }
        let scroll_x = env("RGRIP_DEMO_SCROLL_X").and_then(|v| v.parse::<f32>().ok());
        // `RGRIP_DEMO_COLOR=red`: tint the demo connection (not saved).
        if let (Some(color), Some(c)) = (env("RGRIP_DEMO_COLOR"), self.conns.first_mut()) {
            use rgrip_core::ConnColor::*;
            c.cfg.color = match color.as_str() {
                "red" => Red,
                "orange" => Orange,
                "yellow" => Yellow,
                "green" => Green,
                "blue" => Blue,
                "purple" => Purple,
                "gray" => Gray,
                _ => None,
            };
        }
        // `RGRIP_DEMO_MORE=demo.owners,demo.parcels`: extra table tabs, then
        // back to the first tab; `RGRIP_DEMO_SCHEMA=demo` for consoles.
        static MORE_OPENED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if let Some(more) = env("RGRIP_DEMO_MORE")
            && !self.tabs.is_empty()
            && !MORE_OPENED.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            for t in more.split(',') {
                if let Some((schema, table)) = t.split_once('.') {
                    self.open_table(0, schema.into(), table.into());
                }
            }
            self.active = 0;
        }
        if let Some(schema) = env("RGRIP_DEMO_SCHEMA") {
            let mut list: Vec<String> = schema.split(',').map(String::from).collect();
            let first = list.remove(0);
            for tab in &mut self.tabs {
                if let TabKind::Console(c) = &mut tab.kind {
                    c.schema = Some(first.clone());
                    c.more_schemas = list.clone();
                }
            }
        }
        // `RGRIP_DEMO_FILTER=order`: explorer search.
        if let Some(f) = env("RGRIP_DEMO_FILTER") {
            self.filter = f;
        }
        if let Some(db) = env("RGRIP_DEMO_DATABASE") {
            for tab in &mut self.tabs {
                if let TabKind::Console(c) = &mut tab.kind {
                    c.database = Some(db.clone());
                }
            }
        }
        static ONCE: std::sync::Mutex<Vec<&str>> = std::sync::Mutex::new(Vec::new());
        let once = |what: &'static str| {
            let mut done = ONCE.lock().unwrap();
            (!done.contains(&what)).then(|| done.push(what)).is_some()
        };
        // `RGRIP_DEMO_COMMANDS=<preview args>`: the slash command editor, once.
        static COMMANDS_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if let Some(args) = env("RGRIP_DEMO_COMMANDS") && !COMMANDS_SHOWN.swap(true, std::sync::atomic::Ordering::Relaxed) {
            self.apply(Action::EditCommands);
            if let Some(ed) = &mut self.commands_editor {
                ed.selected = 2;
                ed.preview_args = args;
            }
        }
        if let (Some(list), Some(c)) = (env("RGRIP_DEMO_EXPAND"), self.conns.first()) {
            let id = c.cfg.id.clone();
            self.expanded.insert(format!("c/{id}"));
            for s in list.split(',') {
                self.expanded.insert(format!("s/{id}/{s}"));
            }
        }
        let mut follow_action = None;
        let mut sort_reload = false;
        let settled = match self.tabs.get_mut(self.active).map(|t| &mut t.kind) {
            None => true,
            Some(TabKind::Console(c)) => {
                if let (Some(cols), true) = (env("RGRIP_DEMO_FREEZE"), c.grid.frozen.is_empty()) {
                    c.grid.frozen = cols.split(',').map(String::from).collect();
                }
                (c.running.is_none() && (c.output.is_some() || c.sql.trim().is_empty())) || c.popup.is_some()
            }
            Some(TabKind::Table(t)) => match &t.data {
                Some(Ok(rs)) if !t.loading => {
                    let col = |name: &str| rs.columns.iter().position(|c| c == name);
                    let cell = |spec: &str| -> Option<(usize, usize)> {
                        let (r, c) = spec.split_once(':')?;
                        Some((r.parse().ok()?, col(c)?))
                    };
                    if let (Some(cols), true) = (env("RGRIP_DEMO_FREEZE"), t.grid.frozen.is_empty()) {
                        t.grid.frozen = cols.split(',').map(String::from).collect();
                        t.grid.scroll_x = scroll_x;
                    }
                    if let (Some(sel), true) = (env("RGRIP_DEMO_SELECT"), t.grid.selection.is_empty()) {
                        t.grid.selection = sel.split(',').filter_map(|v| v.parse().ok()).collect();
                    }
                    // `RGRIP_DEMO_SORT=price_usd:desc`, `RGRIP_DEMO_GOTO=pri`,
                    // `RGRIP_DEMO_ROW_MENU=1`.
                    if let Some(sort) = env("RGRIP_DEMO_SORT")
                        && once("sort")
                    {
                        let (col, dir) = sort.split_once(':').unwrap_or((&sort, "asc"));
                        t.order = Some((col.to_string(), dir != "desc"));
                        sort_reload = true;
                    }
                    if let Some(q) = env("RGRIP_DEMO_GOTO")
                        && !sort_reload
                        && once("goto")
                    {
                        t.grid.open_goto(&q);
                    }
                    if let Some(row) = env("RGRIP_DEMO_ROW_MENU").and_then(|v| v.parse().ok())
                        && once("row-menu")
                    {
                        if !t.grid.selection.contains(&row) {
                            t.grid.selection = [row].into();
                        }
                        t.grid.demo_menu = Some(row);
                    }
                    if let Some(v @ ("map" | "split")) = env("RGRIP_DEMO_TABLE_VIEW").as_deref() {
                        t.pane = if v == "split" { Pane::Split } else { Pane::Map };
                        t.map.selected = env("RGRIP_DEMO_FEATURE").and_then(|v| v.parse().ok());
                    }
                    if let (Some(edits), true) = (env("RGRIP_DEMO_EDITS"), t.changes.is_empty()) {
                        for e in edits.split(';') {
                            if let Some((at, v)) = e.split_once('=').and_then(|(spec, v)| Some((cell(spec)?, v))) {
                                t.changes.insert(at, Some(v.to_string()));
                            }
                        }
                    }
                    if let (Some(spec), true) = (env("RGRIP_DEMO_EDITOR"), t.editor.is_none()) {
                        if let Some((r, c)) = cell(&spec) {
                            t.editor = Some(grid::CellEditor::open(r, c, rs.rows[r][c].as_deref()));
                        }
                    }
                    if let Some(path) = env("RGRIP_DEMO_COPY_IMAGE").filter(|p| !std::path::Path::new(p).exists()) {
                        let rows: Vec<usize> = if t.grid.selection.is_empty() { (0..rs.rows.len().min(8)).collect() } else { t.grid.selection.iter().copied().collect() };
                        let sub = export::subset(rs, &rows);
                        if let Some(png) = crate::snapshot::table_png(&sub) {
                            let _ = std::fs::write(path, png);
                        }
                    }
                    match (env("RGRIP_DEMO_RELATED"), &t.related) {
                        (Some(other), None) => {
                            if let Some(r) = t.relations.iter().find(|r| r.other_table == other) {
                                let rows: Vec<usize> = if t.grid.selection.is_empty() { vec![0] } else { t.grid.selection.iter().copied().collect() };
                                follow_action = Some(follow(r, rs, &rows, &[]));
                            }
                            false
                        }
                        (Some(_), Some(rel)) => rel.data.is_some(),
                        _ => true,
                    }
                }
                _ => false,
            },
            Some(TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_)) => true,
        };
        // `RGRIP_DEMO_IMPORT=<file>`: the import dialog with that file picked,
        // into the first `RGRIP_DEMO_EXPAND` schema.
        static IMPORT_OPENED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        // `RGRIP_DEMO_LAYOUT=demo.districts,demo.parcels`: a map layout with
        // those tables as layers (`RGRIP_DEMO_LAYOUT_TITLE` sets the title).
        static LAYOUT_OPENED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if let Some(specs) = env("RGRIP_DEMO_LAYOUT") {
            if self.conns.first().is_some_and(|c| matches!(c.status, Status::Disconnected)) {
                self.apply(Action::Connect(0));
            }
            let ready = self.conns.first().is_some_and(|c| !c.catalog.tables.is_empty());
            if ready && !LAYOUT_OPENED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                for spec in specs.split(',').rev() {
                    let Some((schema, table)) = spec.split_once('.') else { continue };
                    let col = self.conns[0].catalog.tables.iter().find(|t| t.schema == schema && t.name == table).and_then(|t| {
                        t.columns.iter().find(|(_, ty)| ty.starts_with("geometry")).map(|(n, _)| n.clone())
                    });
                    if let Some(col) = col {
                        self.apply(Action::AddToLayout(0, schema.into(), table.into(), col));
                    }
                }
                if let (Some(title), Some(TabKind::Layout(l))) = (env("RGRIP_DEMO_LAYOUT_TITLE"), self.tabs.get_mut(self.active).map(|t| &mut t.kind)) {
                    l.layout.title = title;
                }
            }
            // `RGRIP_DEMO_LAYOUT_SCALE=10000`: zoom in once the layers have loaded.
            static SCALED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if let Some(scale) = env("RGRIP_DEMO_LAYOUT_SCALE").and_then(|v| v.parse::<f64>().ok())
                && let Some(TabKind::Layout(l)) = self.tabs.iter_mut().find(|t| matches!(t.kind, TabKind::Layout(_))).map(|t| &mut t.kind)
                && l.layout.layers.iter().all(|x| x.loaded.is_some())
                && !l.layout.layers.is_empty()
                && !SCALED.swap(true, std::sync::atomic::Ordering::Relaxed)
            {
                l.demo_zoom(scale);
            }
            // `RGRIP_DEMO_LAYOUT_COLOR_BY=land_use`: categories on the top layer.
            static COLORED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if let Some(col) = env("RGRIP_DEMO_LAYOUT_COLOR_BY")
                && LAYOUT_OPENED.load(std::sync::atomic::Ordering::Relaxed)
                && !COLORED.swap(true, std::sync::atomic::Ordering::Relaxed)
                && let Some(ti) = self.tabs.iter().position(|t| matches!(t.kind, TabKind::Layout(_)))
                && let TabKind::Layout(l) = &mut self.tabs[ti].kind
            {
                let loads = l.demo_color_by(&col);
                self.tab_action(ti, TabAction::LoadLayers(loads));
            }
            // `RGRIP_DEMO_LAYOUT_EXPORT=/tmp/map.pdf|png`: export once loaded.
            static EXPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if let (Some(path), Some(TabKind::Layout(l))) = (env("RGRIP_DEMO_LAYOUT_EXPORT"), self.tabs.get_mut(self.active).map(|t| &mut t.kind))
                && !EXPORTED.load(std::sync::atomic::Ordering::Relaxed)
                && l.demo_export(path.into())
            {
                EXPORTED.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        // `RGRIP_DEMO_EDIT_TABLE=demo.owners`: Edit table with a few edits.
        static EDIT_OPENED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
        if let Some((schema, table)) = env("RGRIP_DEMO_EDIT_TABLE").as_deref().and_then(|v| v.split_once('.'))
            && matches!(self.conns.first().map(|c| &c.status), Some(Status::Connected(_)))
        {
            use std::sync::atomic::Ordering::Relaxed;
            if EDIT_OPENED.load(Relaxed) == 0 {
                EDIT_OPENED.store(1, Relaxed);
                self.apply(Action::EditTable(0, schema.into(), table.into()));
            } else if EDIT_OPENED.load(Relaxed) == 1 && self.table_editor.as_mut().is_some_and(|e| e.demo_edits()) {
                EDIT_OPENED.store(2, Relaxed);
            }
        }
        if env("RGRIP_DEMO_EDIT_TABLE").is_some() && self.conns.first().is_some_and(|c| matches!(c.status, Status::Disconnected)) {
            self.apply(Action::Connect(0));
        }
        // Connects, then waits for the schema's tables, as when opened from
        // the explorer.
        if env("RGRIP_DEMO_IMPORT").is_some() && self.conns.first().is_some_and(|c| matches!(c.status, Status::Disconnected)) {
            self.apply(Action::Connect(0));
        }
        let import_schema = env("RGRIP_DEMO_EXPAND").and_then(|v| v.split(',').next().map(String::from)).unwrap_or_else(|| "public".into());
        if let Some(file) = env("RGRIP_DEMO_IMPORT")
            && self.conns.first().is_some_and(|c| matches!(c.tables.get(&import_schema), Some(Load::Ready(_))))
            && !IMPORT_OPENED.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let schema = import_schema;
            self.apply(Action::Import(0, schema, None));
            if let Some(d) = &mut self.import {
                d.demo_open(&self.egui_ctx, file.into());
            }
        }
        // A pending confirmation or the import dialog is a scene of its own
        // (no results come).
        let settled = settled
            || self.confirm.is_some()
            || self.import.as_ref().is_some_and(|d| d.demo_ready())
            || EDIT_OPENED.load(std::sync::atomic::Ordering::Relaxed) == 2;
        if let Some(a) = follow_action {
            self.tab_action(self.active, a);
        }
        if sort_reload {
            self.tab_action(self.active, TabAction::Reload);
        }
        // Touch the ready file once the scene has looked settled for a moment.
        if let Some(path) = env("RGRIP_DEMO_READY") {
            if !settled {
                self.demo_settled = None;
            } else if self.demo_settled.get_or_insert_with(Instant::now).elapsed() > Duration::from_millis(2500) {
                let _ = std::fs::write(&path, "ready");
                // Recordings: play the script once the scene is ready.
                if self.demo_play.is_none()
                    && let Some(steps) = env("RGRIP_DEMO_STEPS")
                {
                    let steps = steps.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                    self.demo_play = Some(DemoPlay { steps, index: 0, since: Instant::now(), typed: 0 });
                }
            }
            self.egui_ctx.request_repaint_after(Duration::from_millis(100));
        }
        while self.demo_step() {}
    }

    /// Advance the demo script by one step if it's ready to move on.
    /// Returns true to run the next step in the same frame.
    ///
    /// Steps: `wait:<ms>`, `type:<text>` (into the console, `\n` = newline),
    /// `keys:<text>` (keystrokes to the focused field), `key:[cmd+]Enter|Escape|/|A|Backspace`, `click:<x>,<y>`, `drag:<x1>,<y1>,<x2>,<y2>`,
    /// `run` (and wait for results), `view:map|grid`, `open:<schema.table>`
    /// (and wait for data), `select:<rows>`, `menu:<row>`,
    /// `related:<table>` (and wait), `editor:<row>:<col>`,
    /// `edit:<row>:<col>=<value>`, `freeze:<cols>`, `scrollx:<px>`,
    /// `sort:<col>:asc|desc` (and wait), `goto` (open the column search),
    /// `commands:<args>` (slash command editor preview), `done` (touch
    /// `RGRIP_DEMO_DONE`).
    #[cfg(debug_assertions)]
    fn demo_step(&mut self) -> bool {
        let Some(play) = &mut self.demo_play else { return false };
        let Some(step) = play.steps.get(play.index).cloned() else { return false };
        self.egui_ctx.request_repaint_after(Duration::from_millis(16));
        let elapsed = play.since.elapsed();
        let (verb, arg) = step.split_once(':').unwrap_or((step.as_str(), ""));
        let advance = |app: &mut App| {
            if let Some(p) = &mut app.demo_play {
                p.index += 1;
                p.since = Instant::now();
                p.typed = 0;
            }
            true
        };
        let active = self.active;
        match verb {
            "wait" => {
                if elapsed.as_millis() >= arg.parse::<u128>().unwrap_or(500) {
                    return advance(self);
                }
            }
            "type" | "keys" => {
                // About 22 characters a second, like a quick typist.
                let text = arg.replace("\\n", "\n");
                let total = text.chars().count();
                let want = ((elapsed.as_millis() / 45) as usize + 1).min(total);
                let typed = play.typed;
                if want > typed {
                    let chunk: String = text.chars().skip(typed).take(want - typed).collect();
                    play.typed = want;
                    if verb == "keys" {
                        self.egui_ctx.input_mut(|i| i.events.push(egui::Event::Text(chunk)));
                    } else if let Some(TabKind::Console(c)) = self.tabs.get_mut(active).map(|t| &mut t.kind) {
                        c.sql.push_str(&chunk);
                        let n = c.sql.chars().count();
                        c.cursor = n..n;
                        let id = Id::new(("editor", self.tabs[active].id));
                        let mut state = TextEdit::load_state(&self.egui_ctx, id).unwrap_or_default();
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(n))));
                        state.store(&self.egui_ctx, id);
                    }
                }
                if want == total && elapsed.as_millis() > 45 * total as u128 + 150 {
                    return advance(self);
                }
            }
            "key" => {
                // `cmd+` in front for ⌘.
                let (modifiers, name) = arg.strip_prefix("cmd+").map_or((Modifiers::NONE, arg), |k| (Modifiers::COMMAND, k));
                let key = match name {
                    "Enter" => Key::Enter,
                    "Escape" => Key::Escape,
                    "Down" => Key::ArrowDown,
                    "/" => Key::Slash,
                    "A" => Key::A,
                    "Backspace" => Key::Backspace,
                    _ => Key::Tab,
                };
                self.egui_ctx.input_mut(|i| i.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }));
                return advance(self);
            }
            "click" => {
                // Press this frame, release the next.
                let (x, y) = arg.split_once(',').unwrap_or(("0", "0"));
                let pos = egui::pos2(x.trim().parse().unwrap_or(0.0), y.trim().parse().unwrap_or(0.0));
                let pressed = play.typed == 0;
                play.typed = 1;
                // Pointer state is read before `update`; these go in with the next frame's input.
                self.demo_input.push(egui::Event::PointerMoved(pos));
                self.demo_input.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                self.egui_ctx.request_repaint();
                if !pressed {
                    return advance(self);
                }
            }
            "hover" => {
                // `hover:x,y`: move the pointer there (no click).
                let (x, y) = arg.split_once(',').unwrap_or(("0", "0"));
                let pos = egui::pos2(x.trim().parse().unwrap_or(0.0), y.trim().parse().unwrap_or(0.0));
                self.demo_input.push(egui::Event::PointerMoved(pos));
                self.egui_ctx.request_repaint();
                return advance(self);
            }
            "drag" => {
                // `drag:x1,y1,x2,y2`: press, move over ten frames, release.
                let v: Vec<f32> = arg.split(',').filter_map(|n| n.trim().parse().ok()).collect();
                if v.len() == 4 {
                    let step = play.typed;
                    play.typed += 1;
                    let t = (step.min(10) as f32) / 10.0;
                    let pos = egui::pos2(v[0] + (v[2] - v[0]) * t, v[1] + (v[3] - v[1]) * t);
                    self.demo_input.push(egui::Event::PointerMoved(pos));
                    if step == 0 || step == 11 {
                        self.demo_input.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: step == 0, modifiers: Modifiers::NONE });
                    }
                    self.egui_ctx.request_repaint();
                    if step < 11 {
                        return false;
                    }
                }
                return advance(self);
            }
            "run" => {
                let state = match self.tabs.get(active).map(|t| &t.kind) {
                    Some(TabKind::Console(c)) => (c.running.is_some(), c.output.is_some()),
                    _ => (false, true),
                };
                if play.typed == 0 {
                    play.typed = 1;
                    if let Some(TabKind::Console(c)) = self.tabs.get_mut(active).map(|t| &mut t.kind) {
                        c.output = None;
                    }
                    self.tab_action(active, TabAction::Run { all: true });
                } else if !state.0 && state.1 {
                    return advance(self);
                }
            }
            "view" => {
                match self.tabs.get_mut(active).map(|t| &mut t.kind) {
                    Some(TabKind::Console(c)) => {
                        c.view = match arg {
                            "map" => View::Map(0),
                            "split" => View::Split(0),
                            _ => View::Result(0),
                        }
                    }
                    Some(TabKind::Table(t)) => {
                        t.pane = match arg {
                            "map" => Pane::Map,
                            "split" => Pane::Split,
                            _ => Pane::Grid,
                        }
                    }
                    Some(TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_)) | None => {}
                }
                return advance(self);
            }
            "open" | "related" | "sort" => {
                if play.typed == 0 {
                    play.typed = 1;
                    match verb {
                        "open" => {
                            if let Some((schema, table)) = arg.split_once('.') {
                                self.open_table(0, schema.into(), table.into());
                            }
                        }
                        "sort" => {
                            if let Some(TabKind::Table(t)) = self.tabs.get_mut(active).map(|t| &mut t.kind) {
                                let (col, dir) = arg.split_once(':').unwrap_or((arg, "asc"));
                                t.order = Some((col.to_string(), dir != "desc"));
                            }
                            self.tab_action(active, TabAction::Reload);
                        }
                        _ => {
                            let a = match self.tabs.get(active).map(|t| &t.kind) {
                                Some(TabKind::Table(t)) => match (&t.data, t.relations.iter().find(|r| r.other_table == arg)) {
                                    (Some(Ok(rs)), Some(r)) => {
                                        let rows: Vec<usize> = if t.grid.selection.is_empty() { vec![0] } else { t.grid.selection.iter().copied().collect() };
                                        Some(follow(r, rs, &rows, &[]))
                                    }
                                    _ => None,
                                },
                                _ => None,
                            };
                            if let Some(a) = a {
                                self.tab_action(active, a);
                            }
                        }
                    }
                    return false;
                }
                let loaded = match self.tabs.get(self.active).map(|t| &t.kind) {
                    Some(TabKind::Table(t)) if verb == "related" => t.related.as_ref().is_some_and(|r| r.data.is_some()),
                    Some(TabKind::Table(t)) => t.data.is_some() && !t.loading,
                    _ => false,
                };
                if loaded {
                    return advance(self);
                }
            }
            _ => {
                if let Some(TabKind::Table(t)) = self.tabs.get_mut(active).map(|t| &mut t.kind)
                    && let Some(Ok(rs)) = &t.data
                {
                    let col = |name: &str| rs.columns.iter().position(|c| c == name);
                    let cell = |spec: &str| -> Option<(usize, usize)> {
                        let (r, c) = spec.split_once(':')?;
                        Some((r.parse().ok()?, col(c)?))
                    };
                    match verb {
                        "select" => t.grid.selection = arg.split(',').filter_map(|v| v.parse().ok()).collect(),
                        "menu" => {
                            if let Ok(row) = arg.parse() {
                                t.grid.demo_menu = Some(row);
                            }
                        }
                        "editor" => {
                            if let Some((r, c)) = cell(arg) {
                                t.editor = Some(grid::CellEditor::open(r, c, rs.rows[r][c].as_deref()));
                            }
                        }
                        "edit" => {
                            if let Some((at, v)) = arg.split_once('=').and_then(|(spec, v)| Some((cell(spec)?, v))) {
                                t.changes.insert(at, Some(v.to_string()));
                                t.editor = None;
                            }
                        }
                        "freeze" => t.grid.frozen = arg.split(',').map(String::from).collect(),
                        "scrollx" => t.grid.scroll_x = arg.parse().ok(),
                        "goto" => t.grid.open_goto(arg),
                        _ => {}
                    }
                }
                match verb {
                    "commands" => {
                        if self.commands_editor.is_none() {
                            self.apply(Action::EditCommands);
                        }
                        if let Some(ed) = &mut self.commands_editor {
                            ed.selected = 2;
                            let want = ((elapsed.as_millis() / 60) as usize + 1).min(arg.chars().count());
                            ed.preview_args = arg.chars().take(want).collect();
                            if want < arg.chars().count() {
                                return false;
                            }
                        }
                    }
                    "done" => {
                        if let Ok(path) = std::env::var("RGRIP_DEMO_DONE") {
                            let _ = std::fs::write(path, "done");
                        }
                    }
                    _ => {}
                }
                return advance(self);
            }
        }
        false
    }

    #[cfg(not(debug_assertions))]
    fn demo_step(&mut self) -> bool {
        false
    }

    #[cfg(debug_assertions)]
    pub(super) fn demo_hook(&mut self) {
        self.demo_scene();
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
                    c.popup = complete::complete(&c.sql, at, &catalog, c.schema_or_default(&[]), false).map(Popup::new);
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
                // `RGRIP_DEMO_SCHEMA=demo,legacy`: several schemas.
                let mut list: Vec<String> = std::env::var("RGRIP_DEMO_SCHEMA").ok().map(|v| v.split(',').map(String::from).collect()).unwrap_or_default();
                c.schema = (!list.is_empty()).then(|| list.remove(0));
                c.more_schemas = list;
                c.database = std::env::var("RGRIP_DEMO_DATABASE").ok();
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

/// Screenshot scenes (debug builds): `<var>=<index>` opens that item's
/// context menu once, as if right-clicked.
pub(super) fn demo_open_menu(ui: &Ui, var: &'static str, index: usize, resp: &egui::Response) {
    if !cfg!(debug_assertions) || std::env::var(var).ok().and_then(|v| v.parse::<usize>().ok()) != Some(index) {
        return;
    }
    static OPENED: std::sync::Mutex<Vec<&str>> = std::sync::Mutex::new(Vec::new());
    let mut opened = OPENED.lock().unwrap();
    if !opened.contains(&var) {
        opened.push(var);
        let at = resp.rect.left_center() + egui::vec2(60.0, 6.0);
        #[allow(deprecated)] // the Popup API has no "open at" for context menus
        ui.ctx().memory_mut(|m| m.open_popup_at(egui::Popup::default_response_id(resp), at));
    }
}

use super::*;

/// A choice in the "Move to folder" submenu.
enum FolderPick {
    To(Option<String>),
    New,
}

/// "Move to folder" submenu: the folders, back to the top level, or a new one.
fn move_to_folder_menu(ui: &mut Ui, folders: &[String], current: Option<&str>) -> Option<FolderPick> {
    let mut pick = None;
    crate::icons::submenu(ui, Svg::FolderOpen, "Move to folder", |ui| {
        for f in folders.iter().filter(|f| Some(f.as_str()) != current) {
            if ui.button(f).clicked() {
                pick = Some(FolderPick::To(Some(f.clone())));
                ui.close();
            }
        }
        if current.is_some() && ui.button("No folder").clicked() {
            pick = Some(FolderPick::To(None));
            ui.close();
        }
        if folders.iter().any(|f| Some(f.as_str()) != current) || current.is_some() {
            ui.separator();
        }
        if ui.button("New folder…").clicked() {
            pick = Some(FolderPick::New);
            ui.close();
        }
    });
    pick
}

/// While a row is dragged: a grab cursor and its label by the pointer.
fn drag_ghost(ui: &Ui, resp: &egui::Response, label: &str) {
    if !resp.dragged() {
        return;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    if let Some(pos) = ui.ctx().pointer_interact_pos() {
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("explorer-drag")));
        let text = painter.layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), theme::TEXT());
        let rect = egui::Rect::from_min_size(pos + egui::vec2(14.0, 4.0), text.size()).expand2(egui::vec2(7.0, 4.0));
        painter.rect(rect, 5.0, theme::SURFACE(), Stroke::new(1.0_f32, theme::BORDER()), egui::StrokeKind::Inside);
        painter.galley(rect.min + egui::vec2(7.0, 4.0), text, theme::TEXT());
    }
}

/// Outline a row that would take the item being dragged.
fn drop_highlight(ui: &Ui, resp: &egui::Response) {
    ui.painter().rect_stroke(resp.rect.shrink(0.5), 5.0, Stroke::new(1.5_f32, theme::ACCENT()), egui::StrokeKind::Inside);
}

// ---- explorer ---------------------------------------------------------

impl App {
    pub(super) fn explorer(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Explorer").size(12.0).color(theme::MUTED()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, "+").on_hover_text("New connection").clicked() {
                    actions.push(Action::NewConnection);
                }
            });
        });
        ui.add_space(6.0);
        let search = ui.add(
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

        self.explorer_keys(ui, &search, actions);
        let filter = self.filter.trim().to_lowercase();
        let scroll_to = self.scroll_to_row.take();
        let conn_folders = conn_folders(&self.conns, &self.conn_folders);
        let tree = Tree {
            expanded: &self.expanded,
            collapsed: &self.settings.collapsed_folders,
            pinned: &self.settings.pinned,
            conn_folders: &conn_folders,
            indent: Default::default(),
            selected: self.selected.as_deref(),
            filter: &filter,
            headers: Default::default(),
            rows: Default::default(),
            scroll_to: scroll_to.as_ref().map(|(k, align)| (k.as_str(), *align)),
        };
        let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
        // Debug screenshots: `RGRIP_DEMO_EXPLORER_SCROLL=300` (kept, so the
        // tree has loaded by the time of the capture).
        if cfg!(debug_assertions) && let Some(y) = std::env::var("RGRIP_DEMO_EXPLORER_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
            scroll = scroll.vertical_scroll_offset(y);
        }
        let out = scroll.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            // Pinned folders and connections first, then folders (by name),
            // then connections not in one; pinned ones lead inside a folder too.
            let pinned = |key: String| self.settings.pinned.contains(&key);
            let conn_pinned = |c: &Conn| pinned(format!("c/{}", c.cfg.id));
            let loose: Vec<(usize, &Conn)> = self.conns.iter().enumerate().filter(|(_, c)| c.cfg.folder.is_none()).collect();
            for top in [true, false] {
                for f in conn_folders.iter().filter(|f| pinned(FolderKind::Conns.key(f)) == top) {
                    let mut members: Vec<(usize, &Conn)> = self.conns.iter().enumerate().filter(|(_, c)| c.cfg.folder.as_ref() == Some(f)).collect();
                    members.sort_by_key(|(_, c)| !conn_pinned(c));
                    tree.conn_folder(ui, f, &members, actions);
                }
                for &(ci, conn) in loose.iter().filter(|(_, c)| conn_pinned(c) == top) {
                    tree.conn(ui, ci, conn, actions);
                }
            }
            self.saved_section(ui, &filter, actions);
            // Right-click below the rows: add a connection or a folder.
            let rest = (ui.clip_rect().bottom() - ui.cursor().top()).max(widgets::ROW_HEIGHT);
            let (_, blank) = ui.allocate_exact_size(egui::vec2(ui.available_width(), rest), egui::Sense::click());
            blank.context_menu(|ui| {
                menu_layout(ui);
                if crate::icons::menu_item(ui, Svg::Plus, "Add Connection…", true).clicked() {
                    actions.push(Action::NewConnection);
                    ui.close();
                }
                if crate::icons::menu_item(ui, Svg::FolderOpen, "Add Folder…", true).clicked() {
                    actions.push(Action::NewFolder(FolderKind::Conns, None));
                    ui.close();
                }
            });
        });
        self.explorer_rows = tree.rows.take();
        // Sticky headers: the connection and schema you're scrolling through
        // stay pinned at the top (drawn last, so they sit on top).
        let headers = tree.headers.into_inner();
        let view = out.inner_rect;
        let row = widgets::ROW_HEIGHT + 1.0;
        let pinned = sticky_headers(&headers, view.top(), row);
        if !pinned.is_empty() {
            // Cover rows peeking above the band (the clip starts a bit higher).
            let band = egui::Rect::from_x_y_ranges(view.x_range(), (view.top() - 10.0)..=view.top());
            ui.painter().with_clip_rect(ui.clip_rect()).rect_filled(band, 0.0, theme::BG());
        }
        for (slot, &i) in pinned.iter().enumerate() {
            let h = &headers[i];
            let rect = egui::Rect::from_min_size(egui::pos2(view.left(), view.top() + slot as f32 * row), egui::vec2(view.width(), row));
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.painter().rect_filled(rect, 0.0, theme::BG());
                if slot + 1 == pinned.len() {
                    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0_f32, theme::BORDER()));
                }
                let resp = widgets::tree_row(
                    ui,
                    TreeRow { depth: h.depth, open: Some(true), icon: h.icon, label: &h.label, label_color: theme::TEXT(), detail: h.detail.as_deref(), selected: false, pinned: false },
                );
                if resp.on_hover_text("Click to collapse").clicked() {
                    actions.push(Action::Toggle(h.key.clone()));
                }
            });
        }
    }

    /// Keyboard use of the tree while it's active (a row was clicked, or ↓
    /// from the search field): arrows move and open/close rows, Enter opens,
    /// and Finder-style typed characters jump to the first matching row.
    fn explorer_keys(&mut self, ui: &Ui, search: &egui::Response, actions: &mut Vec<Action>) {
        const IDLE: Duration = Duration::from_secs(1);
        if search.has_focus() && !self.explorer_rows.is_empty() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            search.surrender_focus();
            self.explorer_active = true;
            let key = self.explorer_rows[0].key.clone();
            (self.selected, self.scroll_to_row) = (Some(key.clone()), Some((key, None)));
            return;
        }
        let panel = ui.max_rect();
        let outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !panel.contains(p)));
        if outside || ui.ctx().memory(|m| m.focused().is_some()) {
            self.explorer_active = false;
        }
        if !self.explorer_active {
            return;
        }
        let at = self.selected.as_ref().and_then(|k| self.explorer_rows.iter().position(|r| &r.key == k));
        let nav = ui.input_mut(|i| {
            [Key::ArrowDown, Key::ArrowUp, Key::ArrowRight, Key::ArrowLeft, Key::Enter]
                .into_iter()
                .filter(|&k| i.consume_key(Modifiers::NONE, k))
                .last()
        });
        match nav.and_then(|k| explorer_nav(&self.explorer_rows, at, k)) {
            Some(Nav::Select(i)) => {
                let key = self.explorer_rows[i].key.clone();
                (self.selected, self.scroll_to_row) = (Some(key.clone()), Some((key, None)));
            }
            Some(Nav::Toggle(i)) => {
                let row = &mut self.explorer_rows[i];
                actions.push(Action::Toggle(row.key.clone()));
                if row.open == Some(false) {
                    actions.extend(row.on_open.take());
                }
            }
            Some(Nav::Enter(i)) => actions.extend(self.explorer_rows[i].on_enter.take()),
            Some(Nav::ToSearch) => {
                self.explorer_active = false;
                search.request_focus();
            }
            None => {}
        }
        let (typed, escape) = ui.input(|i| {
            let typed: String = if i.modifiers.command || i.modifiers.ctrl {
                String::new()
            } else {
                i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.as_str()) } else { None }).collect()
            };
            (typed, i.key_pressed(Key::Escape))
        });
        let (buffer, at) = &mut self.type_ahead;
        if escape || at.elapsed() > IDLE {
            buffer.clear();
        }
        if typed.is_empty() {
            return;
        }
        buffer.push_str(&typed);
        *at = Instant::now();
        if let Some(i) = type_ahead_match(&self.explorer_rows, buffer) {
            let key = self.explorer_rows[i].key.clone();
            self.selected = Some(key.clone());
            self.scroll_to_row = Some((key, Some(Align::Center)));
        }
    }

    /// Consoles that belong to a saved query save themselves shortly after
    /// the last edit (quietly; ⌘S still saves right away).
    pub(super) fn autosave(&mut self) {
        const IDLE: Duration = Duration::from_millis(900);
        let mut errors = Vec::new();
        for tab in &self.tabs {
            let TabKind::Console(c) = &tab.kind else { continue };
            let Some(id) = &c.saved_id else { continue };
            // A new database or schema pick saves too, not just SQL edits.
            let picked = c.picked_schemas();
            let Some(q) = self.saved.iter_mut().find(|q| &q.id == id).filter(|q| q.sql != c.sql || q.database != c.database || q.schemas != picked) else { continue };
            if let Some(at) = c.last_edit
                && at.elapsed() < IDLE
            {
                self.egui_ctx.request_repaint_after(IDLE - at.elapsed());
                continue;
            }
            q.sql = c.sql.clone();
            q.connection = Some(tab.conn.clone());
            (q.database, q.schemas) = (c.database.clone(), picked);
            if let Err(e) = config::update_query(q) {
                errors.push(format!("Could not save “{}”: {e}", q.name));
            }
        }
        if let Some(e) = errors.pop() {
            self.toast = Some((e, theme::ERROR(), Instant::now()));
        }
    }

    /// Title strip for the full-size-content macOS window: drag/zoom area,
    /// plus appearance, updates, GitHub and sponsor buttons on the right.
    pub(super) fn title_bar(&self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        const REPO: &str = "https://github.com/yoouTaro/rgrip";
        egui::TopBottomPanel::top("titlebar")
            .exact_height(30.0)
            .frame(Frame::new().fill(theme::BG()).inner_margin(Margin { left: 0, right: 10, top: 0, bottom: 0 }))
            .show(ctx, |ui| {
                let bar = ui.max_rect();
                ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0_f32, theme::BORDER()));
                // Registered first so the buttons below sit on top of it.
                let resp = ui.interact(bar, Id::new("titlebar-drag"), Sense::click_and_drag());
                if resp.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if resp.double_clicked() {
                    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
                // Painted, not a Label: a (selectable) label would take the
                // double-click and select its text instead of zooming the window.
                ui.painter().text(bar.center(), egui::Align2::CENTER_CENTER, "rgrip", FontId::proportional(12.0), theme::FAINT());

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let gear = title_button(ui, "Settings", |ui, r, hot| {
                        let tint = if hot { theme::TEXT() } else { theme::MUTED() };
                        crate::icons::paint(ui, crate::icons::Svg::Settings, r.shrink(5.0), tint);
                    });
                    self.settings_menu(&gear, actions);
                    if title_button(ui, "Sponsor rgrip", |ui, r, hot| {
                        let tint = if hot { Color32::from_rgb(0xea, 0x4a, 0xaa) } else { theme::MUTED() };
                        crate::icons::paint(ui, crate::icons::Svg::Heart, r.shrink(5.5), tint);
                    })
                    .clicked()
                    {
                        ctx.open_url(egui::OpenUrl::new_tab("https://github.com/sponsors/yoouTaro"));
                    }
                    if title_button(ui, "rgrip on GitHub", |ui, r, hot| {
                        let tint = if hot { theme::TEXT() } else { theme::MUTED() };
                        crate::icons::paint(ui, crate::icons::Svg::GitHub, r.shrink(5.0), tint);
                    })
                    .clicked()
                    {
                        ctx.open_url(egui::OpenUrl::new_tab(REPO));
                    }
                    match &self.update {
                        UpdateState::Available(r) => {
                            let text = RichText::new(format!("Update to {}", r.version)).size(12.0).color(theme::BG());
                            let b = Button::new(text).fill(theme::ACCENT()).corner_radius(6);
                            if ui.add(b).on_hover_text("Download, verify and restart").clicked() {
                                actions.push(Action::InstallUpdate);
                            }
                        }
                        UpdateState::Checking { .. } | UpdateState::Installing(_) => {
                            ui.add_space(4.0);
                            ui.add(egui::Spinner::new().size(13.0));
                            ui.add_space(4.0);
                        }
                        UpdateState::Idle => {
                            let tip = format!("Check for updates (rgrip {})", update::CURRENT);
                            if title_button(ui, &tip, |ui, r, hot| {
                                let tint = if hot { theme::TEXT() } else { theme::MUTED() };
                                crate::icons::paint(ui, crate::icons::Svg::Refresh, r.shrink(6.0), tint);
                            })
                            .clicked()
                            {
                                actions.push(Action::CheckUpdate);
                            }
                        }
                    }
                    // Appearance: System → Light → Dark.
                    let (icon, label, next) = match self.settings.theme {
                        ThemeChoice::System => (Icon::System, "System", ThemeChoice::Light),
                        ThemeChoice::Light => (Icon::Sun, "Light", ThemeChoice::Dark),
                        ThemeChoice::Dark => (Icon::Moon, "Dark", ThemeChoice::System),
                    };
                    if title_button(ui, &format!("Appearance: {label} (click to switch)"), |ui, r, hot| {
                        let tint = if hot { theme::TEXT() } else { theme::MUTED() };
                        widgets::paint_icon_tinted(ui.painter(), r.center(), icon, Some(tint));
                    })
                    .clicked()
                    {
                        actions.push(Action::SetTheme(next));
                    }
                });
            });
    }

    /// The settings menu, opened from the title bar's gear button.
    fn settings_menu(&self, button: &egui::Response, actions: &mut Vec<Action>) {
        const WIDTH: f32 = 260.0;
        let mut popup = egui::Popup::menu(button);
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
            menu_heading(ui, "Editor");
            if ui.button("Slash commands…").on_hover_text("Add or edit /commands and their SQL templates").clicked() {
                actions.push(Action::EditCommands);
                ui.close();
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
        if self.saved.is_empty() && self.query_folders.is_empty() {
            return;
        }
        ui.add_space(14.0);
        let header = ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Saved queries").size(12.0).color(theme::MUTED()));
            ui.label(RichText::new(self.saved.len().to_string()).size(11.0).color(theme::FAINT()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, "↗").on_hover_text("Open queries folder in Finder").clicked() {
                    actions.push(Action::OpenQueriesFolder);
                }
                if icon_button(ui, "+").on_hover_text("New folder").clicked() {
                    actions.push(Action::NewFolder(FolderKind::Queries, None));
                }
            });
        });
        // Dropping a query on the header takes it out of its folder.
        let header = header.response.interact(egui::Sense::hover());
        if let Some(DragItem::Saved(id)) = header.dnd_hover_payload::<DragItem>().as_deref()
            && id.contains('/')
        {
            drop_highlight(ui, &header);
        }
        if let Some(DragItem::Saved(id)) = header.dnd_release_payload::<DragItem>().as_deref() {
            actions.push(Action::MoveSaved(id.clone(), None));
        }
        ui.add_space(4.0);
        let active_saved = self.tabs.get(self.active).and_then(|t| match &t.kind {
            TabKind::Console(c) => c.saved_id.clone(),
            TabKind::Table(_) | TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_) => None,
        });
        let matches = |q: &SavedQuery| filter.is_empty() || q.name.to_lowercase().contains(filter);
        let mut folders: Vec<&String> = self.query_folders.iter().collect();
        folders.sort_by_key(|f| !self.settings.pinned.contains(&FolderKind::Queries.key(f)));
        for f in folders {
            let members: Vec<(usize, &SavedQuery)> =
                self.saved.iter().enumerate().filter(|(_, q)| q.folder.as_ref() == Some(f) && matches(q)).collect();
            if !filter.is_empty() && members.is_empty() {
                continue;
            }
            let key = FolderKind::Queries.key(f);
            let open = !filter.is_empty() || !self.settings.collapsed_folders.contains(&key);
            let pinned = self.settings.pinned.contains(&key);
            let resp = widgets::tree_row(
                ui,
                TreeRow {
                    depth: 0,
                    open: Some(open),
                    icon: Icon::Folder,
                    label: f,
                    label_color: theme::TEXT(),
                    detail: Some(&members.len().to_string()),
                    selected: false,
                    pinned,
                },
            );
            if resp.clicked() {
                actions.push(Action::Toggle(key));
            }
            if let Some(DragItem::Saved(id)) = resp.dnd_hover_payload::<DragItem>().as_deref()
                && members.iter().all(|(_, q)| &q.id != id)
            {
                drop_highlight(ui, &resp);
            }
            if let Some(DragItem::Saved(id)) = resp.dnd_release_payload::<DragItem>().as_deref() {
                actions.push(Action::MoveSaved(id.clone(), Some(f.clone())));
            }
            resp.context_menu(|ui| folder_menu(ui, FolderKind::Queries, f, pinned, actions));
            if open {
                for (i, q) in members {
                    self.saved_row(ui, 1, i, q, active_saved.as_deref(), actions);
                }
            }
        }
        for (i, q) in self.saved.iter().enumerate().filter(|(_, q)| q.folder.is_none() && matches(q)) {
            self.saved_row(ui, 0, i, q, active_saved.as_deref(), actions);
        }
    }

    fn saved_row(&self, ui: &mut Ui, depth: usize, i: usize, q: &SavedQuery, active: Option<&str>, actions: &mut Vec<Action>) {
        let conn = q.connection.as_deref().and_then(|id| self.conn_index(id)).map(|ci| self.conns[ci].cfg.display_name());
        let resp = widgets::tree_row_sense(
            ui,
            TreeRow {
                depth,
                open: None,
                icon: Icon::Query,
                label: &q.name,
                label_color: theme::TEXT(),
                detail: conn.as_deref(),
                selected: active == Some(q.id.as_str()),
                pinned: false,
            },
            egui::Sense::click_and_drag(),
        );
        if resp.clicked() {
            actions.push(Action::OpenSaved(i));
        }
        resp.dnd_set_drag_payload(DragItem::Saved(q.id.clone()));
        drag_ghost(ui, &resp, &q.name);
        // Dropping a query on another one puts it in that one's folder.
        if let Some(DragItem::Saved(id)) = resp.dnd_hover_payload::<DragItem>().as_deref()
            && id != &q.id
        {
            drop_highlight(ui, &resp);
        }
        if let Some(DragItem::Saved(id)) = resp.dnd_release_payload::<DragItem>().as_deref() {
            actions.push(Action::MoveSaved(id.clone(), q.folder.clone()));
        }
        let preview: String = q.sql.lines().take(12).collect::<Vec<_>>().join("\n");
        resp.on_hover_text(RichText::new(preview).monospace().size(11.5)).context_menu(|ui| {
            menu_layout(ui);
            if crate::icons::menu_item(ui, Svg::Terminal, "Open", true).clicked() {
                actions.push(Action::OpenSaved(i));
                ui.close();
            }
            if crate::icons::menu_item(ui, Svg::Pencil, "Rename…", true).clicked() {
                actions.push(Action::RenameSaved(i));
                ui.close();
            }
            match move_to_folder_menu(ui, &self.query_folders, q.folder.as_deref()) {
                Some(FolderPick::To(folder)) => actions.push(Action::MoveSaved(q.id.clone(), folder)),
                Some(FolderPick::New) => actions.push(Action::NewFolder(FolderKind::Queries, Some(q.id.clone()))),
                None => {}
            }
            if crate::icons::menu_item(ui, Svg::FolderOpen, "Reveal in Finder", true).clicked() {
                actions.push(Action::RevealSaved(i));
                ui.close();
            }
            ui.separator();
            if crate::icons::danger_item(ui, Svg::Trash, "Move to Trash").clicked() {
                actions.push(Action::DeleteSaved(i));
                ui.close();
            }
        });
    }

    pub(super) fn status_bar(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
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
                if let Some(col) = theme::conn_color(c.cfg.color) {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(3.0, 12.0), Sense::hover());
                    ui.painter().rect_filled(r, 1.5, col);
                }
                ui.label(
                    RichText::new(format!("{}   {}:{}/{}   {text}", c.cfg.display_name(), c.cfg.host, c.cfg.port, c.cfg.database))
                        .size(11.5)
                        .color(theme::MUTED()),
                );
                if c.cfg.read_only {
                    read_only_badge(ui);
                }
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
            ui.painter().rect_stroke(seg, 5.0, Stroke::new(1.0_f32, theme::BORDER()), egui::StrokeKind::Inside);
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
    p.rect_stroke(swatch, 4.0, Stroke::new(1.0_f32, theme::BORDER()), egui::StrokeKind::Inside);
    p.circle_filled(swatch.center() - egui::vec2(4.5, 0.0), 3.5, t.accent);
    p.circle_filled(swatch.center() + egui::vec2(4.5, 0.0), 3.5, t.text);
    let color = if selected { theme::TEXT() } else { theme::MUTED() };
    p.text(egui::pos2(swatch.right() + 9.0, rect.center().y), egui::Align2::LEFT_CENTER, t.name, FontId::proportional(13.0), color);
    if selected {
        p.text(egui::pos2(rect.right() - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, "✓", FontId::proportional(13.0), theme::ACCENT());
    }
    resp
}

pub(super) fn status_style(s: &Status) -> (Color32, String) {
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
    /// Folder keys shown closed (folders start open).
    collapsed: &'a [String],
    /// Row keys pinned to the top.
    pinned: &'a [String],
    /// Connection folders, for the "Move to folder" menu.
    conn_folders: &'a [String],
    /// Extra depth for rows drawn inside a folder.
    indent: std::cell::Cell<usize>,
    selected: Option<&'a str>,
    filter: &'a str,
    /// Open connection/schema rows as drawn, for the sticky headers.
    headers: std::cell::RefCell<Vec<StickyHeader>>,
    /// Every row as drawn, for keyboard navigation.
    rows: std::cell::RefCell<Vec<NavRow>>,
    /// Scroll this row into view (a keyboard move or type-ahead jump).
    scroll_to: Option<(&'a str, Option<Align>)>,
}

/// A tree row as drawn, for keyboard navigation.
pub(super) struct NavRow {
    key: String,
    label: String,
    /// Indent, folders included.
    depth: usize,
    /// Expandable rows: whether it's open.
    open: Option<bool>,
    /// What opening it does besides expanding (connecting).
    on_open: Option<Action>,
    /// What Enter does (a table opens its data).
    on_enter: Option<Action>,
}

/// A keyboard move in the explorer tree.
#[derive(Debug, PartialEq)]
enum Nav {
    Select(usize),
    Toggle(usize),
    Enter(usize),
    /// ↑ on the first row: back to the search field.
    ToSearch,
}

/// What `key` does with row `at` selected: ↑↓ move, → opens a row (or
/// steps into an open one), ← closes it (or steps out to its parent),
/// Enter opens a table, else toggles.
fn explorer_nav(rows: &[NavRow], at: Option<usize>, key: Key) -> Option<Nav> {
    let last = rows.len().checked_sub(1)?;
    let Some(i) = at else {
        return match key {
            Key::ArrowDown => Some(Nav::Select(0)),
            Key::ArrowUp => Some(Nav::Select(last)),
            _ => None,
        };
    };
    let row = &rows[i];
    match key {
        Key::ArrowDown => Some(Nav::Select((i + 1).min(last))),
        Key::ArrowUp if i == 0 => Some(Nav::ToSearch),
        Key::ArrowUp => Some(Nav::Select(i - 1)),
        Key::ArrowRight => match row.open {
            Some(false) => Some(Nav::Toggle(i)),
            Some(true) => rows.get(i + 1).filter(|r| r.depth > row.depth).map(|_| Nav::Select(i + 1)),
            None => None,
        },
        Key::ArrowLeft if row.open == Some(true) => Some(Nav::Toggle(i)),
        Key::ArrowLeft => rows[..i].iter().rposition(|r| r.depth < row.depth).map(Nav::Select),
        Key::Enter if row.on_enter.is_some() => Some(Nav::Enter(i)),
        Key::Enter => row.open.map(|_| Nav::Toggle(i)),
        _ => None,
    }
}

/// The type-ahead target: the first row whose label starts with `query`,
/// else the first that contains it (ignoring case).
fn type_ahead_match(rows: &[NavRow], query: &str) -> Option<usize> {
    let q = query.to_lowercase();
    let labels: Vec<String> = rows.iter().map(|r| r.label.to_lowercase()).collect();
    labels.iter().position(|l| l.starts_with(&q)).or_else(|| labels.iter().position(|l| l.contains(&q)))
}

/// An open folder, connection or schema row, where it was drawn. Depth is
/// the row's indent: a connection in a folder is at 1, its schemas at 2.
struct StickyHeader {
    depth: usize,
    key: String,
    icon: Icon,
    label: String,
    detail: Option<String>,
    top: f32,
}

/// Which headers to pin at the top of the explorer for a viewport starting
/// at `top`: the folder, connection and schema whose rows scrolled out of
/// view while their contents are still showing.
fn sticky_headers(headers: &[StickyHeader], top: f32, row: f32) -> Vec<usize> {
    let mut pinned: Vec<usize> = Vec::new();
    for depth in 0..3 {
        let slot = top + pinned.len() as f32 * row;
        // Only look inside the last pinned header for the next level.
        let (from, to) = match pinned.last() {
            Some(&p) => {
                let d = headers[p].depth;
                (p + 1, headers[p + 1..].iter().position(|h| h.depth <= d).map_or(headers.len(), |n| p + 1 + n))
            }
            None => (0, headers.len()),
        };
        let Some(i) = (from..to).rev().find(|&i| headers[i].depth == depth && headers[i].top < slot) else { break };
        // Its section ends at the next header of the same or a higher level.
        let end = headers[i + 1..].iter().find(|h| h.depth <= depth).map_or(f32::INFINITY, |h| h.top);
        if end <= slot + row {
            break;
        }
        pinned.push(i);
    }
    pinned
}

impl Tree<'_> {
    fn row(&self, ui: &mut Ui, key: &str, depth: usize, expandable: bool, icon: Icon, label: &str, detail: Option<&str>, actions: &mut Vec<Action>) -> (egui::Response, bool) {
        let depth = depth + self.indent.get();
        let folder = key.starts_with("fc/");
        // While searching, folders, connections and schemas show their matches open.
        let searching = !self.filter.is_empty() && (folder || key.starts_with("c/") || key.starts_with("s/"));
        let open = searching || if folder { !self.collapsed.iter().any(|k| k == key) } else { self.expanded.contains(key) };
        let header_depth = (folder || key.starts_with("c/") || key.starts_with("s/")).then_some(depth);
        let sense = if key.starts_with("c/") { egui::Sense::click_and_drag() } else { egui::Sense::click() };
        let resp = widgets::tree_row_sense(
            ui,
            TreeRow {
                depth,
                open: expandable.then_some(open),
                icon,
                label,
                label_color: theme::TEXT(),
                detail,
                selected: self.selected == Some(key),
                pinned: self.pinned.iter().any(|k| k == key),
            },
            sense,
        );
        if let Some(depth) = header_depth.filter(|_| open) {
            self.headers.borrow_mut().push(StickyHeader {
                depth,
                key: key.to_string(),
                icon,
                label: label.to_string(),
                detail: detail.map(str::to_string),
                top: resp.rect.top(),
            });
        }
        // Rows held open by a search don't toggle: ← steps out of them instead.
        let toggles = expandable && !searching;
        self.rows.borrow_mut().push(NavRow { key: key.to_string(), label: label.to_string(), depth, open: toggles.then_some(open), on_open: None, on_enter: None });
        match self.scroll_to {
            Some((k, Some(align))) if k == key => resp.scroll_to_me(Some(align)),
            // Room for the sticky headers above, and a row of context below.
            Some((k, None)) if k == key => ui.scroll_to_rect(resp.rect.expand2(egui::vec2(0.0, 3.0 * widgets::ROW_HEIGHT)), None),
            _ => {}
        }
        if resp.clicked() {
            actions.push(Action::Select(key.to_string()));
            if expandable {
                actions.push(Action::Toggle(key.to_string()));
            }
        }
        (resp, open)
    }

    fn note(&self, ui: &mut Ui, depth: usize, text: &str, color: Color32) {
        widgets::tree_note(ui, depth + self.indent.get(), text, color);
    }

    /// A connection folder and, when open, its connections.
    fn conn_folder(&self, ui: &mut Ui, name: &str, members: &[(usize, &Conn)], actions: &mut Vec<Action>) {
        let key = FolderKind::Conns.key(name);
        let (resp, open) = self.row(ui, &key, 0, true, Icon::Folder, name, Some(&members.len().to_string()), actions);
        if let Some(DragItem::Conn(id)) = resp.dnd_hover_payload::<DragItem>().as_deref()
            && members.iter().all(|(_, c)| &c.cfg.id != id)
        {
            drop_highlight(ui, &resp);
        }
        if let Some(DragItem::Conn(id)) = resp.dnd_release_payload::<DragItem>().as_deref() {
            actions.push(Action::MoveConn(id.clone(), Some(name.to_string())));
        }
        let pinned = self.pinned.contains(&key);
        resp.context_menu(|ui| folder_menu(ui, FolderKind::Conns, name, pinned, actions));
        if open {
            self.indent.set(self.indent.get() + 1);
            for &(ci, conn) in members {
                self.conn(ui, ci, conn, actions);
            }
            self.indent.set(self.indent.get() - 1);
        }
    }

    fn conn(&self, ui: &mut Ui, ci: usize, conn: &Conn, actions: &mut Vec<Action>) {
        let key = format!("c/{}", conn.cfg.id);
        let (dot, _) = status_style(&conn.status);
        let detail = match conn.status {
            Status::Connecting => Some("connecting…"),
            _ if conn.cfg.read_only => Some("read-only"),
            _ => None,
        };
        let name = conn.cfg.display_name();
        let (resp, open) = self.row(ui, &key, 0, true, Icon::Database(dot, theme::conn_color(conn.cfg.color)), &name, detail, actions);
        if matches!(conn.status, Status::Disconnected | Status::Failed(_)) {
            if resp.clicked() && !open {
                actions.push(Action::Connect(ci));
            }
            self.rows.borrow_mut().last_mut().unwrap().on_open = Some(Action::Connect(ci));
        }
        resp.dnd_set_drag_payload(DragItem::Conn(conn.cfg.id.clone()));
        drag_ghost(ui, &resp, &name);
        // Dropping a connection on another one puts it in that one's folder.
        if let Some(DragItem::Conn(id)) = resp.dnd_hover_payload::<DragItem>().as_deref()
            && id != &conn.cfg.id
        {
            drop_highlight(ui, &resp);
        }
        if let Some(DragItem::Conn(id)) = resp.dnd_release_payload::<DragItem>().as_deref() {
            actions.push(Action::MoveConn(id.clone(), conn.cfg.folder.clone()));
        }
        let resp = resp.on_hover_text(format!("{}@{}:{}/{}", conn.cfg.user, conn.cfg.host, conn.cfg.port, conn.cfg.database));
        demo_open_menu(ui, "RGRIP_DEMO_CONN_MENU", ci, &resp);
        let pinned = self.pinned.contains(&key);
        resp.context_menu(|ui| conn_menu(ui, ci, conn, self.conn_folders, pinned, actions));
        if !open {
            return;
        }
        match &conn.status {
            Status::Disconnected | Status::Connecting => {}
            Status::Failed(e) => self.note(ui, 1, e, theme::ERROR()),
            Status::Connected(_) => match &conn.schemas {
                None => actions.push(Action::LoadSchemas(ci)),
                Some(Load::Loading) => self.note(ui, 1, "loading…", theme::FAINT()),
                Some(Load::Failed(e)) => self.note(ui, 1, e, theme::ERROR()),
                Some(Load::Ready(list)) if !self.filter.is_empty() && !conn.catalog.tables.is_empty() => {
                    // Search every schema (not just expanded ones) via the catalog.
                    let mut any = false;
                    for schema in list {
                        let hits: Vec<TableInfo> = conn
                            .catalog
                            .tables
                            .iter()
                            .filter(|t| &t.schema == schema && t.name.to_lowercase().contains(self.filter))
                            .map(|t| TableInfo { name: t.name.clone(), kind: t.kind })
                            .collect();
                        if hits.is_empty() {
                            continue;
                        }
                        any = true;
                        let key = format!("s/{}/{schema}", conn.cfg.id);
                        self.row(ui, &key, 1, true, Icon::Schema, schema, Some(&hits.len().to_string()), actions);
                        for t in &hits {
                            self.table(ui, ci, conn, schema, t, actions);
                        }
                    }
                    if !any {
                        self.note(ui, 1, "no matching tables", theme::FAINT());
                    }
                }
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
        let (resp, open) = self.row(ui, &key, 1, true, Icon::Schema, schema, count.as_deref(), actions);
        resp.context_menu(|ui| {
            menu_layout(ui);
            if crate::icons::menu_item(ui, Svg::GitBranch, "Open diagram", true).clicked() {
                actions.push(Action::OpenDiagram(ci, schema.to_string()));
                ui.close();
            }
            if crate::icons::menu_item(ui, Svg::Upload, "Import data…", !conn.cfg.read_only).clicked() {
                actions.push(Action::Import(ci, schema.to_string(), None));
                ui.close();
            }
        });
        if !open {
            return;
        }
        match conn.tables.get(schema) {
            None => actions.push(Action::LoadTables(ci, schema.to_string())),
            Some(Load::Loading) => self.note(ui, 2, "loading…", theme::FAINT()),
            Some(Load::Failed(e)) => self.note(ui, 2, e, theme::ERROR()),
            Some(Load::Ready(tables)) => {
                let mut shown = 0;
                for t in tables.iter().filter(|t| self.filter.is_empty() || t.name.to_lowercase().contains(self.filter)) {
                    shown += 1;
                    self.table(ui, ci, conn, schema, t, actions);
                }
                if shown == 0 {
                    let msg = if tables.is_empty() { "no tables" } else { "no matches" };
                    self.note(ui, 2, msg, theme::FAINT());
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
        self.rows.borrow_mut().last_mut().unwrap().on_enter = Some(Action::OpenTable(ci, schema.to_string(), t.name.clone()));
        resp.context_menu(|ui| {
            menu_layout(ui);
            if crate::icons::menu_item(ui, Svg::Table, "Open data", true).clicked() {
                actions.push(Action::OpenTable(ci, schema.to_string(), t.name.clone()));
                ui.close();
            }
            if crate::icons::menu_item(ui, Svg::Terminal, "Open in console", true).clicked() {
                actions.push(Action::OpenInConsole(ci, schema.to_string(), t.name.clone()));
                ui.close();
            }
            if matches!(t.kind, TableKind::Table | TableKind::Partitioned) && crate::icons::menu_item(ui, Svg::Upload, "Import into this table…", !conn.cfg.read_only).clicked() {
                actions.push(Action::Import(ci, schema.to_string(), Some(t.name.clone())));
                ui.close();
            }
            if matches!(t.kind, TableKind::Table | TableKind::Partitioned) && crate::icons::menu_item(ui, Svg::Pencil, "Edit table…", !conn.cfg.read_only).clicked() {
                actions.push(Action::EditTable(ci, schema.to_string(), t.name.clone()));
                ui.close();
            }
            // Tables with a geometry column can go on a map layout.
            let geom = conn.catalog.tables.iter().find(|c| c.schema == schema && c.name == t.name).and_then(|c| {
                c.columns.iter().find(|(_, ty)| ty.starts_with("geometry") || ty.starts_with("geography")).map(|(n, _)| n.clone())
            });
            if let Some(col) = geom
                && crate::icons::menu_item(ui, Svg::Map, "Add to map layout", true).clicked()
            {
                actions.push(Action::AddToLayout(ci, schema.to_string(), t.name.clone(), col));
                ui.close();
            }
            if crate::icons::menu_item(ui, Svg::Copy, "Copy name", true).clicked() {
                ui.ctx().copy_text(format!("{}.{}", sql::quote_ident(schema), sql::quote_ident(&t.name)));
                ui.close();
            }
        });
        if !open {
            return;
        }
        match conn.columns.get(&(schema.to_string(), t.name.clone())) {
            None => actions.push(Action::LoadColumns(ci, schema.to_string(), t.name.clone())),
            Some(Load::Loading) => self.note(ui, 3, "loading…", theme::FAINT()),
            Some(Load::Failed(e)) => self.note(ui, 3, e, theme::ERROR()),
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

/// Context menus: wide enough for their longest item, never wrapping
/// ("Copy connection URL" used to break over four lines).
pub(crate) fn menu_layout(ui: &mut Ui) {
    ui.set_min_width(190.0);
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
}

/// "Pin to top" / "Unpin" for a connection or folder row.
fn pin_item(ui: &mut Ui, key: String, pinned: bool, actions: &mut Vec<Action>) {
    let label = if pinned { "Unpin" } else { "Pin to top" };
    if crate::icons::menu_item(ui, Svg::Pin, label, true).clicked() {
        actions.push(Action::TogglePin(key));
        ui.close();
    }
}

fn conn_menu(ui: &mut Ui, ci: usize, conn: &Conn, folders: &[String], pinned: bool, actions: &mut Vec<Action>) {
    menu_layout(ui);
    if crate::icons::menu_item(ui, Svg::Terminal, "New console", true).clicked() {
        actions.push(Action::NewConsole(ci));
        ui.close();
    }
    if crate::icons::menu_item(ui, Svg::Undo, "Query history", true).clicked() {
        actions.push(Action::ShowHistory(ci));
        ui.close();
    }
    if crate::icons::menu_item(ui, Svg::Map, "New map layout", true).clicked() {
        actions.push(Action::NewLayout(ci));
        ui.close();
    }
    if crate::icons::menu_item(ui, Svg::Activity, "Activity", true).clicked() {
        actions.push(Action::OpenActivity(ci));
        ui.close();
    }
    crate::icons::submenu(ui, Svg::Link, "Copy connection URL", |ui| {
        if ui.button("Without password").clicked() {
            ui.ctx().copy_text(rgrip_core::conn_string::to_url(&conn.cfg, false));
            ui.close();
        }
        if ui.button("With password").clicked() {
            actions.push(Action::CopyUrlWithPassword(ci));
            ui.close();
        }
    });
    if matches!(conn.status, Status::Connected(_)) {
        if crate::icons::menu_item(ui, Svg::Refresh, "Refresh", true).clicked() {
            actions.push(Action::Refresh(ci));
            ui.close();
        }
        if crate::icons::menu_item(ui, Svg::Power, "Disconnect", true).clicked() {
            actions.push(Action::Disconnect(ci));
            ui.close();
        }
    } else if crate::icons::menu_item(ui, Svg::Power, "Connect", true).clicked() {
        actions.push(Action::Connect(ci));
        ui.close();
    }
    ui.separator();
    // Quicker than the dialog: tint this connection's tabs.
    crate::icons::submenu(ui, Svg::Droplet, "Color", |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for c in rgrip_core::ConnColor::ALL {
                let resp = widgets::color_swatch(ui, theme::conn_color(c), conn.cfg.color == c).on_hover_text(c.label());
                if resp.clicked() {
                    actions.push(Action::SetColor(ci, c));
                    ui.close();
                }
            }
        });
    })
    .on_hover_text("Tint this connection's tabs (e.g. red for production)");
    match move_to_folder_menu(ui, folders, conn.cfg.folder.as_deref()) {
        Some(FolderPick::To(folder)) => actions.push(Action::MoveConn(conn.cfg.id.clone(), folder)),
        Some(FolderPick::New) => actions.push(Action::NewFolder(FolderKind::Conns, Some(conn.cfg.id.clone()))),
        None => {}
    }
    pin_item(ui, format!("c/{}", conn.cfg.id), pinned, actions);
    if crate::icons::menu_item(ui, Svg::Pencil, "Edit…", true).clicked() {
        actions.push(Action::Edit(ci));
        ui.close();
    }
    if crate::icons::danger_item(ui, Svg::Trash, "Delete").clicked() {
        actions.push(Action::Delete(ci));
        ui.close();
    }
}

/// Right-click menu of a folder row.
fn folder_menu(ui: &mut Ui, kind: FolderKind, name: &str, pinned: bool, actions: &mut Vec<Action>) {
    menu_layout(ui);
    if kind == FolderKind::Conns && crate::icons::menu_item(ui, Svg::Plus, "Add Connection", true).clicked() {
        actions.push(Action::NewConnectionIn(name.to_string()));
        ui.close();
    }
    pin_item(ui, kind.key(name), pinned, actions);
    if crate::icons::menu_item(ui, Svg::Pencil, "Rename…", true).clicked() {
        actions.push(Action::RenameFolder(kind, name.to_string()));
        ui.close();
    }
    if kind == FolderKind::Queries && crate::icons::menu_item(ui, Svg::FolderOpen, "Reveal in Finder", true).clicked() {
        actions.push(Action::RevealQueryFolder(name.to_string()));
        ui.close();
    }
    ui.separator();
    let (label, hover) = match kind {
        FolderKind::Conns => ("Remove folder", "The connections stay, at the top level"),
        FolderKind::Queries => ("Remove folder", "The queries stay, moved to the top level"),
    };
    if crate::icons::danger_item(ui, Svg::Trash, label).on_hover_text(hover).clicked() {
        actions.push(Action::DeleteFolder(kind, name.to_string()));
        ui.close();
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_type_ahead_match() {
        let rows: Vec<NavRow> = ["public", "customers", "order_items", "orders", "Accounts"].iter().map(|l| nav_row(l, 0, None)).collect();
        assert_eq!(type_ahead_match(&rows, "o"), Some(2), "first prefix match in tree order");
        assert_eq!(type_ahead_match(&rows, "orders"), Some(3), "a longer buffer narrows it");
        assert_eq!(type_ahead_match(&rows, "acc"), Some(4), "ignores case");
        assert_eq!(type_ahead_match(&rows, "item"), Some(2), "falls back to a substring");
        assert_eq!(type_ahead_match(&rows, "zzz"), None);
    }

    fn nav_row(label: &str, depth: usize, open: Option<bool>) -> NavRow {
        NavRow { key: format!("k/{label}"), label: label.to_string(), depth, open, on_open: None, on_enter: None }
    }

    #[test]
    fn explorer_keyboard_nav() {
        // conn (open) > schema (open) > table (closed), then a closed conn.
        let mut rows = vec![nav_row("conn", 0, Some(true)), nav_row("public", 1, Some(true)), nav_row("orders", 2, Some(false)), nav_row("other", 0, Some(false))];
        rows[2].on_enter = Some(Action::OpenTable(0, "public".into(), "orders".into()));
        let nav = |at, key| explorer_nav(&rows, at, key);
        assert_eq!(nav(None, Key::ArrowDown), Some(Nav::Select(0)), "nothing selected: ↓ starts at the top");
        assert_eq!(nav(None, Key::ArrowUp), Some(Nav::Select(3)), "…and ↑ at the bottom");
        assert_eq!(nav(Some(1), Key::ArrowDown), Some(Nav::Select(2)));
        assert_eq!(nav(Some(3), Key::ArrowDown), Some(Nav::Select(3)), "stays on the last row");
        assert_eq!(nav(Some(0), Key::ArrowUp), Some(Nav::ToSearch));
        assert_eq!(nav(Some(2), Key::ArrowRight), Some(Nav::Toggle(2)), "→ opens a closed row");
        assert_eq!(nav(Some(0), Key::ArrowRight), Some(Nav::Select(1)), "→ on an open row steps into it");
        assert_eq!(nav(Some(1), Key::ArrowLeft), Some(Nav::Toggle(1)), "← closes an open row");
        assert_eq!(nav(Some(2), Key::ArrowLeft), Some(Nav::Select(1)), "← on a closed row goes to its parent");
        assert_eq!(nav(Some(3), Key::ArrowLeft), None, "a top-level row has no parent");
        assert_eq!(nav(Some(2), Key::Enter), Some(Nav::Enter(2)), "Enter opens a table");
        assert_eq!(nav(Some(3), Key::Enter), Some(Nav::Toggle(3)), "…and toggles anything else");
        assert_eq!(explorer_nav(&[], None, Key::ArrowDown), None);
    }

    #[test]
    fn sticky_explorer_headers() {
        let h = |depth, top: f32| StickyHeader { depth, key: String::new(), icon: Icon::Schema, label: String::new(), detail: None, top };
        // conn at 0, schema A at 25 (rows to 500), schema B at 500, conn2 at 900.
        let headers = [h(0, 0.0), h(1, 25.0), h(1, 500.0), h(0, 900.0)];
        let row = 25.0;
        assert!(sticky_headers(&headers, -1.0, row).is_empty(), "nothing scrolled yet");
        assert_eq!(sticky_headers(&headers, 200.0, row), [0, 1], "inside schema A");
        assert_eq!(sticky_headers(&headers, 600.0, row), [0, 2], "inside schema B");
        assert_eq!(sticky_headers(&headers, 860.0, row), [0], "schema B nearly done: only the connection");
        assert_eq!(sticky_headers(&headers, 1000.0, row), [3], "second connection");

        // folder at 0 holding a conn (1) with schema A (2); then a top-level conn.
        let headers = [h(0, 0.0), h(1, 25.0), h(2, 50.0), h(0, 900.0), h(1, 925.0)];
        assert_eq!(sticky_headers(&headers, 200.0, row), [0, 1, 2], "inside a schema in a folder");
        assert_eq!(sticky_headers(&headers, 860.0, row), [0], "folder nearly done: only the folder");
        assert_eq!(sticky_headers(&headers, 1000.0, row), [3, 4], "top-level connection and its schema");
    }
}

use super::*;

// ---- tabs -------------------------------------------------------------

impl App {
    pub(super) fn tab_bar(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        // DataGrip-style strip: square tabs edge to edge, hairline
        // separators, a border under the strip.
        Frame::new()
            .fill(theme::BG())
            .inner_margin(Margin::ZERO)
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("tabs")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for (i, tab) in self.tabs.iter().enumerate() {
                                let dirty = match &tab.kind {
                                    TabKind::Console(c) => c.saved_id.as_ref().is_some_and(|id| {
                                        self.saved.iter().find(|q| &q.id == id).is_some_and(|q| q.sql != c.sql)
                                    }),
                                    TabKind::Table(_) | TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_) => false,
                                };
                                let style = match self.conn_index(&tab.conn).map(|ci| &self.conns[ci]) {
                                    Some(c) => TabStyle {
                                        title: match &tab.kind {
                                            TabKind::Console(k) => k.database.as_deref().and_then(|db| console_title(&tab.title, &c.cfg, db)),
                                            TabKind::Table(_) | TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_) => None,
                                        },
                                        color: theme::conn_color(c.cfg.color),
                                        tip: format!("{} · {}:{}/{}", c.cfg.display_name(), c.cfg.host, c.cfg.port, c.cfg.database),
                                        status: status_style(&c.status).0,
                                    },
                                    None => TabStyle::default(),
                                };
                                tab_button(ui, i, tab, &style, i == self.active, dirty, self.tabs.len(), actions);
                            }
                        });
                    });
            });
        let rect = ui.available_rect_before_wrap();
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, theme::BORDER()));
    }

    pub(super) fn tab_content(&mut self, ui: &mut Ui) -> Option<TabAction> {
        let read_only = self.tabs.get(self.active).and_then(|t| self.conn_index(&t.conn)).is_some_and(|ci| self.conns[ci].cfg.read_only);
        let (catalog, schemas, databases, default_db, load_schemas) = {
            let tab = self.tabs.get(self.active)?;
            let database = match &tab.kind {
                TabKind::Console(c) => c.database.as_deref(),
                TabKind::Table(_) | TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_) => None,
            };
            match self.conn_index(&tab.conn).map(|ci| &self.conns[ci]) {
                Some(c) => {
                    let databases = match &c.databases {
                        Some(Load::Ready(list)) => list.clone(),
                        _ => Vec::new(),
                    };
                    let load = (c.schemas_in(database).is_none() || c.databases.is_none()) && matches!(c.status, Status::Connected(_));
                    (c.catalog_in(database), user_schemas(c, database), databases, c.cfg.database.clone(), load)
                }
                None => Default::default(),
            }
        };
        let dirty = match &self.tabs.get(self.active)?.kind {
            TabKind::Console(c) => c.saved_id.as_ref().is_some_and(|id| self.saved.iter().any(|q| &q.id == id && q.sql != c.sql)),
            TabKind::Table(_) | TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_) => false,
        };
        let tab = self.tabs.get_mut(self.active)?;
        let id = tab.id;
        match &mut tab.kind {
            TabKind::Console(c) => {
                let action = console_ui(ui, id, c, &catalog, &schemas, &databases, &default_db, dirty, read_only);
                action.or(load_schemas.then_some(TabAction::LoadSchemas))
            }
            TabKind::Table(t) => table_ui(ui, id, t, read_only),
            TabKind::Layout(l) => {
                let loads = l.ui(ui, &catalog);
                (!loads.is_empty()).then_some(TabAction::LoadLayers(loads))
            }
            TabKind::Activity(a) => {
                let out = a.ui(ui, read_only);
                match out.signal {
                    Some((pid, terminate)) => Some(TabAction::ConfirmBackend(pid, terminate)),
                    None => out.refresh.then_some(TabAction::RefreshActivity),
                }
            }
            TabKind::Diagram(d) => {
                let out = d.ui(ui);
                match out.open {
                    Some(table) => Some(TabAction::OpenTable { schema: d.schema.clone(), table }),
                    None => out.reload.then_some(TabAction::ReloadDiagram),
                }
            }
        }
    }
}

/// The title of a console opened as `name@database` (optionally with a
/// ` (n)` suffix) once it targets `database` instead; `None` for other titles
/// (saved query names).
pub(super) fn console_title(title: &str, cfg: &ConnectionConfig, database: &str) -> Option<String> {
    let rest = title.strip_prefix(&conn_db_title(cfg, &cfg.database))?;
    (rest.is_empty() || rest.starts_with(" (")).then(|| format!("{}{rest}", conn_db_title(cfg, database)))
}

/// `name@database`, or `database@host` for an unnamed connection (whose
/// display name is already `database@host`).
pub(super) fn conn_db_title(cfg: &ConnectionConfig, database: &str) -> String {
    if cfg.name.trim().is_empty() { format!("{database}@{}", cfg.host) } else { format!("{}@{database}", cfg.name) }
}

/// How a tab looks beyond its own title: its connection's color, tooltip
/// and status.
struct TabStyle {
    /// Replaces the tab's own title (a console on another database).
    title: Option<String>,
    color: Option<Color32>,
    tip: String,
    status: Color32,
}

impl Default for TabStyle {
    fn default() -> Self {
        Self { title: None, color: None, tip: String::new(), status: theme::FAINT() }
    }
}

/// One tab in the tab bar. Returns the tab's rect.
fn tab_button(
    ui: &mut Ui,
    i: usize,
    tab: &Tab,
    style: &TabStyle,
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
    // `*` marks unsaved changes to a saved query (as in DataGrip).
    let base = style.title.as_deref().unwrap_or(&tab.title);
    let title = if dirty { format!("* {base}") } else { base.to_string() };
    let galley = ui.painter().layout_no_wrap(title, TextStyle::Body.resolve(ui.style()), color);
    let size = egui::vec2(PAD_L + ICON + GAP + galley.size().x + GAP + CLOSE + PAD_R, 36.0);

    // The tab background is registered first and the close button last, so
    // the close button sits on top and receives its own clicks.
    let (rect, tab_resp) = ui.allocate_exact_size(size, Sense::click());
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - PAD_R - CLOSE / 2.0, rect.center().y),
        egui::vec2(CLOSE, CLOSE),
    );
    let close_resp = ui.interact(close_rect, Id::new(("tab-close", tab.id)), Sense::click()).on_hover_text("Close  ⌘W");

    let painter = ui.painter();
    // Square, edge to edge: the active tab takes the content's surface so it
    // reads as attached to it; a connection color tints the whole tab.
    let fill = if active {
        theme::SURFACE()
    } else if tab_resp.hovered() {
        theme::HOVER()
    } else {
        theme::BG()
    };
    let fill = match style.color {
        Some(col) => theme::tint(fill, col, if active { 0.34 } else if tab_resp.hovered() { 0.24 } else { 0.16 }),
        None => fill,
    };
    painter.rect_filled(rect, 0.0, fill);
    painter.vline(rect.right() - 0.5, rect.y_range().shrink(9.0), Stroke::new(1.0_f32, theme::BORDER()));
    // The active tab is underlined: the connection's color, else the accent.
    if active {
        let bar = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 3.0), rect.right_bottom());
        painter.rect_filled(bar, 0.0, style.color.unwrap_or(theme::ACCENT()));
    }
    let cy = rect.center().y;
    let icon = match tab.kind {
        TabKind::Console(_) => Icon::Database(style.status, style.color),
        TabKind::Table(_) => Icon::Table,
        TabKind::Layout(_) => Icon::Globe,
        TabKind::Diagram(_) => Icon::Schema,
        TabKind::Activity(_) => Icon::Table,
    };
    widgets::paint_icon(painter, egui::pos2(rect.left() + PAD_L + ICON / 2.0, cy), icon);
    painter.galley(egui::pos2(rect.left() + PAD_L + ICON + GAP, cy - galley.size().y / 2.0), galley, color);
    if close_resp.hovered() {
        painter.rect_filled(close_rect, 4.0, theme::BORDER());
    }
    let x_color = if close_resp.hovered() { theme::TEXT() } else if active || tab_resp.hovered() { theme::MUTED() } else { theme::FAINT() };
    widgets::paint_icon_tinted(painter, close_rect.center(), Icon::Close, Some(x_color));

    if close_resp.clicked() || tab_resp.middle_clicked() {
        actions.push(Action::Close(CloseWhich::One(i)));
    } else if tab_resp.clicked() {
        actions.push(Action::ActivateTab(i));
    }
    let tab_resp = if style.tip.is_empty() { tab_resp } else { tab_resp.on_hover_text(&style.tip) };
    demo_open_menu(ui, "RGRIP_DEMO_TAB_MENU", i, &tab_resp);
    tab_resp.context_menu(|ui| {
        menu_layout(ui);
        let mut item = |ui: &mut Ui, icon: Svg, label: &str, enabled: bool, which: CloseWhich| {
            if crate::icons::menu_item(ui, icon, label, enabled).clicked() {
                actions.push(Action::Close(which));
                ui.close();
            }
        };
        item(ui, Svg::X, "Close", true, CloseWhich::One(i));
        item(ui, Svg::CopyX, "Close others", count > 1, CloseWhich::Others(i));
        item(ui, Svg::ArrowRightToLine, "Close tabs to the right", i + 1 < count, CloseWhich::ToTheRight(i));
        ui.separator();
        item(ui, Svg::CircleX, "Close all", true, CloseWhich::All);
    });
    rect
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_title_follows_database() {
        let cfg = ConnectionConfig { name: "prod".into(), database: "app".into(), ..Default::default() };
        assert_eq!(console_title("prod@app", &cfg, "other").as_deref(), Some("prod@other"));
        assert_eq!(console_title("prod@app (2)", &cfg, "other").as_deref(), Some("prod@other (2)"));
        assert_eq!(console_title("my saved query", &cfg, "other"), None);
        assert_eq!(console_title("prod@app2", &cfg, "other"), None);
    }

    #[test]
    fn read_only_console_and_table_render() {
        let ctx = egui::Context::default();
        let mut console = Console::new();
        let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
        let _ = ctx.run(input(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                console_ui(ui, 1, &mut console, &Catalog { tables: Vec::new() }, &[], &[], "db", false, true);
            });
        });
        let _ = ctx.run(input(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| read_only_badge(ui));
        });
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
        let style = TabStyle { title: None, color: theme::conn_color(ConnColor::Red), tip: "Prod · h:1/db".into(), status: theme::FAINT() };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (i, t) in tabs.iter().enumerate() {
                        rects.push(tab_button(ui, i, t, &style, i == 0, false, tabs.len(), &mut actions));
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
}

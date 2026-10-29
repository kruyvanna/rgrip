//! The jump list: bookmark a tab with its state (⌘⇧K), reopen it from the dialog (⌘K).

use super::*;
use rgrip_core::jumps::{self, JumpItem, JumpState};

/// The persistable part of a tab, or None for kinds that can't be reopened from a bookmark.
fn jump_state(kind: &TabKind) -> Option<JumpState> {
    Some(match kind {
        TabKind::Console(c) => JumpState::Console {
            sql: c.sql.clone(),
            cursor_start: c.cursor.start,
            cursor_end: c.cursor.end,
            database: c.database.clone(),
            schema: c.schema.clone(),
            more_schemas: c.more_schemas.clone(),
        },
        TabKind::Table(t) => JumpState::Table {
            schema: t.schema.clone(),
            table: t.table.clone(),
            filter: t.applied_filter.clone(),
            order_col: t.order.as_ref().map(|(c, _)| c.clone()),
            order_desc: t.order.as_ref().is_some_and(|(_, asc)| !asc),
            page: t.page,
        },
        TabKind::Diagram(d) => {
            let (filter, show_inferred) = d.view_state();
            JumpState::Diagram { schema: d.schema.clone(), filter: filter.to_string(), show_inferred }
        }
        TabKind::Activity(a) => {
            let (filter, show_idle, auto, interval) = a.view_state();
            JumpState::Activity { filter: filter.to_string(), show_idle, auto, interval }
        }
        TabKind::Layout(_) => return None,
    })
}

impl App {
    /// Bookmark tab `i` (a no-op toast for map layouts).
    pub(super) fn add_jump(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        let Some(state) = jump_state(&tab.kind) else {
            self.toast = Some(("This kind of tab can't be added to the jump list".into(), theme::WARN(), Instant::now()));
            return;
        };
        jumps::add(&mut self.jumps, JumpItem::new(tab.title.clone(), tab.conn.clone(), state));
        let saved = jumps::save(&self.jumps);
        if saved.is_ok() {
            self.toast = Some(("Added to the jump list".into(), theme::ACCENT(), Instant::now()));
        }
        self.report(saved);
    }

    /// Reopen a bookmark as a tab with its saved state.
    fn open_jump(&mut self, id: &str) {
        let Some(item) = self.jumps.iter().find(|j| j.id == id).cloned() else { return };
        let Some(ci) = self.conn_index(&item.conn) else {
            self.toast = Some(("The connection of this entry no longer exists".into(), theme::WARN(), Instant::now()));
            return;
        };
        match item.state {
            JumpState::Console { sql, cursor_start, cursor_end, database, schema, more_schemas } => {
                self.open_console(ci);
                if let Some(TabKind::Console(c)) = self.tabs.last_mut().map(|t| &mut t.kind) {
                    let end = sql.chars().count();
                    c.cursor = cursor_start.min(end)..cursor_end.min(end);
                    c.sql = sql;
                    c.database = database;
                    c.schema = schema;
                    c.more_schemas = more_schemas;
                }
            }
            JumpState::Table { schema, table, filter, order_col, order_desc, page } => {
                self.open_table(ci, schema, table);
                let active = self.active;
                if let Some(TabKind::Table(t)) = self.tabs.get_mut(active).map(|t| &mut t.kind) {
                    if t.dirty() {
                        self.toast = Some(("Save or discard your changes in this tab first".into(), theme::WARN(), Instant::now()));
                        return;
                    }
                    t.filter = filter.clone();
                    t.applied_filter = filter;
                    t.order = order_col.map(|c| (c, !order_desc));
                    t.page = page;
                    self.tab_action(active, TabAction::Reload);
                }
            }
            JumpState::Diagram { schema, filter, show_inferred } => {
                self.open_diagram(ci, schema);
                if let Some(TabKind::Diagram(d)) = self.tabs.get_mut(self.active).map(|t| &mut t.kind) {
                    d.set_view_state(filter, show_inferred);
                }
            }
            JumpState::Activity { filter, show_idle, auto, interval } => {
                self.open_activity(ci);
                if let Some(TabKind::Activity(a)) = self.tabs.get_mut(self.active).map(|t| &mut t.kind) {
                    a.set_view_state(filter, show_idle, auto, interval);
                }
            }
        }
    }

    pub(super) fn jump_ui(&mut self, ctx: &egui::Context) {
        use crate::jump_view::Outcome;
        let Some(v) = &mut self.jump_view else { return };
        let names: HashMap<String, String> = self.conns.iter().map(|c| (c.cfg.id.clone(), c.cfg.display_name())).collect();
        let name = |id: &str| names.get(id).cloned().unwrap_or_else(|| "(deleted connection)".into());
        let can_add = self.tabs.get(self.active).is_some_and(|t| jump_state(&t.kind).is_some());
        match v.ui(ctx, &self.jumps, &name, can_add) {
            Outcome::Keep => {}
            Outcome::Close => self.jump_view = None,
            Outcome::Open(id) => {
                self.jump_view = None;
                self.open_jump(&id);
            }
            Outcome::Remove(id) => {
                jumps::remove(&mut self.jumps, &id);
                let r = jumps::save(&self.jumps);
                self.report(r);
            }
            Outcome::AddCurrent => self.add_jump(self.active),
        }
    }
}

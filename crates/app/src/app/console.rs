use super::*;

/// Keys for the find bar, taken before the editor and the run shortcuts see them.
#[derive(Default)]
struct FindKeys {
    open: bool,
    with_replace: bool,
    /// Next (true) or previous match.
    step: Option<bool>,
    replace_one: bool,
    replace_all: bool,
    close: bool,
}

fn find_field_id(tab_id: TabId, field: FindField) -> Id {
    Id::new(("find", tab_id, field == FindField::Replace))
}

fn take_find_keys(ui: &mut Ui, tab_id: TabId, c: &Console) -> FindKeys {
    let mut k = FindKeys::default();
    // egui drops focus on Escape before this runs, hence "had focus last frame".
    let focused = |ui: &Ui, field| c.find.open && ui.memory(|m| m.has_focus(find_field_id(tab_id, field)) || m.had_focus_last_frame(find_field_id(tab_id, field)));
    let (in_find, in_replace) = (focused(ui, FindField::Find), focused(ui, FindField::Replace));
    ui.input_mut(|i| {
        // Alt first: a held ⌥ still matches the plain shortcut. ⇧⌘F (format) is taken before this.
        if i.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::F) {
            (k.open, k.with_replace) = (true, true);
        } else if i.consume_key(Modifiers::COMMAND, Key::F) {
            k.open = true;
        }
        if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::G) {
            k.step = Some(false);
        } else if i.consume_key(Modifiers::COMMAND, Key::G) {
            k.step = Some(true);
        }
        if in_find || in_replace {
            if i.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::Enter) {
                k.replace_all = true;
            } else if i.consume_key(Modifiers::SHIFT, Key::Enter) {
                k.step = Some(false);
            } else if i.consume_key(Modifiers::NONE, Key::Enter) {
                if in_replace {
                    k.replace_one = true;
                } else {
                    k.step = Some(true);
                }
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                k.close = true;
            }
        }
    });
    k
}

fn cursor_bytes(c: &Console) -> std::ops::Range<usize> {
    sql::char_to_byte(&c.sql, c.cursor.start)..sql::char_to_byte(&c.sql, c.cursor.end)
}

/// Select a byte range of the console's text in the editor (without focusing it).
fn find_select(ctx: &egui::Context, editor_id: Id, c: &mut Console, range: std::ops::Range<usize>) {
    let start = c.sql[..range.start].chars().count();
    let end = start + c.sql[range].chars().count();
    c.cursor = start..end;
    if let Some(mut state) = TextEdit::load_state(ctx, editor_id) {
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(start), egui::text::CCursor::new(end))));
        state.store(ctx, editor_id);
    }
    c.find.reveal = true;
}

/// Select the next (or previous) match from the editor's selection, wrapping around.
fn find_step(ctx: &egui::Context, editor_id: Id, c: &mut Console, forward: bool) {
    let matches = c.find.matches(&c.sql);
    let at = cursor_bytes(c);
    let i = if forward { rgrip_core::find::next_from(&matches, at.end) } else { rgrip_core::find::prev_before(&matches, at.start) };
    if let Some(i) = i {
        find_select(ctx, editor_id, c, matches[i].clone());
    }
}

fn find_replace_one(ctx: &egui::Context, editor_id: Id, c: &mut Console) {
    let selected = cursor_bytes(c);
    if c.find.matches(&c.sql).contains(&selected) {
        let with = c.find.replace.clone();
        c.cursor = completion::replace_text(ctx, editor_id, &mut c.sql, selected, &with);
        c.popup = None;
    }
    find_step(ctx, editor_id, c, true);
}

/// One edit to the text, so one undo step, like formatting.
fn find_replace_all(ctx: &egui::Context, editor_id: Id, c: &mut Console) {
    let matches = c.find.matches(&c.sql);
    let Some(first) = matches.first().map(|m| m.start) else { return };
    c.sql = rgrip_core::find::replace_all(&c.sql, &matches, &c.find.replace);
    c.popup = None;
    // The text before the first match is unchanged, so this is still a char boundary.
    find_select(ctx, editor_id, c, first..first);
}

/// The find bar above the editor: query, count, next/previous, options and (⌥⌘F) replace.
fn find_bar_ui(ui: &mut Ui, tab_id: TabId, c: &mut Console, editor_id: Id, keys: &FindKeys) {
    let ctx = ui.ctx().clone();
    let (find_id, replace_id) = (find_field_id(tab_id, FindField::Find), find_field_id(tab_id, FindField::Replace));
    let editable = c.running.is_none();
    let (mut step, mut query_changed) = (keys.step, false);
    let (mut replace_one, mut replace_every) = (keys.replace_one, keys.replace_all);
    if let Some(field) = c.find.focus.take() {
        ui.memory_mut(|m| m.request_focus(if field == FindField::Find { find_id } else { replace_id }));
        // Typing replaces the previous query, as in a search field.
        if field == FindField::Find
            && let Some(mut state) = TextEdit::load_state(&ctx, find_id)
        {
            let len = c.find.query.chars().count();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(len))));
            state.store(&ctx, find_id);
        }
    }
    let matches = c.find.matches(&c.sql);
    let current = matches.iter().position(|m| *m == cursor_bytes(c));
    Frame::new().fill(theme::SURFACE()).inner_margin(Margin { left: 12, right: 14, top: 6, bottom: 6 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            let arrow = if c.find.show_replace { "▾" } else { "▸" };
            if ghost_button(ui, arrow).on_hover_text("Replace  ⌥⌘F").clicked() {
                c.find.show_replace = !c.find.show_replace;
            }
            let out = TextEdit::singleline(&mut c.find.query).id(find_id).hint_text("Find").desired_width(220.0).show(ui);
            query_changed |= out.response.changed();
            let (text, color) = match (c.find.query.is_empty(), current) {
                (true, _) => (String::new(), theme::MUTED()),
                (false, _) if matches.is_empty() => ("No results".to_string(), theme::ERROR()),
                (false, Some(i)) => (format!("{} of {}", i + 1, matches.len()), theme::MUTED()),
                (false, None) => (plural(matches.len(), "result"), theme::MUTED()),
            };
            ui.add_sized([72.0, 18.0], egui::Label::new(RichText::new(text).size(12.5).color(color)));
            if ghost_button(ui, "↑").on_hover_text("Previous  ⇧↩  ⇧⌘G").clicked() {
                step = Some(false);
            }
            if ghost_button(ui, "↓").on_hover_text("Next  ↩  ⌘G").clicked() {
                step = Some(true);
            }
            if ui.selectable_label(c.find.case_sensitive, "Aa").on_hover_text("Match case").clicked() {
                c.find.case_sensitive = !c.find.case_sensitive;
                query_changed = true;
            }
            if ui.selectable_label(c.find.whole_word, "W").on_hover_text("Whole word").clicked() {
                c.find.whole_word = !c.find.whole_word;
                query_changed = true;
            }
            if ghost_button(ui, "✕").on_hover_text("Close  Esc").clicked() {
                c.find.open = false;
            }
        });
        if c.find.show_replace {
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                TextEdit::singleline(&mut c.find.replace).id(replace_id).hint_text("Replace").desired_width(220.0).show(ui);
                ui.add_enabled_ui(editable, |ui| {
                    if ui.button("Replace").on_hover_text("↩").clicked() {
                        replace_one = true;
                    }
                    if ui.button("Replace all").on_hover_text("⌥⌘↩").clicked() {
                        replace_every = true;
                    }
                });
            });
        }
    });
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, theme::BORDER()));

    if query_changed {
        // Incremental: the first match from where the cursor is.
        let at = cursor_bytes(c);
        let matches = c.find.matches(&c.sql);
        if let Some(i) = rgrip_core::find::next_from(&matches, at.start) {
            find_select(&ctx, editor_id, c, matches[i].clone());
        }
    }
    if let Some(forward) = step {
        find_step(&ctx, editor_id, c, forward);
    }
    if editable && replace_one {
        find_replace_one(&ctx, editor_id, c);
    }
    if editable && replace_every {
        find_replace_all(&ctx, editor_id, c);
    }
    if keys.close {
        // The current match stays selected.
        c.find.open = false;
        ui.memory_mut(|m| m.request_focus(editor_id));
    }
}

/// Shade the matches over the text (the current one stronger); translucent, so the text stays readable.
fn paint_matches(ui: &Ui, out: &egui::text_edit::TextEditOutput, matches: &[std::ops::Range<usize>], current: Option<usize>, reveal: bool) {
    use egui::text::CCursor;
    let text = out.galley.text();
    let offset = out.galley_pos.to_vec2();
    let clip = ui.clip_rect();
    let (mut byte, mut chars) = (0, 0);
    for (n, m) in matches.iter().enumerate() {
        if m.end > text.len() {
            break;
        }
        let start = chars + text[byte..m.start].chars().count();
        let end = start + text[m.clone()].chars().count();
        (byte, chars) = (m.end, end);
        let from = out.galley.pos_from_cursor(CCursor::new(start)).translate(offset);
        let to = out.galley.pos_from_cursor(CCursor::new(end)).translate(offset);
        let color = if current == Some(n) { theme::ACCENT().gamma_multiply(0.45) } else { theme::ACCENT().gamma_multiply(0.2) };
        // A match that wraps is shaded as the end of one row and the start of the next.
        let rects = if (from.min.y - to.min.y).abs() < 1.0 {
            vec![egui::Rect::from_min_max(from.min, egui::pos2(to.min.x.max(from.min.x + 2.0), from.max.y))]
        } else {
            let right = out.galley_pos.x + out.galley.rect.right();
            vec![
                egui::Rect::from_min_max(from.min, egui::pos2(right, from.max.y)),
                egui::Rect::from_min_max(egui::pos2(out.galley_pos.x, to.min.y), egui::pos2(to.min.x, to.max.y)),
            ]
        };
        if reveal && current == Some(n) {
            ui.scroll_to_rect(rects[0].expand(24.0), None);
        }
        for r in rects.into_iter().filter(|r| clip.intersects(*r)) {
            ui.painter().rect_filled(r, 2.0, color);
        }
    }
}

/// `dirty`: the console differs from its saved query (autosave pending).
pub(super) fn console_ui(
    ui: &mut Ui,
    tab_id: TabId,
    c: &mut Console,
    catalog: &Catalog,
    schemas: &[String],
    databases: &[String],
    default_db: &str,
    dirty: bool,
    read_only: bool,
) -> Option<TabAction> {
    let mut action = None;
    let editor_id = Id::new(("editor", tab_id));
    let keys = completion::handle_keys(ui, &mut c.popup);
    if keys.dismissed {
        ui.memory_mut(|m| m.request_focus(editor_id));
    }
    if let Some(i) = keys.accept {
        if let Some(p) = c.popup.take() {
            c.cursor = completion::accept(ui.ctx(), editor_id, &mut c.sql, &p.completion, i);
            // Picking /new opens the editor right away.
            let byte = sql::char_to_byte(&c.sql, c.cursor.start);
            if let Some((range, line)) = rgrip_core::commands::command_line_at(&c.sql, byte)
                && let Some(name) = rgrip_core::commands::new_command(line)
            {
                let name = name.to_string();
                c.cursor = completion::replace_text(ui.ctx(), editor_id, &mut c.sql, range, "");
                return Some(TabAction::NewCommand(name));
            }
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
            if let Some(name) = rgrip_core::commands::new_command(line).filter(|_| tab) {
                let name = name.to_string();
                c.cursor = completion::replace_text(ui.ctx(), editor_id, &mut c.sql, range, "");
                return Some(TabAction::NewCommand(name));
            }
            match rgrip_core::commands::expand(line, catalog, c.schema_or_default(schemas)) {
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
    let find_keys = take_find_keys(ui, tab_id, c);
    // ⌘/ (and ⌘?, the same key with shift) toggles `--` on the selected lines.
    let comment = ui.input_mut(|i| {
        i.consume_key(Modifiers::COMMAND, Key::Slash)
            || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Slash)
            || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Questionmark)
            || i.consume_key(Modifiers::COMMAND, Key::Questionmark)
    });
    // Quotes and brackets close themselves; backspace in an empty pair takes both.
    if c.running.is_none() && ui.memory(|m| m.has_focus(editor_id)) {
        let sel = TextEdit::load_state(ui.ctx(), editor_id)
            .and_then(|s| s.cursor.char_range())
            .map_or(c.cursor.clone(), |r| r.as_sorted_char_range());
        let sel = sql::char_to_byte(&c.sql, sel.start)..sql::char_to_byte(&c.sql, sel.end);
        let edit = ui.input_mut(|i| {
            let mut edit = None;
            i.events.retain(|e| {
                if edit.is_some() {
                    return true;
                }
                match e {
                    egui::Event::Text(t) if t.chars().count() == 1 => {
                        let ch = t.chars().next().unwrap();
                        edit = rgrip_core::pairs::type_char(&c.sql, sel.clone(), ch);
                    }
                    egui::Event::Key { key: Key::Backspace, pressed: true, modifiers, .. } if modifiers.is_none() && sel.start == sel.end => {
                        edit = rgrip_core::pairs::backspace(&c.sql, sel.start).map(|(t, at)| (t, at..at));
                    }
                    _ => {}
                }
                edit.is_none()
            });
            edit
        });
        if let Some((text, sel)) = edit {
            c.sql = text;
            c.cursor = c.sql[..sel.start].chars().count()..c.sql[..sel.end].chars().count();
            if let Some(mut state) = TextEdit::load_state(ui.ctx(), editor_id) {
                let range = egui::text::CCursorRange::two(egui::text::CCursor::new(c.cursor.start), egui::text::CCursor::new(c.cursor.end));
                state.cursor.set_char_range(Some(range));
                state.store(ui.ctx(), editor_id);
            }
        }
    }
    if comment && c.running.is_none() {
        let sel = sql::char_to_byte(&c.sql, c.cursor.start)..sql::char_to_byte(&c.sql, c.cursor.end);
        let (text, sel) = rgrip_core::format::toggle_comment(&c.sql, sel);
        c.sql = text;
        c.cursor = c.sql[..sel.start].chars().count()..c.sql[..sel.end].chars().count();
        if let Some(mut state) = TextEdit::load_state(ui.ctx(), editor_id) {
            let range = egui::text::CCursorRange::two(egui::text::CCursor::new(c.cursor.start), egui::text::CCursor::new(c.cursor.end));
            state.cursor.set_char_range(Some(range));
            state.store(ui.ctx(), editor_id);
        }
        c.popup = None;
        ui.memory_mut(|m| m.request_focus(editor_id));
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Enter)) {
        action = Some(TabAction::Run { all: true });
    } else if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
        action = Some(TabAction::Run { all: false });
    } else if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::E)) {
        action = Some(TabAction::Explain { analyze: true });
    } else if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::E)) {
        action = Some(TabAction::Explain { analyze: false });
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
            if ghost_button(ui, "Explain").on_hover_text("Show the plan of the statement at cursor, without running it  ⌘E").clicked() {
                action = Some(TabAction::Explain { analyze: false });
            }
            if ghost_button(ui, "Explain analyze").on_hover_text("Run the statement and show the plan with real timings  ⇧⌘E\nA statement that writes runs in a transaction that is rolled back.").clicked() {
                action = Some(TabAction::Explain { analyze: true });
            }
            if ghost_button(ui, "Format").on_hover_text("Format selection or all  ⇧⌘F").clicked() {
                format = true;
            }
            if ghost_button(ui, "History").on_hover_text("Statements run on this connection").clicked() {
                action = Some(TabAction::History);
            }
        }
        if read_only {
            read_only_badge(ui);
        }
        // Right side: save state, then the schema (stays visible while running).
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            match (&c.saved_id, dirty) {
                // Saved queries autosave; this just shows where things are.
                (Some(_), false) => {
                    ui.label(RichText::new("Saved").size(12.5).color(theme::FAINT())).on_hover_text("Changes save automatically");
                }
                (Some(_), true) => {
                    ui.label(RichText::new("Saving…").size(12.5).color(theme::MUTED())).on_hover_text("Saves a moment after you stop typing  ⌘S now");
                }
                (None, _) => {
                    if ghost_button(ui, "Save as…").on_hover_text("⌘S").clicked() {
                        action = Some(TabAction::Save);
                    }
                }
            }
            ui.add_space(8.0);
            ui.add_enabled_ui(c.running.is_none(), |ui| schema_picker(ui, tab_id, c, schemas));
            // Right-to-left: the database ends up left of the schema.
            ui.add_enabled_ui(c.running.is_none(), |ui| database_picker(ui, tab_id, c, databases, default_db));
        });
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

    if find_keys.open {
        // Start from the selected text, unless it's the match already being looked at.
        let selected = &c.sql[cursor_bytes(c)];
        if !selected.is_empty() && !selected.contains('\n') && selected.to_lowercase() != c.find.query.to_lowercase() {
            c.find.query = selected.to_string();
        }
        c.find.open = true;
        c.find.show_replace |= find_keys.with_replace;
        c.find.focus = Some(if find_keys.with_replace && !c.find.query.is_empty() { FindField::Replace } else { FindField::Find });
        c.popup = None;
    }
    if c.find.open {
        find_bar_ui(ui, tab_id, c, editor_id, &find_keys);
    } else if let Some(forward) = find_keys.step.filter(|_| !c.find.query.is_empty()) {
        // ⌘G still works after the bar is closed.
        find_step(ui.ctx(), editor_id, c, forward);
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
                    // Khmer is drawn shaped over the laid-out text (egui doesn't shape it).
                    let mut khmer = Vec::new();
                    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                        let mut job = highlight::sql(buf.as_str(), TextStyle::Monospace.resolve(ui.style()));
                        job.wrap.max_width = wrap;
                        khmer = crate::shaping::hide_runs(ui, &mut job);
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
                    crate::shaping::paint_runs(ui, &out.galley, out.galley_pos, &khmer);
                    paint_run_marker(ui, &out);
                    paint_line_numbers(ui, &out);
                    let previous = c.cursor.clone();
                    if let Some(r) = out.cursor_range {
                        c.cursor = r.as_sorted_char_range();
                    }
                    if out.response.changed() {
                        c.last_edit = Some(Instant::now());
                    }
                    if c.find.open && !c.find.query.is_empty() {
                        let matches = c.find.matches(&c.sql);
                        let current = matches.iter().position(|m| *m == cursor_bytes(c));
                        paint_matches(ui, &out, &matches, current, std::mem::take(&mut c.find.reveal));
                    }
                    update_popup(c, catalog, schemas, out.response.changed(), keys.force, previous);
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

    // Rows of the result can be edited and deleted in the one table it reads
    // (by its key); columns that aren't read from it stay read-only.
    let target = if read_only || c.running.is_some() { None } else { console_edit_target(c, catalog) };
    let deletable = target.is_some();
    let (kinds, locked): (Vec<_>, Vec<bool>) = match (&target, &c.sources) {
        (Some((schema, table, _)), Some((origins, tables))) => {
            let columns = tables.iter().find(|(s, t, _, _)| s == schema && t == table).map_or(&[][..], |(_, _, cols, _)| cols.as_slice());
            origins
                .iter()
                .map(|o| match o {
                    Some((_, _, name)) => (columns.iter().find(|c| c.name == *name).and_then(|c| rgrip_core::datetime::kind_of(&c.data_type)), false),
                    None => (None, true),
                })
                .unzip()
        }
        _ => Default::default(),
    };
    let Console { running, output, view, grid, paged, offset, map, last_sql, order, relations, probe, related, total, area, changes, editor, plan, .. } = c;
    // Only a paged (wrappable) statement can be filtered to an area.
    let area_filter = paged.is_some().then_some(area.as_str());
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

    if *view == View::Plan {
        match plan {
            Some(p) => {
                toolbar(ui, |ui| {
                    segment(ui, true, "Plan");
                    if let Some(Ok(out)) = output {
                        let n = out.statements.iter().filter(|s| s.result.is_some()).count();
                        if n > 0 && segment(ui, false, "Result") {
                            *view = View::Result(n - 1);
                        }
                        if segment(ui, false, "Output") {
                            *view = View::Output;
                        }
                    }
                    ui.add_space(8.0);
                    if segment(ui, p.raw, "Raw") {
                        p.raw = !p.raw;
                    }
                    if ghost_button(ui, "Copy").on_hover_text("Copy the plan as JSON").clicked() {
                        ui.ctx().copy_text(p.plan.raw.clone());
                    }
                });
                if *view == View::Plan {
                    plan_view::show(ui, Id::new(("plan", tab_id)), p);
                    return action;
                }
            }
            None => *view = View::Output,
        }
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
                        *grid = GridState::fresh(grid);
                    }
                    if !rs.geometry_columns().is_empty() {
                        // The grid's selection is per result set.
                        let other = !matches!(*view, View::Result(m) | View::Map(m) | View::Split(m) if m == n);
                        if segment(ui, *view == View::Map(n), if sets.len() > 1 { "Map ↑" } else { "Map" }) {
                            if other {
                                *grid = GridState::fresh(grid);
                            }
                            map.opened(!grid.selection.is_empty());
                            *view = View::Map(n);
                        }
                        if segment(ui, *view == View::Split(n), if sets.len() > 1 { "Split ↑" } else { "Split" })
                        {
                            if other {
                                *grid = GridState::fresh(grid);
                            }
                            map.opened(!grid.selection.is_empty());
                            *view = View::Split(n);
                        }
                    }
                }
                if segment(ui, *view == View::Output, "Output") {
                    *view = View::Output;
                }
                if plan.is_some() && segment(ui, false, "Plan") {
                    *view = View::Plan;
                }
                if !changes.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(plural(changes.len(), "change")).small().color(theme::ACCENT()));
                    // Not "Save": that saves the query (⌘S).
                    let save = Button::new(RichText::new("Save rows").color(theme::BG())).fill(theme::ACCENT());
                    if ui.add(save).on_hover_text("Write the changes to the table in one transaction").clicked() {
                        action = Some(TabAction::SaveEdits);
                    }
                    if ui.button("Discard").clicked() {
                        changes.clear();
                        *editor = None;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ms = out.elapsed.as_secs_f64() * 1000.0;
                    let result = match *view {
                        View::Result(n) | View::Map(n) | View::Split(n) => sets.get(n).copied(),
                        View::Output | View::Plan => None,
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
                            let more = match (rs.truncated, total) {
                                (true, Some(total)) => format!(" of {}", group_thousands(*total)),
                                (true, None) => " +".to_string(),
                                _ => String::new(),
                            };
                            format!("rows {}–{}{more}  ·  {ms:.0} ms", group_thousands(from as i64), group_thousands(to as i64))
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
                    let menu_rows: Vec<usize> = grid.selection.iter().copied().collect();
                    let counts = probe.as_ref().map(|p| p.best(&menu_rows, relations.len())).unwrap_or_default();
                    let source = grid::Source { table: "query_result", sql: Some(last_sql.as_str()), relations, counts: &counts, deletable, filterable: false };
                    let sort = order.as_ref().filter(|(m, _, _)| *m == n).map(|(_, c, asc)| grid::Sort { column: c, ascending: *asc });
                    // Only the first result has sources (a single statement).
                    let editing = (deletable && n == 0).then_some(grid::Editing { changes, editor, kinds: &kinds, locked: &locked, added_from: usize::MAX, existing_locked: false, insertable: false });
                    match grid::show(ui, Id::new(("grid", tab_id, n)), sets[n], grid, Some(sort.unwrap_or(grid::Sort { column: "", ascending: true })), row_offset, &source, editing) {
                        Some(GridEvent::Change { row, col, value }) => record_change(changes, sets[n], row, col, value),
                        Some(GridEvent::Revert { row, col }) => {
                            changes.remove(&(row, col));
                        }
                        Some(grid::GridEvent::HeaderClicked(i)) => action = Some(TabAction::SortResult(n, i)),
                        Some(GridEvent::Follow { relation, rows }) => {
                            let only = probe.as_ref().map(|p| p.matched(&rows, relation)).unwrap_or_default();
                            action = relations.get(relation).map(|r| follow(r, sets[n], &rows, &only));
                        }
                        Some(GridEvent::RowsMenu { rows }) => action = probe_action(relations, sets[n], rows, false),
                        Some(GridEvent::Lookup { relation }) => action = relations.get(relation).map(lookup),
                        Some(GridEvent::Delete { rows }) => action = Some(TabAction::DeleteRows(rows)),
                        _ => {}
                    }
                }
                View::Map(n) if n < sets.len() => {
                    let cols = sets[n].geometry_columns().to_vec();
                    if cols.is_empty() {
                        centered(ui, |ui| muted(ui, "No geometry columns in this result"));
                    } else {
                        let link = crate::map::Link { selection: &mut grid.selection, reveal: &mut grid.reveal, layer: None, area_filter };
                        let out = map.show(ui, sets[n], &cols, paged.is_some(), link);
                        if let Some(col) = out.reproject {
                            action = Some(TabAction::Reproject(col));
                        } else if let Some(a) = out.area {
                            action = Some(area_action(a));
                        }
                    }
                }
                View::Split(n) if n < sets.len() => {
                    let cols = sets[n].geometry_columns().to_vec();
                    let row_offset = if paged.is_some() { *offset } else { 0 };
                    let menu_rows: Vec<usize> = grid.selection.iter().copied().collect();
                    let counts = probe.as_ref().map(|p| p.best(&menu_rows, relations.len())).unwrap_or_default();
                    let source = grid::Source { table: "query_result", sql: Some(last_sql.as_str()), relations, counts: &counts, deletable, filterable: false };
                    let can_reproject = paged.is_some();
                    let mut sort_click = None;
                    let reproject = &mut action;
                    split_view(
                        ui,
                        Id::new(("split", tab_id)),
                        grid,
                        |ui, g| {
                            let link = crate::map::Link { selection: &mut g.selection, reveal: &mut g.reveal, layer: None, area_filter };
                            let out = map.show(ui, sets[n], &cols, can_reproject, link);
                            if let Some(col) = out.reproject {
                                *reproject = Some(TabAction::Reproject(col));
                            } else if let Some(a) = out.area {
                                *reproject = Some(area_action(a));
                            }
                        },
                        |ui, g| {
                            let sort = order.as_ref().filter(|(m, _, _)| *m == n).map(|(_, c, asc)| grid::Sort { column: c, ascending: *asc });
                            let editing = (deletable && n == 0).then_some(grid::Editing { changes, editor: &mut *editor, kinds: &kinds, locked: &locked, added_from: usize::MAX, existing_locked: false, insertable: false });
                            match grid::show(ui, Id::new(("grid", tab_id, n)), sets[n], g, Some(sort.unwrap_or(grid::Sort { column: "", ascending: true })), row_offset, &source, editing) {
                                Some(GridEvent::Change { row, col, value }) => record_change(changes, sets[n], row, col, value),
                                Some(GridEvent::Revert { row, col }) => {
                                    changes.remove(&(row, col));
                                }
                                Some(grid::GridEvent::HeaderClicked(i)) => sort_click = Some(TabAction::SortResult(n, i)),
                                Some(GridEvent::Follow { relation, rows }) => {
                                    let only = probe.as_ref().map(|p| p.matched(&rows, relation)).unwrap_or_default();
                                    sort_click = relations.get(relation).map(|r| follow(r, sets[n], &rows, &only));
                                }
                                Some(GridEvent::RowsMenu { rows }) => sort_click = probe_action(relations, sets[n], rows, false),
                                Some(GridEvent::Lookup { relation }) => sort_click = relations.get(relation).map(lookup),
                                Some(GridEvent::Delete { rows }) => sort_click = Some(TabAction::DeleteRows(rows)),
                                _ => {}
                            }
                        },
                    );
                    if sort_click.is_some() {
                        action = sort_click;
                    }
                }
                _ => output_log(ui, out),
            }
        }
    }
    if let Some(a) = related_ui(ui, tab_id, related) {
        action = Some(a);
    }
    action
}

/// Toolbar combo for the console's database (the connection's own until
/// another is picked). Switching resets the schema to the new database's default.
fn database_picker(ui: &mut Ui, tab_id: TabId, c: &mut Console, databases: &[String], default_db: &str) {
    const FILTER_FROM: usize = 12;
    let current = c.database.as_deref().unwrap_or(default_db).to_string();
    let filter_id = Id::new(("database-filter", tab_id));
    let mut picked = None;
    egui::ComboBox::from_id_salt(("database", tab_id))
        .selected_text(current.clone())
        .width(110.0)
        .show_ui(ui, |ui| {
            let mut filter = String::new();
            if databases.len() > FILTER_FROM {
                filter = ui.data_mut(|d| d.get_temp::<String>(filter_id)).unwrap_or_default();
                let r = ui.add(TextEdit::singleline(&mut filter).hint_text("Filter…").desired_width(140.0));
                if r.changed() {
                    ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
                }
            }
            let needle = filter.to_lowercase();
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for d in databases.iter().filter(|d| d.to_lowercase().contains(&needle)) {
                    if ui.selectable_label(*d == current, d).clicked() {
                        picked = Some(d.clone());
                    }
                }
            });
        })
        .response
        .on_hover_text("Database");
    if let Some(d) = picked.filter(|d| *d != current) {
        c.database = (d != default_db).then_some(d);
        c.schema = None;
        c.more_schemas.clear();
        c.popup = None;
    }
}

/// Toolbar combo for the console's schema. With many schemas the list gets a
/// filter field.
/// Schemas for unqualified names. Tick several to search them all, in the
/// order ticked; the first is the main one (autocomplete, slash commands).
fn schema_picker(ui: &mut Ui, tab_id: TabId, c: &mut Console, schemas: &[String]) {
    const FILTER_FROM: usize = 12;
    let list = c.search_list(schemas);
    let title = match list.len() {
        1 => list[0].clone(),
        n => format!("{} +{}", list[0], n - 1),
    };
    let filter_id = Id::new(("schema-filter", tab_id));
    let mut toggled = None;
    egui::ComboBox::from_id_salt(("schema", tab_id))
        .selected_text(title)
        .width(110.0)
        // Ticking several in a row: stay open until a click outside.
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            menu_layout(ui);
            let mut filter = String::new();
            if schemas.len() > FILTER_FROM {
                filter = ui.data_mut(|d| d.get_temp::<String>(filter_id)).unwrap_or_default();
                let r = ui.add(TextEdit::singleline(&mut filter).hint_text("Filter…").desired_width(160.0));
                if r.changed() {
                    ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
                }
            }
            ui.label(RichText::new("Searched in this order, then public").size(11.5).color(theme::FAINT()));
            let needle = filter.to_lowercase();
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for s in schemas.iter().filter(|s| s.to_lowercase().contains(&needle)) {
                    let pos = list.iter().position(|x| x == s);
                    let mut on = pos.is_some();
                    let label = match pos {
                        Some(i) if list.len() > 1 => format!("{s}  ·{}", i + 1),
                        _ => s.clone(),
                    };
                    // Keep at least one schema ticked.
                    let lone = on && list.len() == 1;
                    if ui.add_enabled(!lone, egui::Checkbox::new(&mut on, label)).clicked() {
                        toggled = Some(s.clone());
                    }
                }
            });
        })
        .response
        .on_hover_text(list.join(", "));
    if let Some(s) = toggled {
        let mut list = list;
        match list.iter().position(|x| *x == s) {
            Some(i) => {
                list.remove(i);
            }
            None => list.push(s),
        }
        if !list.is_empty() {
            c.schema = Some(list.remove(0));
            c.more_schemas = list;
            c.popup = None;
        }
    }
}

/// Open, refresh or close the completion popup after the editor ran.
fn update_popup(c: &mut Console, catalog: &Catalog, schemas: &[String], changed: bool, force: bool, previous: std::ops::Range<usize>) {
    if c.cursor.start != c.cursor.end {
        c.popup = None;
        return;
    }
    let byte = sql::char_to_byte(&c.sql, c.cursor.start);
    let before = c.sql[..byte].chars().next_back();
    let in_word = before.is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '.' | '"' | '/'));
    if force || (changed && in_word) {
        c.popup = complete::complete(&c.sql, byte, catalog, c.schema_or_default(schemas), force).map(Popup::new);
    } else if changed || previous != c.cursor {
        // Typed a separator, or moved the cursor away.
        c.popup = None;
    }
}

/// Export menu for a result grid: copy or save the rows shown, or (when
/// `can_export_all`) write every row of the underlying query to a file.
pub(super) fn export_menu(ui: &mut Ui, rs: &ResultSet, table: &str, can_export_all: bool) -> Option<TabAction> {
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
                let label = if *f == export::Format::Svg { "Image (PNG)" } else { f.label() };
                if ui.button(label).clicked() {
                    if *f == export::Format::Svg {
                        crate::snapshot::copy_table(ui.ctx(), rs, table);
                    } else {
                        ui.ctx().copy_text(export::export(rs, *f, table));
                    }
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

/// What Run executes for a byte cursor with no selection: a command line,
/// else the statement under the cursor.
pub(super) fn run_range(sql: &str, at: usize) -> Option<std::ops::Range<usize>> {
    match rgrip_core::commands::command_line_at(sql, at) {
        // A command line is its own statement (it has no `;`).
        Some((r, _)) => Some(r),
        None => sql::statement_at(sql, at),
    }
}

/// A bar in the gutter beside the lines Run will execute.
fn paint_run_marker(ui: &Ui, out: &egui::text_edit::TextEditOutput) {
    let Some(r) = out.cursor_range else { return };
    if r.primary.index != r.secondary.index {
        return;
    }
    let text = out.galley.text();
    let Some(range) = run_range(text, sql::char_to_byte(text, r.primary.index)) else { return };
    let first = text[..range.start].matches('\n').count() + 1;
    let last = first + text[range].matches('\n').count();
    let mut line = 1;
    let (mut top, mut bottom) = (None, None);
    for row in &out.galley.rows {
        if (first..=last).contains(&line) {
            let rect = row.rect().translate(out.galley_pos.to_vec2());
            top.get_or_insert(rect.top());
            bottom = Some(rect.bottom());
        }
        if row.row.ends_with_newline {
            line += 1;
        }
    }
    if let (Some(top), Some(bottom)) = (top, bottom) {
        let x = out.galley_pos.x - 8.0;
        ui.painter().rect_filled(egui::Rect::from_x_y_ranges(x..=x + 2.0, top..=bottom), 1.0, theme::ACCENT());
    }
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


#[cfg(test)]
mod tests {
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
                keys: Vec::new(),
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
                    console_ui(ui, 1, console, catalog, &[], &[], "db", false, false);
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
    fn find_bar_finds_steps_and_replaces() {
        let mut h = Harness::new();
        h.type_text("select a from t; select b");
        h.key(Key::F, Modifiers::COMMAND);
        assert!(h.console.find.open);
        // Typing goes to the find bar, not the editor, and lands on the first match.
        h.type_text("SELECT");
        assert_eq!(h.console.sql, "select a from t; select b");
        assert_eq!(h.console.cursor, 0..6);

        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.console.cursor, 17..23);
        // Wraps around, and ⇧↩ goes back.
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.console.cursor, 0..6);
        h.key(Key::Enter, Modifiers::SHIFT);
        assert_eq!(h.console.cursor, 17..23);
        h.key(Key::G, Modifiers::COMMAND);
        assert_eq!(h.console.cursor, 0..6);
        h.key(Key::G, Modifiers::COMMAND | Modifiers::SHIFT);
        assert_eq!(h.console.cursor, 17..23);

        // Replace all, from the replace field: ⌥⌘↩.
        h.key(Key::F, Modifiers::COMMAND | Modifiers::ALT);
        assert!(h.console.find.show_replace);
        h.type_text("pick");
        assert_eq!(h.console.find.replace, "pick");
        h.key(Key::Enter, Modifiers::COMMAND | Modifiers::ALT);
        assert_eq!(h.console.sql, "pick a from t; pick b");

        // Esc closes the bar and gives the editor back.
        h.key(Key::Escape, Modifiers::NONE);
        assert!(!h.console.find.open);
        h.type_text("!");
        assert!(h.console.sql.contains('!'));
    }

    #[test]
    fn find_bar_replaces_one_at_a_time_and_keeps_khmer_intact() {
        let mut h = Harness::new();
        h.console.sql = "ក = 'សួស្តី'; ក".to_string();
        h.frame(vec![]);
        h.key(Key::F, Modifiers::COMMAND | Modifiers::ALT);
        h.type_text("ក");
        assert_eq!(h.console.cursor, 0..1);
        h.ctx.memory_mut(|m| m.request_focus(find_field_id(1, FindField::Replace)));
        h.frame(vec![]);
        h.type_text("x");
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.console.sql, "x = 'សួស្តី'; ក");
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.console.sql, "x = 'សួស្តី'; x");
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

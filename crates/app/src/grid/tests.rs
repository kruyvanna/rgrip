use super::*;

/// Run the grid one frame with `events`; returns the selection.
fn frame(ctx: &egui::Context, rs: &ResultSet, state: &mut GridState, events: Vec<egui::Event>) {
    frame_mods(ctx, rs, state, events, egui::Modifiers::NONE);
}

fn frame_mods(ctx: &egui::Context, rs: &ResultSet, state: &mut GridState, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        events,
        modifiers,
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            show(ui, Id::new("g"), rs, state, None, 0, &Source { table: "t", sql: None, relations: &[], counts: &[], deletable: false, filterable: false }, None);
        });
    });
}

fn click(ctx: &egui::Context, rs: &ResultSet, state: &mut GridState, row: usize, modifiers: egui::Modifiers) {
    // Header ~30pt, rows ~25pt: aim at the middle of `row` in the first column.
    let h = ctx.style().text_styles[&TextStyle::Monospace].size;
    let row_h = ctx.fonts_mut(|f| f.row_height(&egui::FontId::monospace(h))) + 10.0;
    let pos = egui::pos2(120.0, 8.0 + row_h + 4.0 + row_h * (row as f32 + 0.5));
    let press = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    frame_mods(ctx, rs, state, vec![egui::Event::PointerMoved(pos)], modifiers);
    frame_mods(ctx, rs, state, vec![press(true)], modifiers);
    frame_mods(ctx, rs, state, vec![press(false)], modifiers);
}

#[test]
fn multi_row_selection() {
    let rs = ResultSet {
        columns: vec!["id".into(), "name".into()],
        rows: (0..10).map(|i| vec![Some(i.to_string()), Some(format!("row {i}"))]).collect(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut st = GridState::default();
    frame(&ctx, &rs, &mut st, vec![]);
    click(&ctx, &rs, &mut st, 1, egui::Modifiers::NONE);
    assert_eq!(st.selection.iter().copied().collect::<Vec<_>>(), [1]);
    click(&ctx, &rs, &mut st, 4, egui::Modifiers::SHIFT);
    assert_eq!(st.selection.iter().copied().collect::<Vec<_>>(), [1, 2, 3, 4]);
    click(&ctx, &rs, &mut st, 2, egui::Modifiers::COMMAND);
    assert_eq!(st.selection.iter().copied().collect::<Vec<_>>(), [1, 3, 4]);
    click(&ctx, &rs, &mut st, 6, egui::Modifiers::NONE);
    assert_eq!(st.selection.iter().copied().collect::<Vec<_>>(), [6]);
    let cmd_a = egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
    frame_mods(&ctx, &rs, &mut st, vec![cmd_a], egui::Modifiers::COMMAND);
    assert_eq!(st.selection.len(), 10);
    assert_eq!(rows_tsv(&rs, &[1, 3], true), "id\tname\n1\trow 1\n3\trow 3");
}

#[test]
fn drag_header_to_reorder() {
    let rs = ResultSet {
        columns: vec!["id".into(), "name".into(), "note".into()],
        rows: (0..3).map(|i| vec![Some(i.to_string()), Some(format!("row {i}")), None]).collect(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut st = GridState::default();
    frame(&ctx, &rs, &mut st, vec![]);
    // Grab `id` (the first header, right of the row numbers) and drop it past `name`.
    let y = 20.0;
    let press = |x, pressed| egui::Event::PointerButton { pos: egui::pos2(x, y), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &rs, &mut st, vec![egui::Event::PointerMoved(egui::pos2(60.0, y))]);
    frame(&ctx, &rs, &mut st, vec![press(60.0, true)]);
    for x in [80.0, 120.0, 150.0] {
        frame(&ctx, &rs, &mut st, vec![egui::Event::PointerMoved(egui::pos2(x, y))]);
    }
    frame(&ctx, &rs, &mut st, vec![press(150.0, false)]);
    assert_eq!(st.order, ["name", "id", "note"]);
    assert_eq!(display_order(&rs.columns, &st.order), [1, 0, 2]);
    // Kept for the next result.
    assert_eq!(GridState::fresh(&st).order, st.order);
}

#[test]
fn order_helpers() {
    let cols: Vec<String> = ["a", "b", "c"].map(String::from).into();
    assert_eq!(display_order(&cols, &["c".into(), "gone".into()]), [2, 0, 1]);
    assert_eq!(move_to(&[0, 1, 2], &0, 2), [1, 2, 0]);
    assert_eq!(move_to(&[0, 1, 2], &2, 0), [2, 0, 1]);
}

/// One frame with inline editing on; returns the grid's event.
fn frame_edit(
    ctx: &egui::Context,
    rs: &ResultSet,
    state: &mut GridState,
    changes: &BTreeMap<(usize, usize), Option<String>>,
    editor: &mut Option<CellEditor>,
    events: Vec<egui::Event>,
    time: f64,
    kinds: &[Option<TemporalKind>],
) -> Option<GridEvent> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        events,
        time: Some(time),
        ..Default::default()
    };
    let mut out = None;
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let edit = Some(Editing { changes, editor: &mut *editor, kinds, locked: &[], added_from: usize::MAX, existing_locked: false, insertable: false });
            out = show(ui, Id::new("g"), rs, state, None, 0, &Source { table: "t", sql: None, relations: &[], counts: &[], deletable: false, filterable: false }, edit);
        });
    });
    out
}

#[test]
fn inline_edit() {
    let rs = ResultSet {
        columns: vec!["id".into(), "name".into()],
        rows: (0..5).map(|i| vec![Some(i.to_string()), Some(format!("row {i}"))]).collect(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut st = GridState::default();
    let changes = BTreeMap::new();
    let mut editor = None;
    let mut t = 0.0;
    let mut step = |events: Vec<egui::Event>, editor: &mut Option<CellEditor>, st: &mut GridState| {
        t += 0.05;
        frame_edit(&ctx, &rs, st, &changes, editor, events, t, &[None, None])
    };
    step(vec![], &mut editor, &mut st);
    // Double-click row 2's "name" cell (x = 120 is past the gutter and id column).
    let h = ctx.style().text_styles[&TextStyle::Monospace].size;
    let row_h = ctx.fonts_mut(|f| f.row_height(&egui::FontId::monospace(h))) + 10.0;
    let pos = egui::pos2(120.0, 8.0 + row_h + 4.0 + row_h * 2.5);
    let press = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    step(vec![egui::Event::PointerMoved(pos)], &mut editor, &mut st);
    for _ in 0..2 {
        step(vec![press(true)], &mut editor, &mut st);
        step(vec![press(false)], &mut editor, &mut st);
    }
    let ed = editor.as_ref().expect("double-click opens the editor");
    assert_eq!((ed.row, ed.col, ed.text.as_str()), (2, 1, "row 2"));
    // Typing replaces the (selected) value; Enter keeps it.
    step(vec![], &mut editor, &mut st);
    step(vec![egui::Event::Text("renamed".into())], &mut editor, &mut st);
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let ev = step(vec![enter], &mut editor, &mut st);
    match ev {
        Some(GridEvent::Change { row: 2, col: 1, value: Some(v) }) => assert_eq!(v, "renamed"),
        _ => panic!("expected a change"),
    }
    assert!(editor.is_none(), "the editor closes");

    // Esc cancels without a change.
    editor = Some(CellEditor::open(0, 1, Some("row 0")));
    step(vec![], &mut editor, &mut st);
    step(vec![egui::Event::Text("zzz".into())], &mut editor, &mut st);
    let esc = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    assert!(step(vec![esc], &mut editor, &mut st).is_none());
    assert!(editor.is_none());
}

#[test]
fn date_picker() {
    let rs = ResultSet {
        columns: vec!["id".into(), "due".into()],
        rows: vec![vec![Some("1".into()), Some("2024-02-10".into())]],
        ..Default::default()
    };
    let kinds = [None, Some(TemporalKind::Date)];
    let ctx = egui::Context::default();
    let mut st = GridState::default();
    let changes = BTreeMap::new();
    let mut editor = Some(CellEditor::open(0, 1, Some("2024-02-10")));
    let mut t = 0.0;
    let mut step = |events: Vec<egui::Event>, editor: &mut Option<CellEditor>| {
        t += 0.05;
        frame_edit(&ctx, &rs, &mut st, &changes, editor, events, t, &kinds)
    };
    step(vec![], &mut editor);
    step(vec![], &mut editor);
    let popup = ctx.memory(|m| m.area_rect(Id::new(("cell-picker", 0usize, 1usize)))).expect("picker shown");
    assert!(popup.height() > 150.0, "calendar rows: {popup:?}");
    // Clicking inside the picker keeps the editor open.
    let inside = popup.center_top() + egui::vec2(0.0, 8.0);
    let press = |pressed| egui::Event::PointerButton { pos: inside, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    step(vec![egui::Event::PointerMoved(inside)], &mut editor);
    step(vec![press(true)], &mut editor);
    step(vec![press(false)], &mut editor);
    assert!(editor.is_some());
    // Typed text and Enter still work.
    editor.as_mut().unwrap().text = "2024-02-29".into();
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    match step(vec![enter], &mut editor) {
        Some(GridEvent::Change { value: Some(v), .. }) => assert_eq!(v, "2024-02-29"),
        _ => panic!("expected a change"),
    }
    assert!(editor.is_none());
    // Esc cancels.
    editor = Some(CellEditor::open(0, 1, Some("2024-02-10")));
    step(vec![], &mut editor);
    let esc = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    assert!(step(vec![esc], &mut editor).is_none());
    assert!(editor.is_none());
}

#[test]
fn frozen_columns_scroll_together() {
    let rs = ResultSet {
        columns: vec!["id".into(), "name".into(), "note".into()],
        rows: (0..200).map(|i| vec![Some(i.to_string()), Some(format!("row {i}")), Some("x".repeat(80))]).collect(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut st = GridState { frozen: vec!["note".into()], ..Default::default() };
    frame(&ctx, &rs, &mut st, vec![]);
    frame(&ctx, &rs, &mut st, vec![]);
    // Wheel over the scrolling half (right side): both halves follow.
    let over = egui::pos2(700.0, 300.0);
    let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -400.0), modifiers: egui::Modifiers::NONE };
    frame(&ctx, &rs, &mut st, vec![egui::Event::PointerMoved(over), wheel]);
    for _ in 0..30 {
        frame(&ctx, &rs, &mut st, vec![]);
    }
    let y = st.scroll_y;
    assert!(y > 100.0, "scrolled: {y}");
    // A click in the frozen half selects the row shown there.
    let h = ctx.style().text_styles[&TextStyle::Monospace].size;
    let row_h = ctx.fonts_mut(|f| f.row_height(&egui::FontId::monospace(h))) + 10.0;
    let target = 8.0 + row_h + 4.0 + row_h * 2.5; // third visible row
    let pos = egui::pos2(120.0, target);
    let press = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &rs, &mut st, vec![egui::Event::PointerMoved(pos)]);
    frame(&ctx, &rs, &mut st, vec![press(true)]);
    frame(&ctx, &rs, &mut st, vec![press(false)]);
    let expected = ((y + row_h * 2.5) / row_h) as usize;
    assert_eq!(st.selection.iter().copied().collect::<Vec<_>>(), [expected], "scroll {y}, row_h {row_h}");
    // Freezing survives a new result with the same column names.
    assert_eq!(st.frozen, ["note"]);
}

#[test]
fn reveal_scrolls_row_into_view() {
    let rs = ResultSet {
        columns: vec!["id".into()],
        rows: (0..300).map(|i| vec![Some(i.to_string())]).collect(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut st = GridState::default();
    frame(&ctx, &rs, &mut st, vec![]);
    st.reveal = Some(150);
    for _ in 0..3 {
        frame(&ctx, &rs, &mut st, vec![]);
    }
    let h = ctx.style().text_styles[&TextStyle::Monospace].size;
    let row_h = ctx.fonts_mut(|f| f.row_height(&egui::FontId::monospace(h))) + 10.0;
    assert!(st.reveal.is_none());
    assert!((st.scroll_y - 148.0 * row_h).abs() < 1.0, "scroll {}", st.scroll_y);
    assert_eq!(reveal_offset(1, 25.0), 0.0);
}

fn key(k: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers }
}

#[test]
fn go_to_column() {
    let rs = ResultSet {
        columns: (0..40).map(|i| format!("col_{i}")).collect(),
        rows: (0..3).map(|r| (0..40).map(|c| Some(format!("{r}:{c}"))).collect()).collect(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut st = GridState::default();
    let none = egui::Modifiers::NONE;
    frame(&ctx, &rs, &mut st, vec![]);
    // Hover the grid so the shortcut applies.
    frame(&ctx, &rs, &mut st, vec![egui::Event::PointerMoved(egui::pos2(300.0, 200.0))]);
    assert!(st.goto.is_none());
    frame_mods(&ctx, &rs, &mut st, vec![key(egui::Key::J, egui::Modifiers::COMMAND)], egui::Modifiers::COMMAND);
    assert!(st.goto.is_some(), "cmd+J opens the popup");
    frame(&ctx, &rs, &mut st, vec![]);
    frame(&ctx, &rs, &mut st, vec![egui::Event::Text("col_3".into())]);
    frame(&ctx, &rs, &mut st, vec![]);
    assert_eq!(st.goto.as_ref().unwrap().query, "col_3");
    // Esc closes without jumping.
    frame(&ctx, &rs, &mut st, vec![key(egui::Key::Escape, none)]);
    assert!(st.goto.is_none());
    assert!(st.scroll_x.is_none() && st.flash.is_none());
    // Reopen, filter to col_3x (10 matches), move down once and jump.
    st.goto = Some(Goto { query: String::new(), highlight: 0, focus: true });
    frame(&ctx, &rs, &mut st, vec![]);
    frame(&ctx, &rs, &mut st, vec![egui::Event::Text("COL_3".into())]);
    frame(&ctx, &rs, &mut st, vec![key(egui::Key::ArrowDown, none)]);
    assert_eq!(st.goto.as_ref().unwrap().highlight, 1);
    frame(&ctx, &rs, &mut st, vec![key(egui::Key::Enter, none)]);
    assert!(st.goto.is_none(), "enter closes");
    // Matches are col_3, col_30.. so the highlighted one is col_30.
    assert_eq!(st.flash.map(|f| f.0), Some(30));
    assert!(st.scroll_x.is_some_and(|x| x > 500.0), "scroll_x {:?}", st.scroll_x);
}

#[test]
fn column_copies() {
    let s = |v: &str| Some(v.to_string());
    let rs = ResultSet {
        columns: vec!["id".into(), "land_right".into()],
        rows: vec![
            vec![s("1"), s("{23F7220B-490C}")],
            vec![s("2"), None],
            vec![s("3"), s("O'Neil")],
            vec![s("4"), s("{23F7220B-490C}")],
        ],
        ..Default::default()
    };
    let all = [0, 1, 2, 3];
    assert_eq!(column_lines(&rs, &all, 1), "{23F7220B-490C}\nNULL\nO'Neil\n{23F7220B-490C}");
    assert_eq!(column_distinct(&rs, &all, 1), ["{23F7220B-490C}", "O'Neil"]);
    assert_eq!(column_sql_list(&rs, &all, 1), "('{23F7220B-490C}', 'O''Neil')");
    assert_eq!(column_sql_list(&rs, &[0, 2], 0), "('1', '3')");
    let codes = ResultSet { columns: vec!["zip".into()], rows: vec![vec![s("012")], vec![s("7")]], ..Default::default() };
    assert_eq!(column_sql_list(&codes, &[0, 1], 0), "('012', '7')");
}

#[test]
fn cell_text_is_bounded() {
    assert_eq!(cell_text("a\nb", 10), "a↵ b");
    assert_eq!(cell_text("abcdef", 3), "abc…");
    assert_eq!(cell_text("ភ្នំពេញ", 3).chars().count(), 4);
    assert_eq!(cell_text(&"x".repeat(100_000), 240).chars().count(), 241);
}

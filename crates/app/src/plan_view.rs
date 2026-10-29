//! The console's Plan view: an EXPLAIN plan as a collapsible tree, the slow
//! steps marked. The parsing and numbers live in `rgrip_core::explain`.

use crate::theme;
use eframe::egui::{self, Color32, Frame, Id, Margin, RichText, ScrollArea, Sense, Shape, Stroke, Ui};
use rgrip_core::explain::{Plan, PlanNode};
use std::collections::HashSet;

pub struct PlanState {
    pub plan: Plan,
    /// A line under the header (e.g. that an ANALYZE ran in a transaction that was rolled back).
    pub note: Option<String>,
    /// Show the JSON instead of the tree.
    pub raw: bool,
    /// Nodes (by preorder index) whose children are hidden.
    collapsed: HashSet<usize>,
    /// Nodes whose conditions and buffers are shown.
    expanded: HashSet<usize>,
}

impl PlanState {
    pub fn new(plan: Plan, note: Option<String>) -> Self {
        Self { plan, note, raw: false, collapsed: HashSet::new(), expanded: HashSet::new() }
    }
}

fn ms(v: f64) -> String {
    if v < 1.0 {
        format!("{v:.3} ms")
    } else if v < 1000.0 {
        format!("{v:.1} ms")
    } else {
        format!("{:.2} s", v / 1000.0)
    }
}

fn count(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        let s = (v.abs() as i64).to_string();
        let mut out = String::new();
        for (i, ch) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                out.push(',');
            }
            out.push(ch);
        }
        if v < 0.0 {
            out.push('-');
        }
        out.chars().rev().collect()
    } else {
        format!("{v:.1}")
    }
}

/// Amber for a little, red for a lot.
fn heat(share: f64) -> Color32 {
    let t = ((share - 0.1) / 0.5).clamp(0.0, 1.0) as f32;
    let (a, b) = (theme::WARN(), theme::ERROR());
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// The header and the tree (or the raw JSON). Copy and the Raw toggle are in the console's toolbar.
pub fn show(ui: &mut Ui, id: Id, st: &mut PlanState) {
    Frame::new().inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let p = &st.plan;
            if let Some(t) = p.planning_ms {
                ui.label(RichText::new(format!("Planning {}", ms(t))).color(theme::MUTED()));
            }
            if let Some(t) = p.execution_ms {
                ui.label(RichText::new(format!("Execution {}", ms(t))).strong().color(theme::TEXT()));
            }
            ui.label(RichText::new(format!("Cost {}", count(p.root.total_cost.round()))).color(theme::MUTED()));
            let n = p.node_count();
            ui.label(RichText::new(if n == 1 { "1 step".to_string() } else { format!("{n} steps") }).color(theme::MUTED()));
            if !p.analyzed {
                ui.label(RichText::new("Estimates only: Explain analyze runs it for real timings").color(theme::FAINT()));
            }
        });
        if let Some(note) = &st.note {
            ui.label(RichText::new(note.as_str()).color(theme::WARN()));
        }
    });
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, theme::BORDER()));
    ScrollArea::both().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
        if st.raw {
            ui.add_space(6.0);
            ui.label(RichText::new(st.plan.raw.as_str()).monospace().color(theme::TEXT()));
            return;
        }
        ui.add_space(4.0);
        let mut next = 0;
        let PlanState { plan, collapsed, expanded, .. } = st;
        node_ui(ui, &plan.root, 0, &mut next, plan.analyzed, collapsed, expanded);
    });
}

fn node_ui(ui: &mut Ui, n: &PlanNode, depth: usize, next: &mut usize, analyzed: bool, collapsed: &mut HashSet<usize>, expanded: &mut HashSet<usize>) {
    let me = *next;
    *next += 1;
    let width = ui.available_width();
    // The share bar is painted behind the row once its height is known.
    let bar = ui.painter().add(Shape::Noop);
    let resp = ui
        .horizontal_wrapped(|ui| {
            ui.add_space(14.0 + depth as f32 * 20.0);
            if n.children.is_empty() {
                ui.add_space(14.0);
            } else {
                let arrow = if collapsed.contains(&me) { "▸" } else { "▾" };
                if ui.add(egui::Button::new(RichText::new(arrow).color(theme::MUTED())).frame(false).min_size(egui::vec2(14.0, 0.0))).clicked() && !collapsed.remove(&me) {
                    collapsed.insert(me);
                }
            }
            ui.label(RichText::new(n.label.as_str()).strong().color(theme::TEXT()));
            let mut stats = Vec::new();
            match &n.actual {
                Some(a) if a.loops > 0.0 => {
                    stats.push(format!("rows {} (est {})", count(a.rows), count(n.plan_rows)));
                    if a.loops != 1.0 {
                        stats.push(format!("{} loops", count(a.loops)));
                    }
                    stats.push(format!("{} (self {})", ms(a.total_ms), ms(a.self_ms)));
                }
                Some(_) => stats.push("never executed".into()),
                None => stats.push(format!("rows {}", count(n.plan_rows))),
            }
            stats.push(format!("cost {}..{}", count(n.startup_cost.round()), count(n.total_cost.round())));
            ui.label(RichText::new(stats.join("  ·  ")).small().color(theme::MUTED()));
            if n.share >= 0.05 {
                ui.label(RichText::new(format!("{:.0}% of {}", n.share * 100.0, if analyzed { "time" } else { "cost" })).small().strong().color(heat(n.share)));
            }
            if let Some(m) = n.miss {
                let what = if m.under { "more rows than estimated" } else { "fewer rows than estimated" };
                ui.label(RichText::new(format!("⚠ {}× {what}", count(m.factor.round()))).small().color(theme::WARN()))
                    .on_hover_text("The planner's row estimate is far off; stale statistics (ANALYZE the table) often explain it");
            }
        })
        .response;
    let rect = resp.rect;
    if n.share > 0.0 {
        let w = (width * n.share as f32).max(3.0);
        let color = heat(n.share).gamma_multiply(0.22);
        ui.painter().set(bar, Shape::rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(w, rect.height())), 0.0, color));
    }
    if !n.details.is_empty() || n.buffers.is_some() {
        let r = ui.interact(rect, ui.id().with(("plan-row", me)), Sense::click());
        if r.clicked() && !expanded.remove(&me) {
            expanded.insert(me);
        }
        r.on_hover_text("Click for conditions and buffers");
    }
    if expanded.contains(&me) {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(14.0 + depth as f32 * 20.0 + 28.0);
            ui.vertical(|ui| {
                for (k, v) in &n.details {
                    ui.label(RichText::new(format!("{k}: {v}")).small().monospace().color(theme::FAINT()));
                }
                if let Some(b) = &n.buffers {
                    let mut parts = vec![format!("shared hit {}", count(b.shared_hit as f64)), format!("read {}", count(b.shared_read as f64))];
                    if b.shared_dirtied > 0 {
                        parts.push(format!("dirtied {}", count(b.shared_dirtied as f64)));
                    }
                    if b.shared_written > 0 {
                        parts.push(format!("written {}", count(b.shared_written as f64)));
                    }
                    if b.temp_read > 0 || b.temp_written > 0 {
                        parts.push(format!("temp read {} written {}", count(b.temp_read as f64), count(b.temp_written as f64)));
                    }
                    ui.label(RichText::new(format!("Buffers: {}", parts.join(", "))).small().monospace().color(theme::FAINT()));
                }
            });
        });
    }
    if collapsed.contains(&me) {
        // Hidden children still take their numbers, so the indexes stay stable.
        *next += subtree(n) - 1;
        return;
    }
    for c in &n.children {
        node_ui(ui, c, depth + 1, next, analyzed, collapsed, expanded);
    }
}

fn subtree(n: &PlanNode) -> usize {
    1 + n.children.iter().map(subtree).sum::<usize>()
}

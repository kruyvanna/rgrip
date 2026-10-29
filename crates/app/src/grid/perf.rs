//! `cargo test --release -p rgrip grid_frame_time -- --ignored --nocapture`
use super::*;
use std::time::Instant;

/// A page shaped like a real PostGIS table: long coordinate text,
/// Khmer HTML, NULLs and a geometry column.
fn heavy_page() -> ResultSet {
    let coords: String = (0..1500).map(|i| format!("104.91{i:05},11.61{i:05},0 ")).collect();
    let khmer = "<div>ផ្លូវកោះពេជ្រ&nbsp;<span>សង្កាត់ទន្លេបាសាក់ ខណ្ឌចំការមន រាជធានីភ្នំពេញ</span></div>".repeat(20);
    let geom = "0101000020E61000008F537424973B5A40AA8251499D202740".to_string();
    let rows = (0..1000)
        .map(|i| {
            vec![
                Some(i.to_string()),
                Some(format!("Untitled polygon {i}")),
                if i % 3 == 0 { Some(khmer.clone()) } else { None },
                Some(coords.clone()),
                Some("Polygon".into()),
                Some(geom.clone()),
            ]
        })
        .collect();
    ResultSet {
        columns: ["id", "name", "description", "coordinates", "geometry_type", "geom"].map(String::from).to_vec(),
        rows,
        ..Default::default()
    }
}

#[test]
#[ignore]
fn grid_frame_time() {
    let rs = heavy_page();
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(2.0);
    crate::theme::apply(&ctx, rgrip_core::config::ThemeChoice::Dark, "rgrip Dark", "rgrip Light");
    let mut state = GridState::default();
    let mut frame = |scroll: f32| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))),
            events: vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, scroll),
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let start = Instant::now();
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, egui::Id::new("perf"), &rs, &mut state, None, 0, &Source { table: "t", sql: None, relations: &[], counts: &[], deletable: false, filterable: false }, None);
            });
        });
        // Tessellation runs every frame in the real app too.
        let _ = ctx.tessellate(out.shapes, 2.0);
        start.elapsed()
    };
    for _ in 0..5 {
        frame(0.0);
    }
    let n = 60;
    let total: std::time::Duration = (0..n).map(|i| frame(if i % 2 == 0 { -40.0 } else { 0.0 })).sum();
    println!("grid frame: {:.2} ms avg over {n} frames", total.as_secs_f64() * 1000.0 / n as f64);
}

mod app;
mod completion;
mod grid;
mod highlight;
mod icons;
mod import;
mod table_edit;
mod map;
mod map_export;
mod layout_tab;
mod shaping;
mod snapshot;
mod widgets;
mod theme;
mod worker;

fn main() -> eframe::Result {
    // Debug builds: RGRIP_DEMO_SIZE=1440x900 for screenshots.
    let size = std::env::var("RGRIP_DEMO_SIZE")
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .unwrap_or([1280.0, 800.0]);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("rgrip")
            .with_inner_size(size)
            .with_min_inner_size([760.0, 480.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon-512.png"))
                    .expect("bundled icon is a valid PNG"),
            ),
        ..Default::default()
    };
    eframe::run_native("rgrip", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}

mod app;
mod completion;
mod grid;
mod highlight;
mod map;
mod shaping;
mod snapshot;
mod widgets;
mod theme;
mod worker;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("rgrip")
            .with_inner_size([1280.0, 800.0])
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

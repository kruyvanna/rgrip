use super::*;

fn layout_with(shapes: Vec<Geometry>) -> Layout {
    let mut l = Layout::default();
    let mut layer = Layer::new(1, "demo", "parcels", "geom", [200, 60, 40]);
    layer.shapes = shapes.into();
    l.layers.push(layer);
    l.basemap = Basemap::None;
    l
}

#[test]
fn page_and_frame() {
    let mut l = Layout::default();
    assert_eq!(l.page_mm(), (297.0, 210.0));
    l.landscape = false;
    assert_eq!(l.frame(), Rect { x: 10.0, y: 10.0, w: 190.0, h: 277.0 });
    // Each side on its own: top, right, bottom, left.
    l.margin = [20.0, 5.0, 30.0, 15.0];
    assert_eq!(l.frame(), Rect { x: 15.0, y: 20.0, w: 190.0, h: 247.0 });
}

#[test]
fn page_and_map_coordinates_round_trip() {
    let l = Layout::default();
    let f = l.frame();
    let mid = l.to_page(l.center);
    assert!((mid[0] - (f.x + f.w / 2.0)).abs() < 1e-6 && (mid[1] - (f.y + f.h / 2.0)).abs() < 1e-6);
    let c = l.from_page([30.0, 40.0]);
    let p = l.to_page(c);
    assert!((p[0] - 30.0).abs() < 1e-6 && (p[1] - 40.0).abs() < 1e-6);
    // 1:50,000: 1 mm on the page is 50 m on the ground.
    let east = l.from_page([mid[0] + 1.0, mid[1]]);
    let ground = crate::layout::tests::haversine(l.center, east);
    assert!((ground - 50.0).abs() < 0.5, "{ground}");
}

fn haversine(a: Coord, b: Coord) -> f64 {
    let (la1, la2) = (a[1].to_radians(), b[1].to_radians());
    let h = ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((b[0] - a[0]).to_radians() / 2.0).sin().powi(2);
    2.0 * 6_371_008.8 * h.sqrt().asin()
}

#[test]
fn zoom_keeps_the_point_under_the_cursor() {
    let mut l = Layout::default();
    let at = [60.0, 50.0];
    let before = l.from_page(at);
    l.zoom_at(2.0, at);
    assert!((l.scale - 25_000.0).abs() < 1e-6);
    let after = l.to_page(before);
    assert!((after[0] - at[0]).abs() < 1e-6 && (after[1] - at[1]).abs() < 1e-6);
}

#[test]
fn fit_frames_the_layers() {
    let mut l = layout_with(vec![Geometry::LineString(vec![[104.85, 11.50], [104.95, 11.60]])]);
    l.fit();
    let f = l.frame();
    for c in [[104.85, 11.50], [104.95, 11.60]] {
        let p = l.to_page(c);
        assert!(f.contains(p), "{p:?} outside {f:?}");
    }
    assert_eq!(l.scale, round_scale(l.scale));
}

#[test]
fn scale_bar_and_numbers() {
    let mut l = Layout::default();
    l.scale = 50_000.0;
    // 40 mm at 1:50,000 is 2 km: the bar is exactly that.
    let (len, label) = l.scale_bar();
    assert_eq!(label, "2 km");
    assert!((len - 40.0).abs() < 1e-9);
    l.scale = 2_500.0;
    assert_eq!(l.scale_bar().1, "100 m");
    assert_eq!(thousands(1_234_567.0), "1,234,567");
    assert_eq!(round_scale(23_456.0), 23_000.0);
    assert_eq!(nice_length(7.0), 5.0);
}

#[test]
fn tile_count_is_capped() {
    let mut l = Layout::default();
    l.basemap = Basemap::Streets;
    l.page = PageSize::A2;
    l.scale = 5_000.0;
    let (z, tiles) = l.tiles(300.0);
    assert!(tiles.len() <= MAX_TILES && !tiles.is_empty(), "{}", tiles.len());
    let (z_low, _) = l.tiles(72.0);
    assert!(z_low <= z);
}

#[test]
fn svg_has_layers_legend_and_elements() {
    let mut l = layout_with(vec![
        Geometry::Polygon(vec![vec![[104.90, 11.55], [104.93, 11.55], [104.93, 11.57], [104.90, 11.55]]]),
        Geometry::Point([104.91, 11.56]),
    ]);
    l.title = "Parcels & roads".into();
    l.layers[0].label = "Land parcels".into();
    let svg = l.svg(&mut |_, _, _| None, 150.0);
    assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
    assert!(svg.contains("fill-rule=\"evenodd\""));
    assert!(svg.contains("<circle"));
    assert!(svg.contains("Parcels &amp; roads"));
    assert!(svg.contains("Land parcels"));
    assert!(svg.contains("1:50,000"));
    // Hidden elements and layers leave no trace.
    l.item_mut(Element::Legend).visible = false;
    l.layers[0].visible = false;
    let svg = l.svg(&mut |_, _, _| None, 150.0);
    assert!(!svg.contains("Land parcels") && !svg.contains("evenodd"));
}

#[test]
fn categories_color_and_label_features() {
    let mut l = layout_with(vec![Geometry::Point([104.91, 11.56]), Geometry::Point([104.92, 11.56]), Geometry::Point([104.93, 11.56])]);
    l.layers[0].values = vec![Some("garden".to_string()), Some("public".to_string()), None].into();
    let class = |v: &str, label: &str, color| Class { value: Some(v.into()), label: label.into(), color, visible: true, count: 1 };
    l.layers[0].style = Style::Categories { column: "land_use".into(), classes: vec![class("garden", "Gardens", [0, 200, 0]), class("public", "Public land", [0, 0, 200])], more: true };
    let svg = l.svg(&mut |_, _, _| None, 150.0);
    assert!(svg.contains("#00c800") && svg.contains("#0000c8"));
    assert!(svg.contains("Gardens") && svg.contains("Public land") && svg.contains(">Other<"));
    // A hidden class is neither drawn nor listed.
    if let Style::Categories { classes, .. } = &mut l.layers[0].style {
        classes[1].visible = false;
    }
    let svg = l.svg(&mut |_, _, _| None, 150.0);
    assert!(!svg.contains("#0000c8") && !svg.contains("Public land"));
}

#[test]
fn legend_labels_can_break_lines() {
    let mut l = layout_with(vec![Geometry::Point([104.91, 11.56])]);
    let one = l.boxes()[Element::Legend as usize].h;
    l.layers[0].label = "Land parcels\n(surveyed 2024)".into();
    let two = l.boxes()[Element::Legend as usize].h;
    assert!((two - one - LEGEND_LINE).abs() < 1e-9);
    let svg = l.svg(&mut |_, _, _| None, 150.0);
    assert!(svg.contains("Land parcels<tspan") && svg.contains(">(surveyed 2024)</tspan>"), "{svg}");
}

#[test]
fn graticule_lines_and_labels() {
    assert_eq!(coord_label(104.9, true, false), "104.9°E");
    assert_eq!(coord_label(-0.5, true, false), "0.5°W");
    assert_eq!(coord_label(11.5, false, true), "11°30′N");
    assert_eq!(coord_label(11.0 + 34.0 / 60.0 + 20.0 / 3600.0, false, true), "11°34′20″N");
    assert_eq!(coord_label(-12.0, false, true), "12°S");
    assert_eq!(step_label(5.0 / 60.0), "5′");
    assert_eq!(step_label(30.0 / 3600.0), "30″");
    assert_eq!(step_label(2.0), "2°");
    let mut l = Layout::default();
    l.basemap = Basemap::None;
    assert!(!l.svg(&mut |_, _, _| None, 150.0).contains("°E"));
    l.graticule.visible = true;
    // 1:50,000 on A4: lines a round step apart, at least 35 mm.
    let step = l.graticule_step();
    assert!(GRATICULE_STEPS_DECIMAL.contains(&step), "{step}");
    assert_eq!(step_label(0.05), "0.05°");
    let svg = l.svg(&mut |_, _, _| None, 150.0);
    assert!(svg.contains("°E") && svg.contains("°N"), "{svg}");
    // Latitude labels run up the frame edges, outside a 10 mm margin.
    assert!(svg.contains("rotate(-90"), "{svg}");
    assert!(!svg.contains(r##"°N</text>"##) || svg.matches("paint-order").count() < 10);
    l.graticule.interval = Some(1.0 / 60.0);
    l.graticule.dms = true;
    let svg = l.svg(&mut |_, _, _| None, 150.0);
    assert!(svg.contains("′E"));
    l.graticule.labels = false;
    assert!(!l.svg(&mut |_, _, _| None, 150.0).contains("′E"));
}

#[test]
fn elements_render_on_their_own() {
    let mut l = layout_with(vec![Geometry::Point([104.91, 11.56])]);
    l.title = "Parcels".into();
    let map = l.svg_map(&mut |_, _, _| None, 150.0);
    assert!(!map.contains("Parcels") && !map.contains("Legend") && map.contains("<circle"));
    let (title, r) = l.element_svg(Element::Title).unwrap();
    assert!(title.contains("Parcels") && !title.contains("Legend"));
    assert!(r.contains(l.item(Element::Title).pos));
    l.item_mut(Element::North).visible = false;
    assert!(l.element_svg(Element::North).is_none());
    l.fit_bounds([104.9, 11.55], [104.9, 11.55], 2000.0);
    assert_eq!(l.scale, 2000.0);
}

#[test]
fn khmer_width_skips_stacked_marks() {
    // ខ្មែរ: ខ with ម written below it, the vowel sign, then រ: two columns.
    assert!((text_w("ខ្មែរ", 1.0) - 0.62 * 2.0).abs() < 1e-9);
}

#[test]
fn text_styles() {
    let mut l = layout_with(vec![Geometry::Point([104.91, 11.56])]);
    l.title = "T".into();
    l.title_color = [200, 0, 0];
    l.title_bold = false;
    l.title_halo = false;
    let (svg, _) = l.element_svg(Element::Title).unwrap();
    assert!(svg.contains(r##"fill="#c80000""##) && svg.contains(r#"font-weight="400""#) && !svg.contains("paint-order"), "{svg}");
    let small = l.boxes()[Element::Legend as usize];
    l.legend_size *= 2.0;
    l.legend_box = false;
    l.legend_color = [0, 0, 200];
    let big = l.boxes()[Element::Legend as usize];
    assert!((big.w - 2.0 * small.w).abs() < 1e-6 && (big.h - 2.0 * small.h).abs() < 1e-6);
    let (svg, _) = l.element_svg(Element::Legend).unwrap();
    assert!(svg.contains("scale(2)") && svg.contains(r##"fill="#0000c8""##) && !svg.contains("<rect x"), "{svg}");
}

#[test]
fn overlapping_polygons_dont_cut_each_other() {
    let square = |x: f64| Geometry::Polygon(vec![vec![[x, 11.55], [x + 0.02, 11.55], [x + 0.02, 11.57], [x, 11.57], [x, 11.55]]]);
    let l = layout_with(vec![square(104.90), square(104.91)]);
    let svg = l.svg(&mut |_, _, _| None, 150.0);
    // Each polygon is its own path, so even-odd only applies within it.
    let group = svg.split("fill-rule=\"evenodd\"").nth(1).and_then(|g| g.split("</g>").next()).unwrap();
    assert_eq!(group.matches("<path ").count(), 2, "{group}");
}

#[test]
fn tiles_are_embedded() {
    let mut l = Layout::default();
    l.page = PageSize::A5;
    l.scale = 2_000_000.0;
    let png = vec![0x89, b'P', b'N', b'G'];
    let svg = l.svg(&mut |_, _, _| Some(png.clone()), 72.0);
    assert!(svg.contains("data:image/png;base64,iVBORw=="));
    assert!(svg.contains(Basemap::Light.attribution().split(',').next().unwrap()));
}

#[test]
fn layer_sql_and_base64() {
    let mut layer = Layer::new(1, "demo", "parcels", "geom", [0, 0, 0]);
    let sql = layer.data_sql([104.8, 11.5, 105.0, 11.7], 50_000.0, 100);
    assert!(sql.contains("ST_MakeEnvelope(104.8, 11.5, 105, 11.7, 4326)"), "{sql}");
    assert!(sql.contains("ST_Transform(\"geom\"::geometry, 4326)") && sql.ends_with("limit 100"), "{sql}");
    layer.filter = "land_use = 'garden'".into();
    layer.style = Style::Categories { column: "land_use".into(), classes: Vec::new(), more: false };
    let sql = layer.data_sql([0.0, 0.0, 1.0, 1.0], 1000.0, 5);
    assert!(sql.contains(", (\"land_use\")::text from") && sql.contains(" and (land_use = 'garden') limit 5"), "{sql}");
    assert!(layer.extent_sql().contains("ST_Extent(\"geom\"::geometry)"));
    // Reload when the view leaves what's loaded, or the scale moves a lot.
    layer.loaded = Some(([0.0, 0.0, 10.0, 10.0], 10_000.0));
    assert!(!layer.needs_reload([1.0, 1.0, 9.0, 9.0], 12_000.0));
    assert!(layer.needs_reload([1.0, 1.0, 11.0, 9.0], 12_000.0));
    assert!(layer.needs_reload([1.0, 1.0, 9.0, 9.0], 4_000.0));
    assert_eq!(base64(b"Man"), "TWFu");
    assert_eq!(base64(b"Ma"), "TWE=");
    assert_eq!(base64(b"M"), "TQ==");
}

#[test]
fn items_stay_on_the_page() {
    let mut l = Layout::default();
    l.item_mut(Element::North).pos = [500.0, -20.0];
    l.clamp_items();
    let b = l.boxes()[Element::North as usize];
    let (w, _) = l.page_mm();
    assert!(b.x + b.w <= w && b.y >= 0.0);
}

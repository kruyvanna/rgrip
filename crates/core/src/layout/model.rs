use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageSize {
    A5,
    A4,
    A3,
    A2,
    Letter,
    Legal,
    Tabloid,
}

impl PageSize {
    pub const ALL: [PageSize; 7] = [PageSize::A5, PageSize::A4, PageSize::A3, PageSize::A2, PageSize::Letter, PageSize::Legal, PageSize::Tabloid];

    pub fn label(self) -> &'static str {
        match self {
            PageSize::A5 => "A5",
            PageSize::A4 => "A4",
            PageSize::A3 => "A3",
            PageSize::A2 => "A2",
            PageSize::Letter => "Letter",
            PageSize::Legal => "Legal",
            PageSize::Tabloid => "Tabloid",
        }
    }

    /// Portrait width and height in mm.
    pub fn mm(self) -> (f64, f64) {
        match self {
            PageSize::A5 => (148.0, 210.0),
            PageSize::A4 => (210.0, 297.0),
            PageSize::A3 => (297.0, 420.0),
            PageSize::A2 => (420.0, 594.0),
            PageSize::Letter => (215.9, 279.4),
            PageSize::Legal => (215.9, 355.6),
            PageSize::Tabloid => (279.4, 431.8),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Basemap {
    None,
    Light,
    Dark,
    Streets,
    Satellite,
}

impl Basemap {
    pub const ALL: [Basemap; 5] = [Basemap::Light, Basemap::Dark, Basemap::Streets, Basemap::Satellite, Basemap::None];

    pub fn label(self) -> &'static str {
        match self {
            Basemap::None => "None",
            Basemap::Light => "Light",
            Basemap::Dark => "Dark",
            Basemap::Streets => "Streets",
            Basemap::Satellite => "Satellite",
        }
    }

    pub fn tile_url(self, z: u8, x: u32, y: u32) -> Option<String> {
        Some(match self {
            Basemap::None => return None,
            Basemap::Light => format!("https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Light_Gray_Base/MapServer/tile/{z}/{y}/{x}"),
            Basemap::Dark => format!("https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{z}/{y}/{x}"),
            Basemap::Streets => format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png"),
            Basemap::Satellite => format!("https://mt1.google.com/vt/lyrs=s&x={x}&y={y}&z={z}"),
        })
    }

    pub fn max_zoom(self) -> u8 {
        match self {
            Basemap::Light | Basemap::Dark => 16,
            Basemap::Streets => 19,
            Basemap::Satellite => 20,
            Basemap::None => 0,
        }
    }

    pub fn attribution(self) -> &'static str {
        match self {
            Basemap::None => "",
            Basemap::Light | Basemap::Dark => "Esri, HERE, Garmin, © OpenStreetMap contributors",
            Basemap::Streets => "© OpenStreetMap contributors",
            Basemap::Satellite => "Imagery © Google",
        }
    }
}

/// What a layer draws, for its legend swatch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Point,
    Line,
    Polygon,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub id: u64,
    pub schema: String,
    pub table: String,
    pub column: String,
    /// SQL WHERE text; empty for all rows.
    pub filter: String,
    /// Legend label.
    pub label: String,
    pub color: [u8; 3],
    /// Line width and polygon outline, mm.
    pub width: f64,
    /// Point radius, mm.
    pub point: f64,
    pub fill_opacity: f64,
    pub visible: bool,
    pub style: Style,
    /// Loaded geometries, WGS84 (shared: layouts are cloned for each render).
    pub shapes: std::sync::Arc<Vec<Geometry>>,
    /// The category column's value for each shape (empty without categories).
    pub values: std::sync::Arc<Vec<Option<String>>>,
    /// The whole layer's extent in lon/lat (for Fit).
    pub extent: Option<(Coord, Coord)>,
    /// The area (lon/lat bbox) and scale the shapes were loaded for.
    pub loaded: Option<([f64; 4], f64)>,
    /// Rows in the table (with the filter).
    pub total: Option<i64>,
}

/// How a layer is colored.
#[derive(Clone, Debug, PartialEq)]
pub enum Style {
    /// One color for every feature.
    Single,
    /// A color per value of `column`; `more` when there were more values than classes.
    Categories { column: String, classes: Vec<Class>, more: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    pub value: Option<String>,
    /// Legend label (starts as the value).
    pub label: String,
    pub color: [u8; 3],
    pub visible: bool,
    /// Features with this value in view.
    pub count: i64,
}

/// Most categories a layer draws and lists; further values draw as "Other".
pub const MAX_CLASSES: usize = 12;

impl Style {
    /// The classes drawn and listed: values in view, most first, at most
    /// [`MAX_CLASSES`] (the classes are kept sorted by count).
    pub fn shown(classes: &[Class]) -> impl Iterator<Item = &Class> {
        classes.iter().filter(|c| c.count > 0).take(MAX_CLASSES)
    }
}

/// One row of the legend: a swatch (layer kind and color) or a heading.
pub struct LegendRow {
    pub label: String,
    pub swatch: Option<(Kind, [u8; 3])>,
    pub indent: bool,
}

//! Loading files (CSV / TSV, GeoJSON, Shapefile) into PostgreSQL tables.
//!
//! Everything here is pure or blocking file work: reading rows as text,
//! inferring column types, and encoding rows as COPY text. The database side
//! (`PgConn::import`) feeds the encoded chunks to `COPY ... FROM STDIN` inside
//! one transaction.
//!
//! Rows are `Vec<Option<String>>` like query results. Geometry values travel
//! as WKT text and get an `SRID=n;` prefix when encoded (EWKT), which PostGIS
//! accepts in COPY text for geometry columns.

use serde_json::Value;
use std::path::{Path, PathBuf};

pub type Row = Vec<Option<String>>;

/// Rows read to infer types and to preview.
pub const SAMPLE_ROWS: usize = 1000;
/// COPY data is sent in chunks of about this many bytes.
const CHUNK_BYTES: usize = 256 * 1024;

/// Column types the dialog offers (`geometry` becomes `geometry(Geometry, srid)`).
pub const TYPES: [&str; 10] =
    ["text", "integer", "bigint", "numeric", "boolean", "date", "timestamp", "timestamptz", "jsonb", "geometry"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Csv,
    GeoJson,
    Shapefile,
}

impl Format {
    pub fn from_path(path: &Path) -> Option<Format> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "csv" | "tsv" | "tab" | "txt" => Some(Format::Csv),
            "geojson" | "json" => Some(Format::GeoJson),
            "shp" => Some(Format::Shapefile),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => "CSV / TSV",
            Format::GeoJson => "GeoJSON",
            Format::Shapefile => "Shapefile",
        }
    }
}

/// What opening a file found: columns, a sample of rows, and format details.
#[derive(Clone, Debug)]
pub struct Opened {
    pub format: Format,
    pub columns: Vec<String>,
    /// Up to [`SAMPLE_ROWS`] rows.
    pub sample: Vec<Row>,
    pub delimiter: u8,
    /// The geometry column (GeoJSON, Shapefile); its values are WKT.
    pub geometry: Option<usize>,
    /// SRID read from the file (GeoJSON `crs`, Shapefile `.prj`), if known.
    pub srid: Option<i32>,
    /// Something worth telling the user (e.g. no `.prj` next to the shapefile).
    pub note: Option<String>,
}

// ---- opening and scanning ------------------------------------------------

/// Read the header and a sample of rows.
pub fn open(path: &Path) -> Result<Opened, String> {
    open_with(path, None)
}

/// Like [`open`], with the CSV delimiter given instead of detected.
pub fn open_with(path: &Path, delimiter: Option<u8>) -> Result<Opened, String> {
    let format = Format::from_path(path).ok_or("Unsupported file type (use .csv, .tsv, .geojson, .json or .shp)")?;
    let mut opened = match format {
        Format::Csv => {
            let delimiter = match delimiter {
                Some(d) => d,
                None => detect_delimiter_in_file(path)?,
            };
            let columns = csv_header(path, delimiter)?;
            Opened { format, columns, sample: Vec::new(), delimiter, geometry: None, srid: None, note: None }
        }
        Format::GeoJson => {
            let gj = load_geojson(path)?;
            Opened { format, columns: gj.columns, sample: Vec::new(), delimiter: b',', geometry: Some(gj.geometry), srid: Some(gj.srid), note: None }
        }
        Format::Shapefile => {
            let (columns, geom) = shapefile_columns(path)?;
            let (srid, note) = match std::fs::read_to_string(path.with_extension("prj")) {
                Ok(prj) => match srid_from_prj(&prj) {
                    Some(s) => (Some(s), None),
                    None => (None, Some("Could not tell the coordinate system from the .prj file; enter the SRID".to_string())),
                },
                Err(_) => (None, Some("No .prj file next to the shapefile; enter the SRID".to_string())),
            };
            Opened { format, columns, sample: Vec::new(), delimiter: b',', geometry: Some(geom), srid, note }
        }
    };
    let mut sample = Vec::new();
    scan(path, format, opened.delimiter, |row| {
        sample.push(row);
        Ok(sample.len() < SAMPLE_ROWS)
    })?;
    opened.sample = sample;
    Ok(opened)
}

/// Feed every data row to `f` until it returns `Ok(false)` (stop) or an
/// error. Errors name the row (counting data rows from 1).
pub fn scan(path: &Path, format: Format, delimiter: u8, mut f: impl FnMut(Row) -> Result<bool, String>) -> Result<(), String> {
    match format {
        Format::Csv => csv_scan(path, delimiter, &mut f),
        Format::GeoJson => {
            let gj = load_geojson(path)?;
            for (i, feature) in gj.features.iter().enumerate() {
                let row = feature_row(feature, &gj.columns, gj.geometry).map_err(|e| format!("row {}: {e}", i + 1))?;
                if !f(row)? {
                    break;
                }
            }
            Ok(())
        }
        Format::Shapefile => shapefile_scan(path, &mut f),
    }
}

// ---- CSV -----------------------------------------------------------------

fn csv_reader(path: &Path, delimiter: u8) -> Result<csv::Reader<std::fs::File>, String> {
    // Flexible: short rows are padded below, long ones reported with their row number.
    csv::ReaderBuilder::new().delimiter(delimiter).flexible(true).from_path(path).map_err(|e| e.to_string())
}

fn csv_header(path: &Path, delimiter: u8) -> Result<Vec<String>, String> {
    let mut rdr = csv_reader(path, delimiter)?;
    let h = rdr.headers().map_err(|e| format!("Could not read the header row: {e}"))?;
    let cols: Vec<String> = h.iter().map(|s| s.trim().to_string()).collect();
    if cols.is_empty() || cols.iter().all(|c| c.is_empty()) {
        return Err("The file has no header row".into());
    }
    Ok(cols)
}

fn csv_scan(path: &Path, delimiter: u8, f: &mut dyn FnMut(Row) -> Result<bool, String>) -> Result<(), String> {
    let mut rdr = csv_reader(path, delimiter)?;
    let width = rdr.headers().map_err(|e| format!("Could not read the header row: {e}"))?.len();
    let mut rec = csv::StringRecord::new();
    let mut n = 0;
    loop {
        n += 1;
        match rdr.read_record(&mut rec) {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(e) => return Err(format!("row {n}: {e}")),
        }
        if rec.len() > width {
            return Err(format!("row {n}: has {} fields but the header has {width}", rec.len()));
        }
        // Empty fields are NULL: the csv crate doesn't tell "" from nothing.
        let mut row: Row = rec.iter().map(|s| (!s.is_empty()).then(|| s.to_string())).collect();
        row.resize(width, None);
        if !f(row)? {
            return Ok(());
        }
    }
}

/// The delimiter that splits the first lines most consistently.
pub fn detect_delimiter(sample: &str) -> u8 {
    const CANDIDATES: [u8; 4] = [b',', b';', b'\t', b'|'];
    // Strip the BOM so a quote right after it still counts as quoting.
    let sample = sample.strip_prefix('\u{feff}').unwrap_or(sample);
    let counts: Vec<Vec<usize>> = CANDIDATES.iter().map(|&d| line_counts(sample, d, 8)).collect();
    // Consistent over all lines first, then simply the most on the header.
    let consistent = (0..CANDIDATES.len())
        .filter(|&i| counts[i].first().is_some_and(|&c| c > 0) && counts[i].iter().all(|&c| c == counts[i][0]))
        .max_by_key(|&i| (counts[i][0], std::cmp::Reverse(i)));
    let best = consistent.or_else(|| {
        (0..CANDIDATES.len()).filter(|&i| counts[i].first().is_some_and(|&c| c > 0)).max_by_key(|&i| (counts[i][0], std::cmp::Reverse(i)))
    });
    best.map_or(b',', |i| CANDIDATES[i])
}

/// Delimiter count per record for the first `max` records, ignoring
/// delimiters inside quotes (a quoted newline doesn't end a record).
fn line_counts(text: &str, delim: u8, max: usize) -> Vec<usize> {
    let (mut counts, mut cur, mut quoted) = (Vec::new(), 0, false);
    for b in text.bytes() {
        match b {
            b'"' => quoted = !quoted,
            b'\n' if !quoted => {
                counts.push(cur);
                cur = 0;
                if counts.len() == max {
                    return counts;
                }
            }
            b if b == delim && !quoted => cur += 1,
            _ => {}
        }
    }
    // The last, possibly cut-off line only counts when it's all we have.
    if counts.is_empty() {
        counts.push(cur);
    }
    counts
}

fn detect_delimiter_in_file(path: &Path) -> Result<u8, String> {
    use std::io::Read;
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("tsv")) {
        return Ok(b'\t');
    }
    let mut buf = vec![0u8; 64 * 1024];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)).map_err(|e| e.to_string())?;
    buf.truncate(n);
    // A cut multi-byte character at the end is fine for counting ASCII.
    Ok(detect_delimiter(&String::from_utf8_lossy(&buf)))
}

// ---- GeoJSON -------------------------------------------------------------

struct GeoJson {
    /// Property names in order of first appearance, then the geometry column.
    columns: Vec<String>,
    geometry: usize,
    features: Vec<Value>,
    srid: i32,
}

fn load_geojson(path: &Path) -> Result<GeoJson, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let doc: Value = serde_json::from_reader(std::io::BufReader::new(file)).map_err(|e| format!("Not valid JSON: {e}"))?;
    let srid = geojson_srid(&doc).unwrap_or(4326);
    let features = match doc.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => match doc.get("features") {
            Some(Value::Array(a)) => a.clone(),
            _ => return Err("FeatureCollection without a features array".into()),
        },
        Some("Feature") => vec![doc],
        Some(_) => return Err("Expected a GeoJSON FeatureCollection or Feature".into()),
        None => return Err("Not GeoJSON (no \"type\")".into()),
    };
    let mut names: Vec<String> = Vec::new();
    for f in &features {
        if let Some(Value::Object(props)) = f.get("properties") {
            for k in props.keys() {
                if !names.contains(k) {
                    names.push(k.clone());
                }
            }
        }
    }
    // serde_json (without preserve_order) sorts keys; keep that, it's stable.
    let mut geom = "geom".to_string();
    while names.contains(&geom) {
        geom.push('_');
    }
    let geometry = names.len();
    names.push(geom);
    Ok(GeoJson { columns: names, geometry, features, srid })
}

/// `crs.properties.name` such as `urn:ogc:def:crs:EPSG::3857` or `EPSG:3857`.
fn geojson_srid(doc: &Value) -> Option<i32> {
    let name = doc.get("crs")?.get("properties")?.get("name")?.as_str()?;
    if name.to_ascii_uppercase().contains("CRS84") {
        return Some(4326);
    }
    name.rsplit([':', '/']).next()?.parse().ok()
}

fn feature_row(feature: &Value, columns: &[String], geometry: usize) -> Result<Row, String> {
    let props = feature.get("properties").and_then(Value::as_object);
    let mut row: Row = columns
        .iter()
        .take(geometry)
        .map(|c| match props.and_then(|p| p.get(c)) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(v) => Some(v.to_string()),
        })
        .collect();
    // A bare geometry object counts as its own feature's geometry.
    let g = feature.get("geometry").unwrap_or(&Value::Null);
    row.push(geojson_geometry(g)?.map(|g| g.to_wkt()));
    Ok(row)
}

type Pt = (f64, f64, Option<f64>);

/// A geometry we can write as WKT, whatever format it came from.
#[derive(Debug, PartialEq)]
enum Geom {
    Point(Pt),
    Line(Vec<Pt>),
    Polygon(Vec<Vec<Pt>>),
    MultiPoint(Vec<Pt>),
    MultiLine(Vec<Vec<Pt>>),
    MultiPolygon(Vec<Vec<Vec<Pt>>>),
    Collection(Vec<Geom>),
}

fn fmt_pt(p: &Pt) -> String {
    match p.2 {
        Some(z) => format!("{} {} {}", p.0, p.1, z),
        None => format!("{} {}", p.0, p.1),
    }
}

fn fmt_pts(pts: &[Pt]) -> String {
    pts.iter().map(fmt_pt).collect::<Vec<_>>().join(", ")
}

fn fmt_rings(rings: &[Vec<Pt>]) -> String {
    rings.iter().map(|r| format!("({})", fmt_pts(r))).collect::<Vec<_>>().join(", ")
}

impl Geom {
    fn has_z(&self) -> bool {
        match self {
            Geom::Point(p) => p.2.is_some(),
            Geom::Line(p) | Geom::MultiPoint(p) => p.first().is_some_and(|p| p.2.is_some()),
            Geom::Polygon(r) | Geom::MultiLine(r) => r.first().and_then(|r| r.first()).is_some_and(|p| p.2.is_some()),
            Geom::MultiPolygon(p) => p.first().and_then(|r| r.first()).and_then(|r| r.first()).is_some_and(|p| p.2.is_some()),
            Geom::Collection(g) => g.first().is_some_and(Geom::has_z),
        }
    }

    pub fn to_wkt(&self) -> String {
        let z = if self.has_z() { " Z" } else { "" };
        match self {
            Geom::Point(p) => format!("POINT{z} ({})", fmt_pt(p)),
            Geom::Line(p) => format!("LINESTRING{z} ({})", fmt_pts(p)),
            Geom::Polygon(r) => format!("POLYGON{z} ({})", fmt_rings(r)),
            Geom::MultiPoint(p) => format!("MULTIPOINT{z} ({})", p.iter().map(|p| format!("({})", fmt_pt(p))).collect::<Vec<_>>().join(", ")),
            Geom::MultiLine(l) => format!("MULTILINESTRING{z} ({})", fmt_rings(l)),
            Geom::MultiPolygon(p) => format!(
                "MULTIPOLYGON{z} ({})",
                p.iter().map(|r| format!("({})", fmt_rings(r))).collect::<Vec<_>>().join(", ")
            ),
            Geom::Collection(g) => format!("GEOMETRYCOLLECTION{z} ({})", g.iter().map(Geom::to_wkt).collect::<Vec<_>>().join(", ")),
        }
    }
}

fn gj_pt(v: &Value) -> Result<Pt, String> {
    let a = v.as_array().ok_or("coordinates are not an array")?;
    let n = |i: usize| a.get(i).and_then(Value::as_f64);
    match (n(0), n(1)) {
        (Some(x), Some(y)) => Ok((x, y, n(2))),
        _ => Err("a position needs two numbers".into()),
    }
}

fn gj_list<T>(v: &Value, f: impl Fn(&Value) -> Result<T, String>) -> Result<Vec<T>, String> {
    v.as_array().ok_or("coordinates are not an array")?.iter().map(f).collect()
}

fn gj_pts(v: &Value) -> Result<Vec<Pt>, String> {
    gj_list(v, gj_pt)
}

fn gj_rings(v: &Value) -> Result<Vec<Vec<Pt>>, String> {
    gj_list(v, gj_pts)
}

/// `Ok(None)` for a null geometry.
fn geojson_geometry(g: &Value) -> Result<Option<Geom>, String> {
    if g.is_null() {
        return Ok(None);
    }
    let kind = g.get("type").and_then(Value::as_str).ok_or("geometry without a type")?;
    let coords = g.get("coordinates");
    let c = || coords.ok_or_else(|| format!("{kind} without coordinates"));
    Ok(Some(match kind {
        "Point" => Geom::Point(gj_pt(c()?)?),
        "LineString" => Geom::Line(gj_pts(c()?)?),
        "Polygon" => Geom::Polygon(gj_rings(c()?)?),
        "MultiPoint" => Geom::MultiPoint(gj_pts(c()?)?),
        "MultiLineString" => Geom::MultiLine(gj_rings(c()?)?),
        "MultiPolygon" => Geom::MultiPolygon(gj_list(c()?, gj_rings)?),
        "GeometryCollection" => {
            let parts = g.get("geometries").ok_or("GeometryCollection without geometries")?;
            Geom::Collection(gj_list(parts, |p| geojson_geometry(p)?.ok_or_else(|| "null geometry in a collection".to_string()))?)
        }
        other => return Err(format!("unsupported geometry type {other}")),
    }))
}

// ---- Shapefile -----------------------------------------------------------

fn shapefile_columns(path: &Path) -> Result<(Vec<String>, usize), String> {
    let dbf = path.with_extension("dbf");
    let reader = shapefile::dbase::Reader::from_path(&dbf).map_err(|e| format!("Could not read {}: {e}", dbf.display()))?;
    let mut names: Vec<String> = reader.fields().iter().map(|f| f.name().to_string()).collect();
    let mut geom = "geom".to_string();
    while names.iter().any(|n| n.eq_ignore_ascii_case(&geom)) {
        geom.push('_');
    }
    let geometry = names.len();
    names.push(geom);
    Ok((names, geometry))
}

fn shapefile_scan(path: &Path, f: &mut dyn FnMut(Row) -> Result<bool, String>) -> Result<(), String> {
    use shapefile::dbase::FieldValue as V;
    let (columns, geometry) = shapefile_columns(path)?;
    let mut reader = shapefile::Reader::from_path(path).map_err(|e| match e {
        shapefile::Error::MissingDbf => "The shapefile needs its .dbf file next to the .shp".to_string(),
        e => e.to_string(),
    })?;
    for (i, item) in reader.iter_shapes_and_records().enumerate() {
        let n = i + 1;
        let (shape, record) = item.map_err(|e| format!("row {n}: {e}"))?;
        let mut row: Row = columns[..geometry]
            .iter()
            .map(|name| match record.get(name)? {
                V::Character(s) => s.clone(),
                V::Memo(s) => Some(s.clone()),
                V::Numeric(v) => v.map(|v| v.to_string()),
                V::Float(v) => v.map(|v| v.to_string()),
                V::Logical(v) => v.map(|v| v.to_string()),
                V::Date(d) => d.map(|d| format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())),
                V::Integer(v) => Some(v.to_string()),
                V::Currency(v) | V::Double(v) => Some(v.to_string()),
                V::DateTime(t) => {
                    let (d, t) = (t.date(), t.time());
                    Some(format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", d.year(), d.month(), d.day(), t.hours(), t.minutes(), t.seconds()))
                }
            })
            .collect();
        row.push(shape_geometry(&shape).map_err(|e| format!("row {n}: {e}"))?.map(|g| g.to_wkt()));
        if !f(row)? {
            break;
        }
    }
    Ok(())
}

fn conv<P>(pts: &[P], f: impl Fn(&P) -> Pt) -> Vec<Pt> {
    pts.iter().map(f).collect()
}

fn shape_geometry(shape: &shapefile::Shape) -> Result<Option<Geom>, String> {
    use shapefile::{PolygonRing, Shape as S};
    let xy = |p: &shapefile::Point| (p.x, p.y, None);
    // M (measure) values are dropped; Z is kept.
    let xym = |p: &shapefile::PointM| (p.x, p.y, None);
    let xyz = |p: &shapefile::PointZ| (p.x, p.y, Some(p.z));
    // Each outer ring starts a polygon; inner rings are its holes.
    fn polygons<P>(rings: &[PolygonRing<P>], f: &dyn Fn(&P) -> Pt) -> Geom {
        let mut out: Vec<Vec<Vec<Pt>>> = Vec::new();
        for ring in rings {
            match (ring, out.last_mut()) {
                (PolygonRing::Inner(p), Some(last)) => last.push(conv(p, f)),
                (PolygonRing::Outer(p) | PolygonRing::Inner(p), _) => out.push(vec![conv(p, f)]),
            }
        }
        Geom::MultiPolygon(out)
    }
    Ok(Some(match shape {
        S::NullShape => return Ok(None),
        S::Point(p) => Geom::Point(xy(p)),
        S::PointM(p) => Geom::Point(xym(p)),
        S::PointZ(p) => Geom::Point(xyz(p)),
        S::Multipoint(m) => Geom::MultiPoint(conv(m.points(), xy)),
        S::MultipointM(m) => Geom::MultiPoint(conv(m.points(), xym)),
        S::MultipointZ(m) => Geom::MultiPoint(conv(m.points(), xyz)),
        S::Polyline(l) => Geom::MultiLine(l.parts().iter().map(|p| conv(p, xy)).collect()),
        S::PolylineM(l) => Geom::MultiLine(l.parts().iter().map(|p| conv(p, xym)).collect()),
        S::PolylineZ(l) => Geom::MultiLine(l.parts().iter().map(|p| conv(p, xyz)).collect()),
        S::Polygon(p) => polygons(p.rings(), &xy),
        S::PolygonM(p) => polygons(p.rings(), &xym),
        S::PolygonZ(p) => polygons(p.rings(), &xyz),
        S::Multipatch(_) => return Err("MultiPatch shapes are not supported".into()),
    }))
}

/// Guess an EPSG code from the text of a `.prj` file: an `AUTHORITY`/`ID`
/// at the end of the definition, else the common ESRI names (WGS 84 and
/// its UTM zones).
pub fn srid_from_prj(prj: &str) -> Option<i32> {
    // The CRS's own code is the last one: nested datum/unit codes come first.
    for key in ["AUTHORITY[\"EPSG\",\"", "ID[\"EPSG\","] {
        if let Some(i) = prj.rfind(key) {
            let digits: String = prj[i + key.len()..].trim_start_matches('"').chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = digits.parse() {
                return Some(n);
            }
        }
    }
    let up = prj.to_ascii_uppercase();
    if let Some(i) = up.find("UTM_ZONE_").or_else(|| up.find("UTM ZONE ")) {
        let rest = &up[i + 9..];
        let zone: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let south = rest[zone.len()..].starts_with('S');
        if let Ok(z) = zone.parse::<i32>()
            && (1..=60).contains(&z)
            && (up.contains("WGS_1984") || up.contains("WGS 84") || up.contains("WGS84"))
        {
            return Some(if south { 32700 + z } else { 32600 + z });
        }
    }
    if up.starts_with("GEOGCS") && (up.contains("GCS_WGS_1984") || up.contains("\"WGS 84\"") || up.contains("WGS_1984")) {
        return Some(4326);
    }
    None
}

// ---- names and type inference ----------------------------------------------

/// A lowercase identifier from a header or file name: letters, digits and
/// underscores, so queries need no quoting.
pub fn sanitize_name(raw: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in raw.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let mut out = out.trim_matches('_').to_string();
    if out.is_empty() {
        out = fallback.to_string();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert_str(0, "c_");
    }
    // Postgres identifiers are at most 63 bytes.
    while out.len() > 63 {
        out.pop();
    }
    out
}

/// Sanitized names, made unique with `_2`, `_3`, ….
pub fn unique_names(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for (i, r) in raw.iter().enumerate() {
        let base = sanitize_name(r, &format!("column_{}", i + 1));
        let mut name = base.clone();
        let mut n = 2;
        while out.contains(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        out.push(name);
    }
    out
}

/// Longitude and latitude columns by their usual names.
pub fn detect_lonlat(columns: &[String]) -> Option<(usize, usize)> {
    let find = |names: &[&str]| columns.iter().position(|c| names.contains(&c.trim().to_ascii_lowercase().as_str()));
    let lon = find(&["lon", "lng", "long", "longitude", "x", "lon_deg", "long_deg"])?;
    let lat = find(&["lat", "latitude", "y", "lat_deg"])?;
    Some((lon, lat))
}

/// The best column type for `values` (NULLs left out): the narrowest of
/// boolean, integer, bigint, numeric, date, timestamp, timestamptz, jsonb
/// and geometry that every value fits, else text. Numbers with leading
/// zeros ("007", zip codes) stay text.
pub fn infer_type<'a>(values: impl Iterator<Item = &'a str>) -> &'static str {
    // Candidate flags, cleared as values rule them out.
    let (mut boolean, mut int, mut bigint, mut numeric, mut date, mut ts, mut json, mut geom) = (true, true, true, true, true, true, true, true);
    let mut tz = false;
    let mut any = false;
    for v in values {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        any = true;
        boolean &= is_bool(v);
        let big = is_int(v);
        bigint &= big;
        int &= big && v.parse::<i32>().is_ok();
        numeric &= is_numeric(v);
        date &= is_date(v);
        match timestamp_kind(v) {
            Some(has_tz) => tz |= has_tz,
            None => ts = false,
        }
        json &= is_json(v);
        geom &= is_wkt(v);
        if !(boolean || int || bigint || numeric || date || ts || json || geom) {
            return "text";
        }
    }
    if !any {
        "text"
    } else if boolean {
        "boolean"
    } else if int {
        "integer"
    } else if bigint {
        "bigint"
    } else if numeric {
        "numeric"
    } else if date {
        "date"
    } else if ts {
        if tz { "timestamptz" } else { "timestamp" }
    } else if json {
        "jsonb"
    } else if geom {
        "geometry"
    } else {
        "text"
    }
}

fn is_bool(v: &str) -> bool {
    ["true", "false", "t", "f", "yes", "no"].iter().any(|b| v.eq_ignore_ascii_case(b))
}

fn is_int(v: &str) -> bool {
    let d = v.strip_prefix('-').unwrap_or(v);
    !d.is_empty() && d.len() <= 19 && d.bytes().all(|b| b.is_ascii_digit()) && (d == "0" || !d.starts_with('0')) && v.parse::<i64>().is_ok()
}

fn is_numeric(v: &str) -> bool {
    let d = v.strip_prefix('-').unwrap_or(v);
    if !d.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')) || !d.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return false;
    }
    let int_part = d.split(['.', 'e', 'E']).next().unwrap_or("");
    (int_part.is_empty() || int_part == "0" || !int_part.starts_with('0')) && v.parse::<f64>().is_ok_and(f64::is_finite)
}

fn is_date(v: &str) -> bool {
    let b = v.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    if !b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()) {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (v[0..4].parse::<u32>(), v[5..7].parse::<u32>(), v[8..10].parse::<u32>()) else { return false };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

/// `Some(has_zone)` for `YYYY-MM-DD[T ]HH:MM[:SS[.fff]][Z|±HH[:MM]]`.
fn timestamp_kind(v: &str) -> Option<bool> {
    let (d, t) = v.split_once(['T', ' '])?;
    if !is_date(d) {
        return None;
    }
    let (clock, tz) = match t.find(['Z', '+', '-']) {
        Some(i) => (&t[..i], Some(&t[i..])),
        None => (t, None),
    };
    let mut parts = clock.split(':');
    let (h, m) = (parts.next()?, parts.next()?);
    let s = parts.next();
    if parts.next().is_some() || h.len() != 2 || m.len() != 2 || h.parse::<u32>().ok()? > 23 || m.parse::<u32>().ok()? > 59 {
        return None;
    }
    if let Some(s) = s {
        let (whole, frac) = s.split_once('.').unwrap_or((s, "0"));
        if whole.len() != 2 || whole.parse::<u32>().ok()? > 60 || frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    match tz {
        None => Some(false),
        Some("Z") => Some(true),
        Some(z) => {
            let body = &z[1..];
            let ok = !body.is_empty()
                && body.len() <= 5
                && body.bytes().all(|b| b.is_ascii_digit() || b == b':')
                && body.split(':').all(|p| p.len() == 2 || p.len() == body.len());
            ok.then_some(true)
        }
    }
}

fn is_json(v: &str) -> bool {
    (v.starts_with('{') || v.starts_with('[')) && serde_json::from_str::<Value>(v).is_ok()
}

/// WKT or EWKT text such as `POINT(1 2)` or `SRID=4326;POLYGON((...))`.
pub fn is_wkt(v: &str) -> bool {
    let body = match v.get(..5) {
        Some(p) if p.eq_ignore_ascii_case("SRID=") => v.split_once(';').map_or("", |(_, b)| b),
        _ => v,
    };
    let up = body.trim().to_ascii_uppercase();
    const KINDS: [&str; 7] = ["POINT", "LINESTRING", "POLYGON", "MULTIPOINT", "MULTILINESTRING", "MULTIPOLYGON", "GEOMETRYCOLLECTION"];
    KINDS.iter().any(|k| {
        up.strip_prefix(k).is_some_and(|rest| {
            let rest = rest.trim_start().trim_start_matches(['Z', 'M']).trim_start();
            (rest.starts_with('(') && up.ends_with(')')) || rest == "EMPTY"
        })
    })
}

// ---- the import itself --------------------------------------------------------

/// Where a target column's values come from.
#[derive(Clone, Debug, PartialEq)]
pub enum Src {
    /// A column of the file.
    Field(usize),
    /// A point built from two numeric columns of the file.
    Point { lon: usize, lat: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanColumn {
    pub name: String,
    /// One of [`TYPES`] when creating a table; `geometry` also marks the
    /// columns of an existing table that get EWKT values.
    pub ty: String,
    pub src: Src,
}

#[derive(Clone, Debug)]
pub struct ImportRequest {
    pub path: PathBuf,
    pub format: Format,
    pub delimiter: u8,
    pub schema: String,
    pub table: String,
    /// Create the table first; otherwise append to the existing one.
    pub create: bool,
    pub columns: Vec<PlanColumn>,
    /// SRID of geometry columns (0: none).
    pub srid: i32,
    /// With `create`: a GiST index on each geometry column.
    pub spatial_index: bool,
    /// With `create`: the primary key. An imported column of that name, or
    /// else a new identity column (without a key rows can't be edited).
    pub key: Option<String>,
}

fn is_geometry(ty: &str) -> bool {
    ty.trim().to_ascii_lowercase().starts_with("geometry")
}

/// SQL type for a dialog type name and SRID.
pub fn sql_type(ty: &str, srid: i32) -> String {
    if ty == "geometry" {
        if srid > 0 { format!("geometry(Geometry, {srid})") } else { "geometry(Geometry)".into() }
    } else {
        ty.to_string()
    }
}

fn qualified(req: &ImportRequest) -> String {
    format!("{}.{}", crate::sql::quote_ident(&req.schema), crate::sql::quote_ident(&req.table))
}

pub fn create_table_sql(req: &ImportRequest) -> String {
    let key = req.key.as_deref();
    let mut cols: Vec<String> = req
        .columns
        .iter()
        .map(|c| {
            let pk = if key == Some(c.name.as_str()) { " primary key" } else { "" };
            format!("  {} {}{pk}", crate::sql::quote_ident(&c.name), sql_type(&c.ty, req.srid))
        })
        .collect();
    if let Some(k) = key.filter(|k| !req.columns.iter().any(|c| c.name == *k)) {
        cols.insert(0, format!("  {} bigint generated always as identity primary key", crate::sql::quote_ident(k)));
    }
    format!("create table {} (\n{}\n)", qualified(req), cols.join(",\n"))
}

pub fn copy_sql(req: &ImportRequest) -> String {
    let cols: Vec<String> = req.columns.iter().map(|c| crate::sql::quote_ident(&c.name)).collect();
    format!("copy {} ({}) from stdin", qualified(req), cols.join(", "))
}

/// GiST index statements for the geometry columns of a created table.
pub fn index_sql(req: &ImportRequest) -> Vec<String> {
    if !req.create || !req.spatial_index {
        return Vec::new();
    }
    req.columns
        .iter()
        .filter(|c| is_geometry(&c.ty))
        .map(|c| {
            let name = crate::sql::quote_ident(&format!("{}_{}_gix", req.table, c.name).chars().take(63).collect::<String>());
            format!("create index {name} on {} using gist ({})", qualified(req), crate::sql::quote_ident(&c.name))
        })
        .collect()
}

/// Append `value` as one COPY text field: backslash, tab, newline and
/// carriage return are escaped. NUL can't be stored in text.
pub fn copy_escape(value: &str, out: &mut Vec<u8>) -> Result<(), String> {
    for &b in value.as_bytes() {
        match b {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            0 => return Err("contains a NUL character, which PostgreSQL can't store".into()),
            b => out.push(b),
        }
    }
    Ok(())
}

/// EWKT for `wkt`, unless it already has an SRID or is hex WKB.
fn ewkt(wkt: &str, srid: i32) -> String {
    let w = wkt.trim();
    let hex = w.len() >= 10 && w.bytes().all(|b| b.is_ascii_hexdigit());
    let has_srid = w.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("SRID="));
    if srid > 0 && !hex && !has_srid { format!("SRID={srid};{w}") } else { w.to_string() }
}

fn number(s: Option<&String>) -> Result<Option<f64>, String> {
    match s.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => s.replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite()).map(Some).ok_or_else(|| format!("\"{s}\" is not a number")),
    }
}

/// Encode one row as a COPY text line (with the trailing newline).
pub fn encode_row(req: &ImportRequest, row: &Row, out: &mut Vec<u8>) -> Result<(), String> {
    for (i, col) in req.columns.iter().enumerate() {
        if i > 0 {
            out.push(b'\t');
        }
        let value: Option<String> = match &col.src {
            Src::Field(f) => row.get(*f).cloned().flatten().map(|v| if is_geometry(&col.ty) { ewkt(&v, req.srid) } else { v }),
            Src::Point { lon, lat } => {
                let (x, y) = (number(row.get(*lon).and_then(Option::as_ref)).map_err(|e| format!("longitude {e}"))?, number(row.get(*lat).and_then(Option::as_ref)).map_err(|e| format!("latitude {e}"))?);
                match (x, y) {
                    (Some(x), Some(y)) => Some(ewkt(&format!("POINT({x} {y})"), req.srid)),
                    _ => None,
                }
            }
        };
        match value {
            Some(v) => copy_escape(&v, out).map_err(|e| format!("column \"{}\" {e}", col.name))?,
            None => out.extend_from_slice(b"\\N"),
        }
    }
    out.push(b'\n');
    Ok(())
}

/// Read the file on a blocking thread, sending COPY text in chunks of
/// (bytes, rows). A read or encoding error is sent last, naming the row.
/// Stops quietly when the receiver is gone (the import failed or ended).
pub fn spawn_encoder(
    req: ImportRequest,
    tx: tokio::sync::mpsc::Sender<Result<(bytes::Bytes, usize), String>>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let (mut buf, mut rows, mut n) = (Vec::with_capacity(CHUNK_BYTES + 4096), 0usize, 0usize);
        let result = scan(&req.path, req.format, req.delimiter, |row| {
            n += 1;
            encode_row(&req, &row, &mut buf).map_err(|e| format!("row {n}: {e}"))?;
            rows += 1;
            if buf.len() >= CHUNK_BYTES {
                let chunk = bytes::Bytes::from(std::mem::replace(&mut buf, Vec::with_capacity(CHUNK_BYTES + 4096)));
                // A closed channel means nobody wants more rows.
                return Ok(tx.blocking_send(Ok((chunk, std::mem::take(&mut rows)))).is_ok());
            }
            Ok(true)
        });
        match result {
            Ok(()) if rows > 0 => {
                let _ = tx.blocking_send(Ok((bytes::Bytes::from(buf), rows)));
            }
            Ok(()) => {}
            Err(e) => {
                let _ = tx.blocking_send(Err(e));
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rgrip-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    fn infer(values: &[&str]) -> &'static str {
        infer_type(values.iter().copied())
    }

    #[test]
    fn infers_types() {
        assert_eq!(infer(&["1", "2", "-3"]), "integer");
        assert_eq!(infer(&["1", "3000000000"]), "bigint");
        assert_eq!(infer(&["1", "2.5", "1e3"]), "numeric");
        assert_eq!(infer(&["true", "F", "yes"]), "boolean");
        assert_eq!(infer(&["2024-01-31", "2024-02-29"]), "date");
        assert_eq!(infer(&["2023-02-29"]), "text");
        assert_eq!(infer(&["2024-01-31 10:00:00", "2024-01-31T10:00:00.123"]), "timestamp");
        assert_eq!(infer(&["2024-01-31T10:00:00Z", "2024-01-31 10:00:00+07:00"]), "timestamptz");
        assert_eq!(infer(&["{\"a\":1}", "[1,2]"]), "jsonb");
        assert_eq!(infer(&["POINT(1 2)", "SRID=4326;POLYGON((0 0,1 0,1 1,0 0))"]), "geometry");
        assert_eq!(infer(&["1", "abc"]), "text");
        assert_eq!(infer(&[]), "text");
        assert_eq!(infer(&["", " "]), "text");
    }

    #[test]
    fn leading_zeros_stay_text() {
        assert_eq!(infer(&["00501", "10001"]), "text");
        assert_eq!(infer(&["007"]), "text");
        assert_eq!(infer(&["0", "10"]), "integer");
        assert_eq!(infer(&["0.5", "10"]), "numeric");
        assert_eq!(infer(&["NaN"]), "text");
    }

    #[test]
    fn escapes_copy_text() {
        let mut out = Vec::new();
        copy_escape("a\tb\nc\\d\re", &mut out).unwrap();
        assert_eq!(out, b"a\\tb\\nc\\\\d\\re");
        assert!(copy_escape("a\0b", &mut Vec::new()).is_err());
    }

    fn req(columns: Vec<PlanColumn>, srid: i32) -> ImportRequest {
        ImportRequest {
            path: PathBuf::new(),
            format: Format::Csv,
            delimiter: b',',
            schema: "s".into(),
            table: "t".into(),
            create: true,
            columns,
            srid,
            spatial_index: true,
            key: None,
        }
    }

    fn col(name: &str, ty: &str, src: Src) -> PlanColumn {
        PlanColumn { name: name.into(), ty: ty.into(), src }
    }

    #[test]
    fn encodes_rows_with_nulls_and_geometry() {
        let r = req(vec![col("a", "text", Src::Field(0)), col("b", "text", Src::Field(1)), col("g", "geometry", Src::Field(2))], 4326);
        let mut out = Vec::new();
        let row = vec![Some("x\ty".to_string()), None, Some("POINT(1 2)".to_string())];
        encode_row(&r, &row, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "x\\ty\t\\N\tSRID=4326;POINT(1 2)\n");
        // A literal backslash-N is data, not NULL.
        let mut out = Vec::new();
        encode_row(&r, &vec![Some("\\N".to_string()), None, None], &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "\\\\N\t\\N\t\\N\n");
    }

    #[test]
    fn keeps_existing_srid_and_builds_points() {
        assert_eq!(ewkt("SRID=3857;POINT(1 2)", 4326), "SRID=3857;POINT(1 2)");
        assert_eq!(ewkt("POINT(1 2)", 0), "POINT(1 2)");
        assert_eq!(ewkt("0101000000000000000000F03F0000000000000040", 4326), "0101000000000000000000F03F0000000000000040");
        let r = req(vec![col("g", "geometry", Src::Point { lon: 0, lat: 1 })], 4326);
        let mut out = Vec::new();
        encode_row(&r, &vec![Some("100.5".into()), Some("13,75".into())], &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "SRID=4326;POINT(100.5 13.75)\n");
        let mut out = Vec::new();
        encode_row(&r, &vec![Some("100.5".into()), None], &mut out).unwrap();
        assert_eq!(out, b"\\N\n");
        assert!(encode_row(&r, &vec![Some("east".into()), Some("1".into())], &mut Vec::new()).is_err());
    }

    #[test]
    fn builds_sql() {
        let r = req(vec![col("id", "integer", Src::Field(0)), col("we\"ird", "text", Src::Field(1)), col("geom", "geometry", Src::Field(2))], 4326);
        assert_eq!(
            create_table_sql(&r),
            "create table \"s\".\"t\" (\n  \"id\" integer,\n  \"we\"\"ird\" text,\n  \"geom\" geometry(Geometry, 4326)\n)"
        );
        assert_eq!(copy_sql(&r), "copy \"s\".\"t\" (\"id\", \"we\"\"ird\", \"geom\") from stdin");
        assert_eq!(index_sql(&r), ["create index \"t_geom_gix\" on \"s\".\"t\" using gist (\"geom\")"]);
        // A new identity key goes first; an imported column becomes the key.
        let mut k = req(vec![col("code", "text", Src::Field(0))], 0);
        k.key = Some("id".into());
        assert_eq!(create_table_sql(&k), "create table \"s\".\"t\" (\n  \"id\" bigint generated always as identity primary key,\n  \"code\" text\n)");
        k.key = Some("code".into());
        assert_eq!(create_table_sql(&k), "create table \"s\".\"t\" (\n  \"code\" text primary key\n)");
        assert_eq!(sql_type("geometry", 0), "geometry(Geometry)");
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_name(" Population (2020) ", "c"), "population_2020");
        assert_eq!(sanitize_name("2020", "c"), "c_2020");
        assert_eq!(sanitize_name("***", "column_3"), "column_3");
        assert_eq!(unique_names(&["A".into(), "a".into(), "".into(), "a ".into()]), ["a", "a_2", "column_3", "a_3"]);
    }

    #[test]
    fn detects_delimiters() {
        assert_eq!(detect_delimiter("a,b,c\n1,2,3\n"), b',');
        assert_eq!(detect_delimiter("a;b;c\n1,5;2;3\n"), b';');
        assert_eq!(detect_delimiter("a\tb\tc\n1\t2\t3\n"), b'\t');
        assert_eq!(detect_delimiter("a|b\n1|2\n"), b'|');
        // Commas inside quotes don't count.
        assert_eq!(detect_delimiter("\"x, y\";b\n\"1, 2\";3\n"), b';');
        assert_eq!(detect_delimiter("\u{feff}a;b\n1;2\n"), b';');
        assert_eq!(detect_delimiter("single\nvalue\n"), b',');
    }

    #[test]
    fn reads_csv_with_bom_quotes_and_short_rows() {
        let p = tmp("a.csv", "\u{feff}id,name,note\n1,\"Smith, J\",\"line1\nline2\"\n2,Lee\n3,,\n");
        let o = open(&p).unwrap();
        assert_eq!(o.columns, ["id", "name", "note"]);
        assert_eq!(o.sample.len(), 3);
        assert_eq!(o.sample[0], vec![Some("1".into()), Some("Smith, J".into()), Some("line1\nline2".into())]);
        assert_eq!(o.sample[1], vec![Some("2".into()), Some("Lee".into()), None]);
        assert_eq!(o.sample[2], vec![Some("3".into()), None, None]);
    }

    #[test]
    fn reads_tsv_and_reports_bad_rows() {
        let p = tmp("b.tsv", "a\tb\n1\t2\n3\t4\t5\n");
        let o = open(&p);
        assert!(o.unwrap_err().starts_with("row 2:"));
        let p = tmp("c.tsv", "a\tb\n1\t2\n");
        assert_eq!(open(&p).unwrap().delimiter, b'\t');
    }

    #[test]
    fn rejects_non_utf8_csv() {
        let dir = std::env::temp_dir().join(format!("rgrip-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("latin1.csv");
        std::fs::write(&p, b"a,b\ncaf\xe9,1\n").unwrap();
        assert!(open(&p).unwrap_err().contains("row 1"));
    }

    const GEOJSON: &str = r#"{"type":"FeatureCollection","features":[
        {"type":"Feature","properties":{"name":"A","pop":10,"ok":true,"tags":{"k":1}},"geometry":{"type":"Point","coordinates":[100.5,13.75]}},
        {"type":"Feature","properties":{"name":"B","extra":"x"},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}},
        {"type":"Feature","properties":null,"geometry":null}]}"#;

    #[test]
    fn reads_geojson() {
        let p = tmp("a.geojson", GEOJSON);
        let o = open(&p).unwrap();
        assert_eq!(o.columns, ["name", "ok", "pop", "tags", "extra", "geom"]);
        assert_eq!(o.geometry, Some(5));
        assert_eq!(o.srid, Some(4326));
        assert_eq!(o.sample.len(), 3);
        assert_eq!(o.sample[0][0].as_deref(), Some("A"));
        assert_eq!(o.sample[0][1].as_deref(), Some("true"));
        assert_eq!(o.sample[0][3].as_deref(), Some("{\"k\":1}"));
        assert_eq!(o.sample[0][5].as_deref(), Some("POINT (100.5 13.75)"));
        assert_eq!(o.sample[1][5].as_deref(), Some("POLYGON ((0 0, 1 0, 1 1, 0 0))"));
        assert_eq!(o.sample[2], vec![None; 6]);
    }

    #[test]
    fn geojson_crs_and_geometry_kinds() {
        let doc: Value = serde_json::from_str(r#"{"crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::3857"}}}"#).unwrap();
        assert_eq!(geojson_srid(&doc), Some(3857));
        let doc: Value = serde_json::from_str(r#"{"crs":{"properties":{"name":"urn:ogc:def:crs:OGC:1.3:CRS84"}}}"#).unwrap();
        assert_eq!(geojson_srid(&doc), Some(4326));
        let g = |s: &str| geojson_geometry(&serde_json::from_str(s).unwrap()).unwrap().unwrap().to_wkt();
        assert_eq!(g(r#"{"type":"Point","coordinates":[1,2,3]}"#), "POINT Z (1 2 3)");
        assert_eq!(g(r#"{"type":"MultiPoint","coordinates":[[1,2],[3,4]]}"#), "MULTIPOINT ((1 2), (3 4))");
        assert_eq!(g(r#"{"type":"MultiLineString","coordinates":[[[0,0],[1,1]],[[2,2],[3,3]]]}"#), "MULTILINESTRING ((0 0, 1 1), (2 2, 3 3))");
        assert_eq!(
            g(r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]],[[[5,5],[6,5],[6,6],[5,5]]]]}"#),
            "MULTIPOLYGON (((0 0, 1 0, 1 1, 0 0)), ((5 5, 6 5, 6 6, 5 5)))"
        );
        assert_eq!(
            g(r#"{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]},{"type":"LineString","coordinates":[[0,0],[1,1]]}]}"#),
            "GEOMETRYCOLLECTION (POINT (1 2), LINESTRING (0 0, 1 1))"
        );
        assert!(geojson_geometry(&serde_json::from_str(r#"{"type":"Point","coordinates":["a"]}"#).unwrap()).is_err());
        assert!(is_wkt(&g(r#"{"type":"Point","coordinates":[1,2,3]}"#)));
    }

    #[test]
    fn rejects_non_geojson() {
        assert!(open(&tmp("bad.geojson", "{\"type\":\"Topology\"}")).is_err());
        assert!(open(&tmp("bad2.geojson", "nope")).is_err());
    }

    #[test]
    fn guesses_srid_from_prj() {
        let wgs84 = r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS_1984",6378137.0,298.257223563]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]]"#;
        assert_eq!(srid_from_prj(wgs84), Some(4326));
        let utm = r#"PROJCS["WGS_1984_UTM_Zone_48N",GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984"]],PROJECTION["Transverse_Mercator"]]"#;
        assert_eq!(srid_from_prj(utm), Some(32648));
        assert_eq!(srid_from_prj(&utm.replace("48N", "48S")), Some(32748));
        let epsg = r#"PROJCS["WGS 84 / UTM zone 47N",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],AUTHORITY["EPSG","4326"]],AUTHORITY["EPSG","32647"]]"#;
        assert_eq!(srid_from_prj(epsg), Some(32647));
        assert_eq!(srid_from_prj(r#"PROJCS["Indian_1975_UTM_Zone_47N",GEOGCS["GCS_Indian_1975"]]"#), None);
    }

    #[test]
    fn detects_lonlat_columns() {
        let c = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(detect_lonlat(&c(&["id", "Latitude", "Longitude"])), Some((2, 1)));
        assert_eq!(detect_lonlat(&c(&["lng", "lat"])), Some((0, 1)));
        assert_eq!(detect_lonlat(&c(&["id", "name"])), None);
    }

    #[test]
    fn formats_from_extensions() {
        assert_eq!(Format::from_path(Path::new("a/B.CSV")), Some(Format::Csv));
        assert_eq!(Format::from_path(Path::new("a.geojson")), Some(Format::GeoJson));
        assert_eq!(Format::from_path(Path::new("a.shp")), Some(Format::Shapefile));
        assert_eq!(Format::from_path(Path::new("a.zip")), None);
    }

    #[test]
    fn reads_a_shapefile() {
        use shapefile::{Point, Polygon, PolygonRing, Writer, dbase};
        let dir = std::env::temp_dir().join(format!("rgrip-shp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.shp");
        let table = dbase::TableWriterBuilder::new()
            .add_character_field("name".try_into().unwrap(), 20)
            .add_numeric_field("pop".try_into().unwrap(), 10, 0);
        let mut w = Writer::from_path(&path, table).unwrap();
        let mut rec = dbase::Record::default();
        rec.insert("name".to_string(), dbase::FieldValue::Character(Some("A".into())));
        rec.insert("pop".to_string(), dbase::FieldValue::Numeric(Some(12.0)));
        let ring = PolygonRing::Outer(vec![Point::new(0.0, 0.0), Point::new(0.0, 1.0), Point::new(1.0, 1.0), Point::new(0.0, 0.0)]);
        w.write_shape_and_record(&Polygon::with_rings(vec![ring]), &rec).unwrap();
        drop(w);
        std::fs::write(dir.join("sample.prj"), r#"PROJCS["WGS_1984_UTM_Zone_48N",GEOGCS["GCS_WGS_1984"]]"#).unwrap();

        let o = open(&path).unwrap();
        assert_eq!(o.columns, ["name", "pop", "geom"]);
        assert_eq!(o.srid, Some(32648));
        assert_eq!(o.sample[0][0].as_deref(), Some("A"));
        assert_eq!(o.sample[0][1].as_deref(), Some("12"));
        assert_eq!(o.sample[0][2].as_deref(), Some("MULTIPOLYGON (((0 0, 0 1, 1 1, 0 0)))"));
        std::fs::remove_file(dir.join("sample.prj")).unwrap();
        let o = open(&path).unwrap();
        assert_eq!(o.srid, None);
        assert!(o.note.is_some());
    }
}

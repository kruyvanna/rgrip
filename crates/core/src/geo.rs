//! PostGIS geometry support: EWKB decoding (PostGIS returns geometry and
//! geography as hex EWKB in text mode), WKT output and reprojection of
//! common SRIDs to WGS84 for display on a web map.

use crate::model::ResultSet;

pub type Coord = [f64; 2];

#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    Point(Coord),
    LineString(Vec<Coord>),
    Polygon(Vec<Vec<Coord>>),
    MultiPoint(Vec<Coord>),
    MultiLineString(Vec<Vec<Coord>>),
    MultiPolygon(Vec<Vec<Vec<Coord>>>),
    Collection(Vec<Geometry>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Geom {
    pub srid: Option<i32>,
    pub geometry: Geometry,
}

/// Decode hex (E)WKB as printed by PostGIS for geometry/geography values.
pub fn parse_hex(s: &str) -> Option<Geom> {
    let s = s.trim();
    if s.len() < 18 || s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bytes: Vec<u8> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<Result<_, _>>()
        .ok()?;
    let mut r = Reader { b: &bytes, pos: 0, le: true };
    let mut srid = None;
    let geometry = read_geometry(&mut r, &mut srid, 0)?;
    // Every byte must be accounted for, so random hex strings don't match.
    (r.pos == bytes.len()).then_some(Geom { srid, geometry })
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    le: bool,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let v = self.b.get(self.pos..self.pos + N)?.try_into().ok()?;
        self.pos += N;
        Some(v)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take::<4>().map(|b| if self.le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }
    fn f64(&mut self) -> Option<f64> {
        self.take::<8>().map(|b| if self.le { f64::from_le_bytes(b) } else { f64::from_be_bytes(b) })
    }
    /// A length prefix, sanity-checked against the remaining bytes.
    fn count(&mut self, min_item_bytes: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        (n * min_item_bytes <= self.b.len() - self.pos).then_some(n)
    }
}

fn read_geometry(r: &mut Reader, srid: &mut Option<i32>, depth: usize) -> Option<Geometry> {
    if depth > 32 {
        return None;
    }
    r.le = match r.u8()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let raw = r.u32()?;
    // EWKB flags in the high bits; ISO WKB uses 1000/2000/3000 offsets.
    let (mut z, mut m) = (raw & 0x8000_0000 != 0, raw & 0x4000_0000 != 0);
    let has_srid = raw & 0x2000_0000 != 0;
    let code = raw & 0x0FFF_FFFF;
    let base = code % 1000;
    match code / 1000 {
        0 => {}
        1 => z = true,
        2 => m = true,
        3 => (z, m) = (true, true),
        _ => return None,
    }
    if has_srid {
        let s = r.u32()? as i32;
        if s != 0 {
            *srid = Some(s);
        }
    }
    let dims = 2 + z as usize + m as usize;
    let coord = |r: &mut Reader| -> Option<Coord> {
        let c = [r.f64()?, r.f64()?];
        for _ in 2..dims {
            r.f64()?;
        }
        Some(c)
    };
    let coords = |r: &mut Reader| -> Option<Vec<Coord>> {
        let n = r.count(dims * 8)?;
        (0..n).map(|_| coord(r)).collect()
    };
    Some(match base {
        1 => Geometry::Point(coord(r)?),
        2 => Geometry::LineString(coords(r)?),
        3 => {
            let n = r.count(4)?;
            Geometry::Polygon((0..n).map(|_| coords(r)).collect::<Option<_>>()?)
        }
        4..=7 => {
            let n = r.count(5)?;
            let parts: Vec<Geometry> = (0..n).map(|_| read_geometry(r, srid, depth + 1)).collect::<Option<_>>()?;
            match base {
                4 => Geometry::MultiPoint(
                    parts.into_iter().map(|g| if let Geometry::Point(c) = g { Some(c) } else { None }).collect::<Option<_>>()?,
                ),
                5 => Geometry::MultiLineString(
                    parts.into_iter().map(|g| if let Geometry::LineString(l) = g { Some(l) } else { None }).collect::<Option<_>>()?,
                ),
                6 => Geometry::MultiPolygon(
                    parts.into_iter().map(|g| if let Geometry::Polygon(p) = g { Some(p) } else { None }).collect::<Option<_>>()?,
                ),
                _ => Geometry::Collection(parts),
            }
        }
        _ => return None,
    })
}

impl Geometry {
    pub fn type_name(&self) -> &'static str {
        match self {
            Geometry::Point(_) => "POINT",
            Geometry::LineString(_) => "LINESTRING",
            Geometry::Polygon(_) => "POLYGON",
            Geometry::MultiPoint(_) => "MULTIPOINT",
            Geometry::MultiLineString(_) => "MULTILINESTRING",
            Geometry::MultiPolygon(_) => "MULTIPOLYGON",
            Geometry::Collection(_) => "GEOMETRYCOLLECTION",
        }
    }

    pub fn point_count(&self) -> usize {
        match self {
            Geometry::Point(_) => 1,
            Geometry::LineString(l) | Geometry::MultiPoint(l) => l.len(),
            Geometry::Polygon(p) | Geometry::MultiLineString(p) => p.iter().map(Vec::len).sum(),
            Geometry::MultiPolygon(mp) => mp.iter().flatten().map(Vec::len).sum(),
            Geometry::Collection(c) => c.iter().map(Geometry::point_count).sum(),
        }
    }

    /// Visit every coordinate.
    pub fn for_each_coord(&self, f: &mut impl FnMut(Coord)) {
        match self {
            Geometry::Point(c) => f(*c),
            Geometry::LineString(l) | Geometry::MultiPoint(l) => l.iter().copied().for_each(f),
            Geometry::Polygon(p) | Geometry::MultiLineString(p) => p.iter().flatten().copied().for_each(f),
            Geometry::MultiPolygon(mp) => mp.iter().flatten().flatten().copied().for_each(f),
            Geometry::Collection(c) => c.iter().for_each(|g| g.for_each_coord(f)),
        }
    }

    /// Apply `f` to every coordinate; `None` if any coordinate fails.
    pub fn try_map(&self, f: &impl Fn(Coord) -> Option<Coord>) -> Option<Geometry> {
        let line = |l: &Vec<Coord>| l.iter().map(|c| f(*c)).collect::<Option<Vec<_>>>();
        let rings = |p: &Vec<Vec<Coord>>| p.iter().map(line).collect::<Option<Vec<_>>>();
        Some(match self {
            Geometry::Point(c) => Geometry::Point(f(*c)?),
            Geometry::LineString(l) => Geometry::LineString(line(l)?),
            Geometry::MultiPoint(l) => Geometry::MultiPoint(line(l)?),
            Geometry::Polygon(p) => Geometry::Polygon(rings(p)?),
            Geometry::MultiLineString(p) => Geometry::MultiLineString(rings(p)?),
            Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(mp.iter().map(rings).collect::<Option<_>>()?),
            Geometry::Collection(c) => Geometry::Collection(c.iter().map(|g| g.try_map(f)).collect::<Option<_>>()?),
        })
    }

    pub fn to_wkt(&self) -> String {
        fn c(p: &Coord) -> String {
            format!("{} {}", num(p[0]), num(p[1]))
        }
        fn line(l: &[Coord]) -> String {
            format!("({})", l.iter().map(c).collect::<Vec<_>>().join(", "))
        }
        fn rings(p: &[Vec<Coord>]) -> String {
            format!("({})", p.iter().map(|r| line(r)).collect::<Vec<_>>().join(", "))
        }
        let body = match self {
            Geometry::Point(p) if p[0].is_nan() => return "POINT EMPTY".into(),
            Geometry::Point(p) => format!("({})", c(p)),
            Geometry::LineString(l) => line(l),
            Geometry::MultiPoint(l) => line(l),
            Geometry::Polygon(p) | Geometry::MultiLineString(p) => rings(p),
            Geometry::MultiPolygon(mp) => format!("({})", mp.iter().map(|p| rings(p)).collect::<Vec<_>>().join(", ")),
            Geometry::Collection(g) => format!("({})", g.iter().map(Geometry::to_wkt).collect::<Vec<_>>().join(", ")),
        };
        format!("{} {body}", self.type_name())
    }

    /// GeoJSON geometry object.
    pub fn to_geojson(&self) -> serde_json::Value {
        use serde_json::json;
        let line = |l: &Vec<Coord>| l.iter().map(|c| json!([c[0], c[1]])).collect::<Vec<_>>();
        let rings = |p: &Vec<Vec<Coord>>| p.iter().map(line).collect::<Vec<_>>();
        match self {
            Geometry::Point(c) => json!({"type": "Point", "coordinates": [c[0], c[1]]}),
            Geometry::LineString(l) => json!({"type": "LineString", "coordinates": line(l)}),
            Geometry::MultiPoint(l) => json!({"type": "MultiPoint", "coordinates": line(l)}),
            Geometry::Polygon(p) => json!({"type": "Polygon", "coordinates": rings(p)}),
            Geometry::MultiLineString(p) => json!({"type": "MultiLineString", "coordinates": rings(p)}),
            Geometry::MultiPolygon(mp) => {
                json!({"type": "MultiPolygon", "coordinates": mp.iter().map(rings).collect::<Vec<_>>()})
            }
            Geometry::Collection(g) => {
                json!({"type": "GeometryCollection", "geometries": g.iter().map(Geometry::to_geojson).collect::<Vec<_>>()})
            }
        }
    }
}

impl Geom {
    /// Short form for grid cells, e.g. `POINT (104.93 11.56)` or `POLYGON · 5 pts`.
    pub fn summary(&self) -> String {
        let srid = self.srid.map(|s| format!("  ·  {s}")).unwrap_or_default();
        match &self.geometry {
            Geometry::Point(p) if !p[0].is_nan() => format!("POINT ({} {}){srid}", short(p[0]), short(p[1])),
            g => format!("{} · {} pts{srid}", g.type_name(), g.point_count()),
        }
    }

    /// The geometry in WGS84 lon/lat, if its SRID is one we can convert.
    pub fn to_wgs84(&self) -> Option<Geometry> {
        let srid = self.srid.unwrap_or(4326);
        self.geometry.try_map(&|c| to_wgs84(srid, c))
    }
}

fn num(v: f64) -> String {
    let s = format!("{v}");
    if s.contains('e') { format!("{v:.10}") } else { s }
}

fn short(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Convert a coordinate to WGS84 lon/lat. Supports geographic SRIDs,
/// Web Mercator and UTM zones (WGS84 datum).
pub fn to_wgs84(srid: i32, c: Coord) -> Option<Coord> {
    match srid {
        0 | 4326 | 4269 | 4258 | 4979 | 4283 => {
            (c[0].abs() <= 180.0 && c[1].abs() <= 90.0).then_some(c)
        }
        3857 | 900913 | 3785 | 102100 | 102113 => {
            const R: f64 = 6_378_137.0;
            let lon = (c[0] / R).to_degrees();
            let lat = (2.0 * (c[1] / R).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
            Some([lon, lat])
        }
        32601..=32660 => Some(utm_to_wgs84(c, (srid - 32600) as u8, true)),
        32701..=32760 => Some(utm_to_wgs84(c, (srid - 32700) as u8, false)),
        _ => None,
    }
}

/// Inverse transverse Mercator (Snyder, "Map Projections", USGS 1987).
fn utm_to_wgs84(c: Coord, zone: u8, north: bool) -> Coord {
    let a: f64 = 6_378_137.0;
    let f: f64 = 1.0 / 298.257_223_563;
    let k0 = 0.9996;
    let e2 = f * (2.0 - f);
    let ep2 = e2 / (1.0 - e2);
    let x = c[0] - 500_000.0;
    let y = if north { c[1] } else { c[1] - 10_000_000.0 };
    let lon0 = ((zone as f64 - 1.0) * 6.0 - 180.0 + 3.0).to_radians();

    let m = y / k0;
    let mu = m / (a * (1.0 - e2 / 4.0 - 3.0 * e2 * e2 / 64.0 - 5.0 * e2.powi(3) / 256.0));
    let e1 = (1.0 - (1.0 - e2).sqrt()) / (1.0 + (1.0 - e2).sqrt());
    let phi1 = mu
        + (3.0 * e1 / 2.0 - 27.0 * e1.powi(3) / 32.0) * (2.0 * mu).sin()
        + (21.0 * e1 * e1 / 16.0 - 55.0 * e1.powi(4) / 32.0) * (4.0 * mu).sin()
        + (151.0 * e1.powi(3) / 96.0) * (6.0 * mu).sin()
        + (1097.0 * e1.powi(4) / 512.0) * (8.0 * mu).sin();

    let (sin1, cos1, tan1) = (phi1.sin(), phi1.cos(), phi1.tan());
    let n1 = a / (1.0 - e2 * sin1 * sin1).sqrt();
    let t1 = tan1 * tan1;
    let c1 = ep2 * cos1 * cos1;
    let r1 = a * (1.0 - e2) / (1.0 - e2 * sin1 * sin1).powf(1.5);
    let d = x / (n1 * k0);

    let lat = phi1
        - (n1 * tan1 / r1)
            * (d * d / 2.0 - (5.0 + 3.0 * t1 + 10.0 * c1 - 4.0 * c1 * c1 - 9.0 * ep2) * d.powi(4) / 24.0
                + (61.0 + 90.0 * t1 + 298.0 * c1 + 45.0 * t1 * t1 - 252.0 * ep2 - 3.0 * c1 * c1) * d.powi(6) / 720.0);
    let lon = lon0
        + (d - (1.0 + 2.0 * t1 + c1) * d.powi(3) / 6.0
            + (5.0 - 2.0 * c1 + 28.0 * t1 - 3.0 * c1 * c1 + 8.0 * ep2 + 24.0 * t1 * t1) * d.powi(5) / 120.0)
            / cos1;
    [lon.to_degrees(), lat.to_degrees()]
}

/// SRID declared by a column type such as `geometry(Point,32648)`;
/// `Some(0)` for an unconstrained geometry, `None` for non-spatial types.
pub fn column_srid(ty: &str) -> Option<i32> {
    let t = ty.trim().to_ascii_lowercase();
    let (default, rest) = if let Some(r) = t.strip_prefix("geometry") {
        (0, r)
    } else if let Some(r) = t.strip_prefix("geography") {
        (4326, r)
    } else {
        return None;
    };
    if !(rest.is_empty() || rest.starts_with('(') || rest.starts_with('[')) {
        return None;
    }
    let srid = rest.trim_start_matches('(').split(')').next().and_then(|m| m.split(',').nth(1)).and_then(|s| s.trim().parse().ok());
    Some(srid.unwrap_or(default))
}

/// Columns whose sampled non-null values all decode as geometries.
pub fn geometry_columns(rs: &ResultSet) -> Vec<usize> {
    (0..rs.columns.len())
        .filter(|&i| {
            let mut sample = rs.rows.iter().filter_map(|r| r[i].as_deref()).take(20).peekable();
            sample.peek().is_some() && sample.all(|v| parse_hex(v).is_some())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // All fixtures are real PostGIS 3.4 output.
    // select st_setsrid(st_makepoint(104.9311, 11.5637), 4326)
    const POINT: &str = "0101000020E61000008F537424973B5A40AA8251499D202740";

    #[test]
    fn parses_ewkb_point() {
        let g = parse_hex(POINT).unwrap();
        assert_eq!(g.srid, Some(4326));
        assert_eq!(g.geometry, Geometry::Point([104.9311, 11.5637]));
        assert_eq!(g.geometry.to_wkt(), "POINT (104.9311 11.5637)");
        assert_eq!(g.summary(), "POINT (104.9311 11.5637)  ·  4326");
    }

    #[test]
    fn parses_plain_wkb_polygon() {
        // st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))'), no SRID
        let hex = "0103000000010000000400000000000000000000000000000000000000000000000000F03F0000000000000000000000000000F03F000000000000F03F00000000000000000000000000000000";
        let g = parse_hex(hex).unwrap();
        assert_eq!(g.srid, None);
        assert_eq!(g.geometry.to_wkt(), "POLYGON ((0 0, 1 0, 1 1, 0 0))");
        assert_eq!(g.summary(), "POLYGON · 4 pts");
    }

    #[test]
    fn parses_z_and_collections() {
        // POINT Z (1 2 3), SRID 4326: the Z value is skipped.
        let g = parse_hex("01010000A0E6100000000000000000F03F00000000000000400000000000000840").unwrap();
        assert_eq!(g.geometry, Geometry::Point([1.0, 2.0]));
        let gc = "0107000020E6100000020000000101000000000000000000F03F000000000000004001020000000200000000000000000000000000000000000000000000000000F03F000000000000F03F";
        let g = parse_hex(gc).unwrap();
        assert_eq!(g.geometry.to_wkt(), "GEOMETRYCOLLECTION (POINT (1 2), LINESTRING (0 0, 1 1))");
        assert_eq!(g.srid, Some(4326));
    }

    #[test]
    fn column_srids() {
        assert_eq!(column_srid("geometry(Point,32648)"), Some(32648));
        assert_eq!(column_srid("geometry(MultiPolygon, 4326)"), Some(4326));
        assert_eq!(column_srid("geometry"), Some(0));
        assert_eq!(column_srid("geography(Point)"), Some(4326));
        assert_eq!(column_srid("geometry[]"), Some(0));
        assert_eq!(column_srid("geometrytype"), None);
        assert_eq!(column_srid("text"), None);
    }

    #[test]
    fn rejects_non_geometry() {
        for s in ["", "hello", "deadbeef", "0101000020E6100000", "c4ca4238a0b923820dcc509a6f75849b"] {
            assert!(parse_hex(s).is_none(), "{s}");
        }
    }

    #[test]
    fn reprojects() {
        // Royal Palace (104.9311, 11.5637) via PostGIS st_transform.
        let close = |c: Coord| (c[0] - 104.9311).abs() < 1e-7 && (c[1] - 11.5637).abs() < 1e-7;
        let merc = to_wgs84(3857, [11_680_876.620378068, 1_296_094.3233966087]).unwrap();
        assert!(close(merc), "{merc:?}");
        let utm = to_wgs84(32648, [492_487.761098532, 1_278_310.0837367245]).unwrap();
        assert!(close(utm), "{utm:?}");
        assert!(to_wgs84(2263, [0.0, 0.0]).is_none());
        assert!(to_wgs84(4326, [500.0, 0.0]).is_none());
    }
}

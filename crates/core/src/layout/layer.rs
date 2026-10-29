use super::*;

impl Layer {
    pub fn new(id: u64, schema: &str, table: &str, column: &str, color: [u8; 3]) -> Self {
        Self {
            id,
            schema: schema.into(),
            table: table.into(),
            column: column.into(),
            filter: String::new(),
            label: table.into(),
            color,
            width: 0.35,
            point: 1.2,
            fill_opacity: 0.35,
            visible: true,
            style: Style::Single,
            shapes: Default::default(),
            values: Default::default(),
            extent: None,
            loaded: None,
            total: None,
        }
    }

    fn table_ref(&self) -> String {
        format!("{}.{}", crate::sql::quote_ident(&self.schema), crate::sql::quote_ident(&self.table))
    }

    fn geom(&self) -> String {
        format!("{}::geometry", crate::sql::quote_ident(&self.column))
    }

    fn filter_and(&self) -> String {
        let f = self.filter.trim();
        if f.is_empty() { String::new() } else { format!(" and ({f})") }
    }

    pub fn category_column(&self) -> Option<&str> {
        match &self.style {
            Style::Categories { column, .. } => Some(column),
            Style::Single => None,
        }
    }

    /// The features inside `bbox` (lon/lat), in WGS84 and simplified to
    /// about 0.15 mm at `scale`, plus the category column's value. PostGIS
    /// does the reprojection, so any SRID works; the bbox uses the index.
    pub fn data_sql(&self, bbox: [f64; 4], scale: f64, limit: usize) -> String {
        let (t, g) = (self.table_ref(), self.geom());
        let tol = scale / 1000.0 * 0.15 / 111_320.0;
        let cat = self.category_column().map_or(String::new(), |c| format!(", ({})::text", crate::sql::quote_ident(c)));
        let [x0, y0, x1, y1] = [bbox[0].max(-180.0), bbox[1].max(-85.0), bbox[2].min(180.0), bbox[3].min(85.0)];
        format!(
            "with s as (select ST_SRID({g}) as srid from {t} where {g} is not null limit 1) \
             select ST_SimplifyPreserveTopology(case when ST_SRID({g}) = 0 then ST_SetSRID({g}, 4326) else ST_Transform({g}, 4326) end, {tol:e}){cat} \
             from {t} where {g} is not null \
             and ((select srid from s) = 0 or {g} && ST_Transform(ST_MakeEnvelope({x0}, {y0}, {x1}, {y1}, 4326), (select srid from s))){} limit {limit}",
            self.filter_and()
        )
    }

    /// The whole layer's extent, as a WGS84 geometry.
    pub fn extent_sql(&self) -> String {
        let (t, g) = (self.table_ref(), self.geom());
        let f = self.filter.trim();
        let filter = if f.is_empty() { String::new() } else { format!(" where {f}") };
        format!(
            "with s as (select coalesce(nullif(ST_SRID({g}), 0), 4326) as srid from {t} where {g} is not null limit 1) \
             select ST_Transform(ST_SetSRID(ST_Extent({g})::geometry, (select srid from s)), 4326), count(*) from {t}{filter}"
        )
    }

    /// Whether the loaded shapes no longer cover `view` (lon/lat) or were
    /// simplified for a very different scale.
    pub fn needs_reload(&self, view: [f64; 4], scale: f64) -> bool {
        match self.loaded {
            None => true,
            Some((b, s)) => view[0] < b[0] || view[1] < b[1] || view[2] > b[2] || view[3] > b[3] || scale < s / 2.0 || scale > s * 2.0,
        }
    }

    /// Legend rows for this layer.
    pub(super) fn legend_rows(&self, out: &mut Vec<LegendRow>) {
        match &self.style {
            Style::Single => out.push(LegendRow { label: self.label.clone(), swatch: Some((self.kind(), self.color)), indent: false }),
            Style::Categories { classes, more, .. } => {
                // Nothing of it on the map: nothing in the legend either.
                if !*more && !Style::shown(classes).any(|c| c.visible) {
                    return;
                }
                out.push(LegendRow { label: self.label.clone(), swatch: None, indent: false });
                for c in Style::shown(classes).filter(|c| c.visible) {
                    out.push(LegendRow { label: c.label.clone(), swatch: Some((self.kind(), c.color)), indent: true });
                }
                if *more {
                    out.push(LegendRow { label: "Other".into(), swatch: Some((self.kind(), self.color)), indent: true });
                }
            }
        }
    }

    pub fn kind(&self) -> Kind {
        fn of(g: &Geometry) -> Kind {
            match g {
                Geometry::Point(_) | Geometry::MultiPoint(_) => Kind::Point,
                Geometry::LineString(_) | Geometry::MultiLineString(_) => Kind::Line,
                Geometry::Polygon(_) | Geometry::MultiPolygon(_) => Kind::Polygon,
                Geometry::Collection(c) => c.first().map_or(Kind::Point, of),
            }
        }
        self.shapes.first().map_or(Kind::Polygon, of)
    }
}

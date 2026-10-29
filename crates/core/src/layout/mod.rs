//! Map layouts: a page with a map frame, layers from tables, and a title,
//! legend, scale bar and north arrow placed on the page. Rendered to one SVG
//! (in millimetres); the app turns that SVG into the preview, a PNG or a PDF,
//! so all three match.

use crate::geo::{Coord, Geometry};
use std::fmt::Write;

/// Millimetres per typographic point.
pub const PT: f64 = 25.4 / 72.0;
/// Web Mercator sphere radius.
const R: f64 = 6_378_137.0;
/// Tiles fetched for one render at most; past that the tile zoom drops (the
/// basemap gets softer instead of the render getting slow or abusive).
const MAX_TILES: usize = 160;

mod elements;
mod graticule;
mod layer;
mod measure;
mod model;
mod page;
mod svg;
#[cfg(test)]
mod tests;
mod tiles;

pub use self::{graticule::*, measure::*, model::*, page::*, svg::*, tiles::*};
use self::elements::*;

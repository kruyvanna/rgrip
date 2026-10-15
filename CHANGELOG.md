# Changelog

All notable changes to rgrip are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed
- Passwords, SSH passwords and key passphrases are stored in the macOS Keychain instead of plain text in `connections.toml`. Existing plain-text secrets migrate automatically; if the Keychain is unavailable rgrip warns once and falls back to the file.

## [0.0.8-alpha] - 2026-10-14

### Added
- Smart relations: Related also follows relations inferred from column names and types when there is no foreign key (`orders.customer_id` → `customers.id`, plurals, shared key names like `parcel_code`). Inferred ones are marked `~ inferred`; generic keys such as `objectid` are ignored.
- Go to column: ⌘J, the corner button of any grid, or the header menu. Search, jump and the column flashes.
- Sort indicators: painted arrows on sorted columns, a hint on sortable headers.
- Connection colors: tint every tab of a connection (e.g. red for production); console tabs are titled `name@database`.
- Schema picker per console: queries, paging and Export all run with that schema on the `search_path`; autocomplete and slash commands prefer it.
- Configurable slash commands (Settings → Slash commands…): edit the built-ins or add your own SQL templates with `{table}`, `{field}`, `{field+_4326}` and `{n=100}`, with a live preview. Saved to `commands.toml`.
- Title bar buttons: appearance (System / Light / Dark), check for updates (or "Update to x.y"), GitHub and Sponsor.

## [0.0.7-alpha] - 2026-10-13

### Added
- Date/time picker for inline cell edits, matched to the column type (date, timestamp, timestamptz, time, timetz), with Today / Now in local time. Offsets and fractional seconds are kept.
- Related rows: right-click rows, then Related, to follow foreign keys in both directions (→ users, ← posts) in a popup. Chain further, go Back, or open the result as a tab. Works with self-references and multi-column keys.
- Freeze (pin) columns: right-click a column header, then Freeze column. Frozen columns and row numbers stay on the left while scrolling sideways, and survive reloads and re-runs.
- Landing page, screenshots, dual MIT/Apache-2.0 license, contributing guide and issue templates.

### Fixed
- Map: polygons with a repeated vertex no longer draw spikes across the map.

## [0.0.6-alpha] - 2026-10-13

### Added
- Inline cell editing in table tabs: double-click a cell, type, Enter to keep (Esc cancels). Right-click for Edit cell, Set NULL and Revert cell.
  - Changed cells are highlighted; the toolbar shows N changes with Save and Discard. Save (⌘S) writes one `UPDATE` per row, matched on the primary key, in a single transaction: if any row was changed or deleted meanwhile, nothing is saved.
  - Tables without a primary key, and views, are read-only. Geometry columns are not editable. Paging, sorting and filtering wait until you Save or Discard.
- Copy as image: right-click rows, then Copy as image (or Export, Copy as, Image (PNG)) to put a clean table picture on the clipboard.

### Fixed
- Khmer in table images renders correctly (stacked consonants, mixed Latin/Khmer, no dropped letters).
- No more empty boxes in Khmer text (zero-width spaces, stray vowel marks).
- Map popup: proper copy and close icons, lined up in one column.

## [0.0.5-alpha] - 2026-10-13

### Added
- Format query: ⇧⌘F or the Format button formats the selection, or the whole editor. Slash command lines and comments are kept.
- SVG table export: Save rows as, SVG table (or Copy as, SVG table to paste into Figma), also in Export. Bold header, zebra rows, right-aligned numbers, grey NULLs, geometries as WKT.

### Changed
- Slash commands always expand to a complete query: `/con_4326` alone gives `select t.*, ST_Transform(t.shape, 4326) as shape_4326 from table_name t;`.
- `/con_merc` source SRID defaults to 32648 (UTM 48N); add a number to override, e.g. `/con_merc parcels 3857`.
- Table names in slash commands match regardless of case, and a table the catalog does not know is used as typed.
- Run and Run all expand slash command lines before sending.

### Fixed
- A `-- comment` after the final `;` no longer causes a syntax error.
- Trailing `/* ... */` comments are no longer treated as a separate statement.

## [0.0.4-alpha] - 2026-10-13

### Added
- Slash commands in the editor (Tab to expand, or ⌘↩ to expand and run): `/con_4326`, `/con_4326_2` (ArcGIS via text), `/con_merc`, `/srid`, `/top`, `/count`. Add a field or a table to write the whole query.
- ArcGIS `st_geometry` columns get the same conversions as autocomplete suggestions.
- Connection strings: paste a `postgres://` URL, a `host=... dbname=...` string or a `jdbc:postgresql://` URL into the URL field. Right-click a connection, then Copy connection URL.
- Multi-row selection (click, ⇧-click, ⌘-click, ⌘A, ⌘C) with Copy as CSV / JSON / Markdown / GeoJSON / SQL INSERT / `where id in (...)`, Copy for AI agent (query, column types and rows as Markdown), and Save rows as.
- ⌘N opens a new query.
- Map: features with a vertex far from the rest (e.g. swapped lat/lon) are flagged ("N far off", Hide), and Fit zooms to the real data.
- Map: the feature popup is selectable, with copy buttons (geometries copy as WKT).
- Map: queries from `/con_*` show their `*_4326` column automatically.

## [0.0.3-alpha] - 2026-10-13

### Added
- PostGIS autocomplete: `ST_*` functions with arguments, plus generated SQL for each geometry column based on its SRID (e.g. `ST_Transform(geom, 4326) AS geom`, `ST_AsGeoJSON`, `ST_AsText`, Web Mercator transform).
- Reproject to 4326: when the map cannot draw a geometry's SRID itself, one click re-runs the query with `ST_Transform(..., 4326)`.
- Light basemap (Esri Light Gray), the default in light themes.
- Settings: System / Light / Dark switch with icons, and compact theme lists with color swatches.

### Fixed
- The theme menu no longer stretches across the whole window.
- PostGIS helper schemas (`tiger`, `topology`) no longer clutter autocomplete.

## [0.0.2-alpha] - 2026-10-13

### Added
- Themes: Settings, Appearance (System / Light / Dark), with Dracula, Nord, One Dark Pro, Tokyo Night, Monokai, Catppuccin Mocha and Latte, Gruvbox Dark, GitHub Dark and Light, Solarized Dark and Light, One Light. System follows macOS and switches live.
- Over-the-air updates: rgrip checks GitHub Releases on launch (can be turned off) or via Settings, Check for updates. Install & restart downloads the update, verifies its SHA-256 and replaces the app in place.
- Saved queries are `.sql` files in `~/Library/Application Support/dev.rgrip.rgrip/queries/`. Reveal in Finder, edit in any editor; deleting moves them to the Trash.

### Fixed
- The app icon now has a transparent background (no white square in the Dock).

## [0.0.1-alpha] - 2026-10-13

First alpha: a minimal DataGrip-style PostgreSQL client written in Rust.

### Added
- Connections: SSL from `disable` to `verify-full`, custom CA and client certificates; SSH tunnels with a password or key.
- Explorer: schemas, tables, views and columns, plus saved queries.
- Console: syntax highlighting, line numbers and context-aware autocomplete; ⌘↩ runs a statement, ⇧⌘↩ runs everything, ⌘S saves the query.
- Results: virtualized grid with 1,000-row server-side paging, sorting and filtering.
- Export: CSV, TSV, JSON, GeoJSON, Markdown and SQL INSERT, for the current page or all rows.
- PostGIS: readable geometry cells and a built-in map (dark, streets and satellite basemaps); WGS84, Web Mercator and UTM are reprojected automatically.
- Khmer: correctly shaped text in the grid, explorer and map.

### Known limitations
- Passwords are stored in plain text in `connections.toml` (file mode 0600).
- Khmer typed into the SQL editor is not shaped yet.
- Only macOS on Apple silicon is published; other platforms can build from source.

[Unreleased]: https://github.com/yoouTaro/rgrip/compare/v0.0.8-alpha...HEAD
[0.0.8-alpha]: https://github.com/yoouTaro/rgrip/releases/tag/v0.0.8-alpha
[0.0.7-alpha]: https://github.com/yoouTaro/rgrip/releases/tag/v0.0.7-alpha
[0.0.6-alpha]: https://github.com/yoouTaro/rgrip/releases/tag/v0.0.6-alpha
[0.0.5-alpha]: https://github.com/yoouTaro/rgrip/releases/tag/v0.0.5-alpha
[0.0.4-alpha]: https://github.com/yoouTaro/rgrip/releases/tag/v0.0.4-alpha
[0.0.3-alpha]: https://github.com/yoouTaro/rgrip/releases/tag/v0.0.3-alpha
[0.0.2-alpha]: https://github.com/yoouTaro/rgrip/releases/tag/v0.0.2-alpha
[0.0.1-alpha]: https://github.com/yoouTaro/rgrip/releases/tag/v0.0.1-alpha

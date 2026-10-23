<p align="center"><img src="assets/icon-512.png" width="112" alt="rgrip icon"></p>

<h1 align="center">rgrip</h1>

<p align="center">A small Postgres client, with a map.</p>

<p align="center">
  <a href="https://github.com/yoouTaro/rgrip/releases"><img alt="Latest release" src="https://img.shields.io/github/v/release/yoouTaro/rgrip?include_prereleases&label=release"></a>
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue">
  <img alt="Platform: macOS" src="https://img.shields.io/badge/platform-macOS-lightgrey?logo=apple">
</p>

<p align="center">
  <a href="https://yoouTaro.github.io/rgrip/">Website</a> ·
  <a href="https://github.com/yoouTaro/rgrip/releases">Download</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

<p align="center"><img src="docs/clips/linked.gif" alt="Split view: rows selected in the grid light up on the map" width="900"></p>

A native PostgreSQL client for macOS, with a map for PostGIS data built in. For everyday Postgres work, and for checking and loading spatial data without opening a desktop GIS. One Rust binary, about a 10 MB download.

## Highlights

### Map and grid, linked

Show any query or table on a map, or put the grid and the map side by side. Rows selected in the grid are highlighted on the map, and clicking a feature selects its row. You can zoom to the selected rows, or draw a box on the map to filter the table by area. Data is reprojected in the client (WGS84, Web Mercator, UTM), and the basemap can be dark, light, streets or satellite.

### Import data

Load a CSV, TSV, GeoJSON or Shapefile into a new or existing table. You get a preview, editable column names and types, and an SRID (guessed from the `.prj` for shapefiles). Rows go in inside one transaction, so an error rolls everything back. New tables get a primary key by default, so the rows are editable right away.

<p align="center"><img src="docs/screenshots/import-markets.png" alt="Import data dialog" width="700"></p>

### Map layouts and export

Put one or more tables on a page (A4, A3, Letter, portrait or landscape) and export it as a PDF or PNG. Drag the title, legend, scale bar and north arrow where you want them; they snap to the page and the frame. Color a layer by a column and rename the categories in the legend. Only the data in view is loaded, and the graticule labels sit outside the frame. The plain map view also has a quick PNG export.

<p align="center"><img src="docs/screenshots/layout.png" alt="Map layout tab: provinces of Cambodia on an A4 page with a graticule, legend, scale bar and north arrow" width="900"></p>

## Also included

- SQL console with autocomplete, a formatter and slash commands (editable, with your own templates)
- Inline editing of table cells, saved in one transaction
- Edit table: rename, add, retype and drop columns, defaults and the primary key, with the SQL shown first
- Add primary key to tables that have none
- Related rows across foreign keys, including relations inferred from column names
- Production safety: colored connections, read-only connections, confirmation before risky statements
- SSL up to `verify-full`, SSH tunnels, passwords in the macOS Keychain
- Export to CSV, TSV, JSON, GeoJSON, Markdown, SQL INSERT and SVG; copy rows as an image or for an AI agent
- Sort query results by clicking a column header
- Dark, light and system themes, and in-app updates

<table>
  <tr>
    <td><img src="docs/clips/edit.gif" alt="Inline editing"><br><sub>Inline editing with pending changes</sub></td>
    <td><img src="docs/clips/relations.gif" alt="Inferred relation"><br><sub>Related rows through an inferred relation</sub></td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/tabs.png" alt="Connection colors"><br><sub>Color-coded connection tabs</sub></td>
    <td><img src="docs/clips/commands.gif" alt="Slash commands"><br><sub>Slash commands with a live preview</sub></td>
  </tr>
</table>

## Install

Run this in Terminal (Apple silicon):

```sh
curl -fsSL https://raw.githubusercontent.com/yoouTaro/rgrip/main/install.sh | sh
```

It downloads the latest release, checks its SHA-256, puts **rgrip.app** in Applications and opens it. Run it again to update.

Or download the zip from [Releases](https://github.com/yoouTaro/rgrip/releases), unzip it and move **rgrip.app** to Applications. It isn't notarized, so macOS blocks the first launch of a downloaded zip: right-click **rgrip.app** and choose **Open**, or run:

```sh
xattr -dr com.apple.quarantine /Applications/rgrip.app
```

After that, rgrip updates itself: ⚙ → **Check for updates…** → **Install & restart**.

<details>
<summary>Keyboard shortcuts</summary>

| Shortcut | Action |
|---|---|
| ⌘↩ | Run statement |
| ⇧⌘↩ | Run all (script) |
| ⌘S | Save query / save inline edits |
| ⇧⌘F | Format query |
| ⌘/ | Comment or uncomment lines |
| ⌘N | New query |
| ⌘W | Close tab |
| ⌘J | Go to column (in a results grid) |
| ⌘R | Reload a table tab |
| Tab | Expand slash command |
| Esc | Close popups / cancel cell edit |

</details>

## Build from source

Requires Rust 1.88+.

```sh
cargo run -p rgrip
```

To build the app bundle yourself:

```sh
./scripts/bundle-macos.sh   # dist/rgrip.app and dist/rgrip-<version>-macos-<arch>.zip
./scripts/build-icon.sh     # regenerate assets/rgrip.icns from assets/logo.svg
```

<details>
<summary>Development and tests</summary>

```sh
cargo test
```

`crates/core/tests/pg.rs` runs against a local Postgres on `localhost:5432` (skipped if unreachable). `crates/core/tests/remote.rs` needs the dev environment, PostGIS with SSL and an SSH bastion:

```sh
./dev/gen-certs.sh
docker compose -f dev/docker-compose.yml up -d --build
cargo test
```

| Path | Contents |
|---|---|
| `crates/core` | Postgres driver (tokio-postgres + rustls), SSH tunnel, SQL splitting, autocomplete, export, PostGIS decoding, inline-edit SQL, date/time |
| `crates/app` | egui UI: explorer, console, grid, map, dialogs |
| `dev/` | Docker environment for integration tests |
| `scripts/` | macOS bundling and icon scripts |
| `site/` | Landing page (GitHub Pages) |

</details>

<details>
<summary>Configuration</summary>

Connections and saved queries are stored in `~/Library/Application Support/dev.rgrip.rgrip/` (`connections.toml`, and `.sql` files under `queries/`, one level of subfolders for query folders).

Passwords (database, SSH password and key passphrase) are stored in the macOS Keychain under the service `dev.rgrip.rgrip`, not in `connections.toml`. Older plain-text passwords are moved to the Keychain on first launch. If the Keychain is unavailable, rgrip shows a notice and keeps them in the file (mode 0600).

</details>

## How it's made

rgrip is built with an AI coding agent ([Claude Code](https://claude.com/claude-code)), and a human reviews and tests the changes. It is alpha software, so please keep backups.

## Contributing

Bug reports, ideas and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Support

If rgrip is useful to you, you can support it through [GitHub Sponsors](https://github.com/sponsors/yoouTaro).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

<p align="center"><img src="assets/icon-512.png" width="128" alt="rgrip"></p>

# rgrip

A minimal, DataGrip-style PostgreSQL client written in Rust ([egui](https://github.com/emilk/egui)).

- **Connections**: SSL (`disable` … `verify-full`, custom CA, client certificates) and SSH tunnels (password or key)
- **Explorer**: schemas, tables, views and columns, loaded on demand
- **Console**: SQL editor with highlighting, line numbers and context-aware autocomplete; run statement (⌘↩) or script (⇧⌘↩); saved queries (⌘S)
- **Results**: virtualized grid, 1,000-row server-side paging, export to CSV / TSV / JSON / GeoJSON / Markdown / SQL INSERT
- **PostGIS**: readable geometry cells and a built-in map (WGS84, Web Mercator and UTM are reprojected client-side)
- **Khmer**: shaped rendering for complex scripts in the grid, explorer and map

## Build and run

Requires Rust 1.88+ (egui 0.33).

```sh
cargo run -p rgrip
```

Download the macOS app (Apple silicon) from [Releases](https://github.com/yoouTaro/rgrip/releases).
It is ad-hoc signed, not notarized: on first launch right-click **rgrip.app → Open**, or run
`xattr -dr com.apple.quarantine rgrip.app`.

To build the app bundle yourself:

```sh
./scripts/bundle-macos.sh   # → dist/rgrip.app and dist/rgrip-<version>-macos-<arch>.zip
./scripts/build-icon.sh     # regenerate assets/rgrip.icns from assets/logo.svg
```

Connections are stored in `~/Library/Application Support/dev.rgrip.rgrip/` (`connections.toml`, `queries.toml`).
Passwords are currently stored in plain text there (file mode 0600).

## Layout

| Crate | Contents |
|---|---|
| `crates/core` | Postgres driver (tokio-postgres + rustls), SSH tunnel, SQL splitting, autocomplete, export, PostGIS decoding |
| `crates/app` | egui UI: explorer, console, grid, map, dialogs |

## Tests

```sh
cargo test
```

`crates/core/tests/pg.rs` runs against a local Postgres on `localhost:5432` (skipped if unreachable).
`crates/core/tests/remote.rs` needs the dev environment — PostGIS with SSL and an SSH bastion:

```sh
./dev/gen-certs.sh
docker compose -f dev/docker-compose.yml up -d --build
cargo test
```

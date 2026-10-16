# Product

<!-- impeccable:product-schema 1 -->

## Platform

web (the landing site in `site/`; the product itself is a native macOS app built with Rust and egui)

## Users

Two audiences, weighted equally:

- Postgres developers who want a small, fast native client for everyday querying, browsing and editing.
- People working with spatial data in PostGIS who want to see it on a map, check it and load it without opening a GIS desktop app.

## Product Purpose

rgrip is a free, open-source native PostgreSQL client with a built-in map for PostGIS data. It exists so that querying, looking at and fixing data (including geometry) happens in one small app. Success: someone downloads it, connects, and gets value from the first query.

## Positioning

A small native Postgres client where the map is part of the client, not a separate GIS tool: the grid and the map share one selection, and spatial files go straight into tables.

## Operating Context

Used on a Mac next to a terminal and an editor, against local, staging and production databases (often over SSL or an SSH tunnel). Distributed as an Apple silicon zip on GitHub Releases, with in-app updates.

## Capabilities and Constraints

Must-feature showcase for the landing page (confirmed): **Map + linked grid** (map view, split view, shared selection, zoom to selected, filter by drawn area) and **Import data** (CSV/TSV, GeoJSON, Shapefile into a new or existing table, with preview, types and SRID). Everything else (SQL console, inline editing, Edit table, relations, production safety, export, slash commands, shortcuts) is supporting and stays brief.

Constraints: macOS, Apple silicon only for now; not notarized. Licensed MIT OR Apache-2.0. Releases are tagged `vX.Y.Z-alpha`.

## Brand Commitments

- Name: rgrip, lowercase. App icon in `site/img/icon.png`.
- Voice: modest and plain. Don't sound proud or salesy. No Khmer showcase on the site.
- Icons: line icons (Lucide style), no emoji.
- Donations go through GitHub Sponsors.
- The site stays familiar and easy to read: a standard developer-tool page at the craft level of Zed and Linear. A themed concept (a split-flap board, 2026-10-16) was tried and dropped as too hard to read.

## Evidence on Hand

Real screenshots and clips from scripted scenes against the demo PostGIS database (`scripts/screenshots.sh`, `scripts/record.sh`; outputs in `docs/screenshots`, `docs/clips`, `site/img`, `site/clips`). Measured on 0.0.9: 10.3 MB zip (~10 MB), ~65 MB memory at idle with no connection open. No testimonials, user counts or benchmarks exist; don't invent any.

## Product Principles

1. Show the data: the map and the grid are the product, so real screenshots lead.
2. Small and fast over feature lists.
3. Safe by default with real databases: transactions, confirmations, read-only.
4. Say what it does plainly; let the screenshots carry the weight.

---
name: rgrip
description: A small Postgres client with a map, presented as one split-flap departure board.
colors:
  amber: "#f2b33d"
  amber-hover: "#ffc552"
  amber-ink: "#17130a"
  ground: "#101113"
  board: "#141518"
  cell: "#1d1e21"
  cell-lo: "#17181b"
  seam: "#060607"
  frame: "#2b2d32"
  steel-edge: "#3a3d43"
  steel-face: "#1b1c1f"
  well: "#0b0b0c"
  line: "#2a2c31"
  ink: "#f2f2ee"
  muted: "#a6a9b0"
  faint: "#7c8088"
typography:
  display:
    fontFamily: "Big Shoulders, Arial Narrow, sans-serif"
    fontSize: "calc(min(74px, (min(100vw, 1200px) - 140px) / 15.3) * 0.8)"
    fontWeight: 600
    lineHeight: 1
    fontVariation: "\"opsz\" 72"
  headline:
    fontFamily: "Big Shoulders, Arial Narrow, sans-serif"
    fontSize: "clamp(32px, 3.4vw, 44px)"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "0.01em"
  title:
    fontFamily: "Big Shoulders, Arial Narrow, sans-serif"
    fontSize: "26px"
    fontWeight: 600
    lineHeight: 1
    letterSpacing: "0.08em"
  title-sm:
    fontFamily: "Big Shoulders, Arial Narrow, sans-serif"
    fontSize: "18px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "0.06em"
  figure:
    fontFamily: "Big Shoulders, Arial Narrow, sans-serif"
    fontSize: "21px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "0.03em"
    fontFeature: "\"tnum\" 1"
  button:
    fontFamily: "Big Shoulders, Arial Narrow, sans-serif"
    fontSize: "19px"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "0.06em"
  nav:
    fontFamily: "Big Shoulders, Arial Narrow, sans-serif"
    fontSize: "17px"
    fontWeight: 600
    lineHeight: 1
    letterSpacing: "0.07em"
  label:
    fontFamily: "Big Shoulders, Arial Narrow, sans-serif"
    fontSize: "13px"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "0.16em"
  lead:
    fontFamily: "-apple-system, BlinkMacSystemFont, SF Pro Text, Segoe UI, Roboto, system-ui, sans-serif"
    fontSize: "clamp(18px, 1.6vw, 21px)"
    fontWeight: 400
    lineHeight: 1.55
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, SF Pro Text, Segoe UI, Roboto, system-ui, sans-serif"
    fontSize: "17px"
    fontWeight: 400
    lineHeight: 1.6
  mono:
    fontFamily: "ui-monospace, SF Mono, Menlo, Consolas, monospace"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.6
rounded:
  chip: "4px"
  icon: "6px"
  button: "7px"
  frame: "8px"
spacing:
  gutter: "28px"
  gutter-mobile: "18px"
  row-y: "12px"
  row-x: "24px"
  column-gap: "56px"
  column-gap-stacked: "32px"
  section: "72px"
  container: "1200px"
components:
  button-go:
    backgroundColor: "{colors.amber}"
    textColor: "{colors.amber-ink}"
    typography: "{typography.button}"
    rounded: "{rounded.button}"
    padding: "14px 22px"
  button-go-hover:
    backgroundColor: "{colors.amber-hover}"
    textColor: "{colors.amber-ink}"
  button-steel:
    backgroundColor: "{colors.steel-face}"
    textColor: "{colors.ink}"
    typography: "{typography.button}"
    rounded: "{rounded.button}"
    padding: "14px 22px"
  flap-cell:
    backgroundColor: "{colors.cell}"
    textColor: "{colors.ink}"
    typography: "{typography.display}"
  board:
    backgroundColor: "{colors.board}"
    rounded: "{rounded.frame}"
  departure-row-hover:
    backgroundColor: "#18191c"
    textColor: "{colors.amber}"
  code-well:
    backgroundColor: "{colors.well}"
    textColor: "#d9dade"
    typography: "{typography.mono}"
    rounded: "{rounded.frame}"
    padding: "16px 18px"
  nav-link:
    textColor: "{colors.muted}"
    typography: "{typography.nav}"
---

# Design System: rgrip

This system covers the landing site in `site/` only; the native egui app has its own look and does not follow it.

## Overview

**Creative North Star: "The Solari Board"**

A departure board is a results grid, so the page is one. The site reads as a concourse split-flap board for rgrip: steel-framed boards of graphite flap cells carrying white condensed capitals, a single amber lamp marking what to act on, and everything else set in quiet greys on a near-black ground. Real product screenshots sit in the same steel frame as "platforms" beside the board rows that describe them.

The voice is plain and the density is moderate: big flap capitals do the shouting once, at the top, and the rest of the page is short system-sans prose and ruled tables. The board is mechanical, not glowing; depth comes from the frame, the cell seam and soft drop shadows, never from gradients of light or neon. It rejects the centered-glow developer-tool hero with a grid of feature cards.

Motion has one grammar: a flap cascade. Each board flips its characters through a few random glyphs and settles, once as it enters view, and again on a departure row you point at.

**Key Characteristics:**
- Graphite flap cells with a dark centre seam, white condensed capitals, fixed cell widths.
- Brushed-steel frame (7px on boards, 5px on screenshot bezels) around every board and platform.
- Amber is the only accent: the primary action, the active or latest row, focus, selection.
- Big Shoulders for everything on the board; system sans for prose; mono for SQL and commands.
- Tables, not cards: features and releases are ruled departure rows.

## Colors

A near-black graphite board under one amber lamp; every other value is a grey.

### Primary
- **Platform Amber** (`amber`): the lamp. The Download button fill, the latest release row, hovered departure cells, link underlines (at 70% mix), focus rings, text selection and the caret. Hover lifts to **Lit Amber** (`amber-hover`). Text on amber is **Lamp Ink** (`amber-ink`), a warm near-black.

### Neutral
- **Concourse Ground** (`ground`): page background and the translucent sticky top bar (88% mix, 12px blur).
- **Board Black** (`board`): the inside of every steel frame.
- **Flap Graphite** (`cell`) over **Lower Flap** (`cell-lo`), split by **Seam** (`seam`): the two halves of a flap cell and the hinge line between them.
- **Brushed Steel** (`frame`): frame strips around boards and screenshot bezels.
- **Steel Edge** (`steel-edge`): the frame's inner highlight, the steel button border, the scrollbar thumb. **Steel Face** (`steel-face`) fills the secondary button.
- **Well** (`well`): code blocks and the screenshot bezel backing; the darkest surface.
- **Rule** (`line`): every 1px divider, table rule and outline border.
- **Board White** (`ink`): flap characters, headings, primary text.
- **Dim Grey** (`muted`): body prose, nav links, table descriptions, board titles.
- **Faint Grey** (`faint`): column headers, fact labels, small print, retired (past) releases.

### Named Rules
**The One Lamp Rule.** Amber marks exactly what is live or actionable: the primary button, the current row, the hovered row, focus. Never use it for decoration, headings or large fills beyond the primary button.

**The No Second Accent Rule.** There is no secondary hue. Retired rows step down to Faint Grey; the dim red the direction proposed for retired rows was not built and is not part of the palette.

## Typography

**Display Font:** Big Shoulders (self-hosted variable WOFF, weights 100-900, `font-display: swap`, preloaded; fallback Arial Narrow)
**Body Font:** System sans (-apple-system, SF Pro Text, Segoe UI, Roboto)
**Label/Mono Font:** ui-monospace / SF Mono / Menlo for SQL, commands and `kbd`

**Character:** A narrow, signage-cut grotesque set in uppercase with open tracking reads as station lettering; the system sans underneath stays out of the way so the prose sounds plain.

### Hierarchy
- **Display** (600, 0.8 of the cell height, line-height 1): flap characters only. Hero cells are up to 74px tall. Before the script runs the hero heading renders as plain 700 type at clamp(34px, 6vw, 72px), uppercase, 0.04em.
- **Headline** (700, clamp(32px, 3.4vw, 44px), 1): section headings beside platforms and on Download; uppercase, balanced wrap.
- **Title** (600, 26px, 0.08em): departure row names; **Title small** (600, 18px, 0.06em) for the row heads in feature tables; Download sub-heads use 700 22px.
- **Figure** (600, 21px, tabular figures): facts strip values and numeric table cells; any number on the board uses tabular figures.
- **Button / Nav** (700 19px 0.06em / 600 17px 0.07em): uppercase.
- **Label** (700, 13px, 0.16em, uppercase): column headers and fact labels in Faint Grey; board titles use the same cut at 15px in Dim Grey.
- **Lead** (400, clamp(18px, 1.6vw, 21px), 1.55, max 40em) and **Body** (400, 17px, 1.6, max 34em in copy blocks): system sans, Dim Grey.
- **Mono** (14px, 1.6): code wells; inline code at 0.88em.

### Named Rules
**The Board Speaks Caps Rule.** Everything that sits on the board (cells, row names, labels, buttons, nav) is Big Shoulders in uppercase with positive tracking. Sentences are never set in Big Shoulders; prose is always system sans in sentence case.

**The Fixed Cell Rule.** Flap text is set one character per cell; cells never stretch to fit text. Width is 0.66 of the cell height, the gap 0.07, so every row lines up on one character grid.

## Layout

A single 1200px column with 28px gutters (18px under 700px). Sections breathe at 72px vertical padding (feature pairs tighten to 24px between them; the hero opens at 44px top, 72px bottom).

- **Hero:** a full-width board holds three flap rows, padded with blank cells to the longest row like a real board. Cell size is `min(74px, (min(100vw,1200px) - 140px) / 15.3)` so the longest row fills the frame. Below it a two-column intro (lead left, buttons right, bottom-aligned) and a four-column facts strip ruled top and bottom with vertical rules between cells.
- **Platforms:** screenshot bezel and copy side by side at 7fr / 5fr with a 56px gap; alternate sections reverse to 5fr / 7fr.
- **Departures board:** a full-width framed table; row padding 12px 24px; the row name column shrinks to its content.
- **Download:** two equal columns: instructions and code well left, Releases board right.

**Responsive.** At 960px every two-column layout stacks to one column with a 32px gap, reversed platforms restore image-first order, and the facts strip becomes 2 x 2. At 700px the nav keeps only the GitHub link, hero cells resize to `(100vw - 92px) / 10.2`, trailing blank cells and word-space cells are dropped and rows wrap by word; departure rows become stacked blocks (name over description, headers hidden, 24px cells); release rows become a three-column grid.

## Elevation & Depth

Depth is physical and low: the board is a mounted object, not a floating card. Steel frames carry an inset black hairline and a 1px top highlight; flap cells carry a faint top bevel and a tight shadow; boards and bezels drop one long, soft shadow below. There are no glows and no hover lifts.

### Shadow Vocabulary
- **Board mount** (`box-shadow: inset 0 0 0 1px #000, 0 1px 0 #3a3d43 inset, 0 34px 70px -34px #000`): every steel-framed board.
- **Bezel drop** (`box-shadow: 0 30px 60px -30px #000`): screenshot platforms.
- **Flap bevel** (`box-shadow: inset 0 1px 0 rgba(255,255,255,.06), 0 1px 1px rgba(0,0,0,.7)`): every flap cell.
- **Button weight** (`box-shadow: 0 8px 20px -10px rgba(0,0,0,.9)`): the amber primary button only.

### Named Rules
**The Mounted, Not Floating Rule.** Shadows fall straight down and are soft; nothing moves up on hover. State is shown by colour (amber, lighter steel border), never by elevation.

## Shapes

Small, machined corners throughout. Boards, bezels, code wells and the support box share an 8px radius; buttons 7px; the app icon 6px; inline code, kbd and focus rings 4px. Flap cells round at 0.07 of their height. Frames are thick solid strips (7px boards, 5px bezels), never hairline outlines. The only recurring silhouette is the flap cell: a short upright rectangle split horizontally by a 1.2% seam at mid-height.

## Components

### Buttons
Signage plates: uppercase Big Shoulders, firm and flat.
- **Shape:** gently machined corners (7px), 14px 22px padding, 9px gap to an 18px line icon.
- **Primary (Go):** Platform Amber fill, Lamp Ink text, the one weighted shadow. Used for Download only.
- **Secondary (Steel):** Steel Face fill, Board White text, Steel Edge border; hover brightens the border to #6b7077.
- **Hover / Focus:** background, border and shadow transition over 0.2s on `cubic-bezier(.2,.7,.2,1)`; focus is a 2px amber outline, 3px offset.

### Flap cell (signature)
- **Geometry:** height `--c`, width 0.66c, radius 0.07c, glyph 0.8c at weight 600, `opsz` 72; cells and words gap 0.07c, hero rows gap 0.12c.
- **Surface:** Flap Graphite top half, Seam hinge, Lower Flap bottom half.
- **Sizes:** hero up to 74px; board rows 30px (24px on mobile); a blank cell keeps its slot with transparent text.
- **Accessibility:** the element keeps the words as its `aria-label`; cells are `aria-hidden`.

### Flap cascade (motion grammar)
- **Fold:** on each tick the top half folds down: an overlay rotates `rotateX(0 -> -90deg)` about its bottom edge over 110ms on `cubic-bezier(.5,0,.9,.4)` (an accelerating drop).
- **Ticks:** every 70ms a cell shows a random glyph from A-Z, 0-9 and `. · -`, for 3 to 7 flips (deterministic per cell position), then lands on its real character.
- **Stagger:** 32ms between cells along a row, 140ms between rows; the hover replay uses 18ms.
- **Triggers:** once per board when 60% of it enters view; again on pointer-enter of a departure row, at most every 900ms.
- **Reduced motion:** no cascade and no fold; the final text shows at once. Product clips still play, but gain controls.

### Departure tables
- **Departures board:** framed table; 13px Label headers in Faint Grey over a Rule line; rows divided by #202125; row name in 30px flap cells, description in 16px Dim Grey prose.
- **Row hover:** background shifts to #18191c over 0.25s, the row's cells turn amber, and its name cascades once.
- **Releases:** the latest row is lit (amber cells and link); past rows step down to Faint Grey and lift to Dim Grey on hover. Status links are 15px uppercase Big Shoulders.
- **Feature spec tables:** unframed; 1px Rule top and bottom per row, 10px vertical padding; uppercase 18px row heads at 40% width, prose descriptions, numbers in tabular Figure.

### Cards / Containers
- **Board:** Board Black inside a 7px Brushed Steel frame, 8px radius, Board mount shadow; optional title bar (15px Label in Dim Grey, 18px 24px 14px padding, Rule below).
- **Bezel:** 5px steel frame, 8px radius, Well backing, Bezel drop shadow, 1600/950 aspect ratio (or the image's own ratio), media cover-fit.
- **Support box:** 1px Rule outline, 8px radius, 22px 26px padding; no fill, no shadow.
- **Code well:** Well fill, 1px Rule border, 8px radius, 16px 18px padding, horizontal scroll, keyboard focusable.

### Navigation
- **Top bar:** sticky, 60px tall, translucent Concourse Ground with a 12px backdrop blur and a Rule bottom border. Wordmark: 26px icon plus "rgrip" in 700 24px Big Shoulders.
- **Links:** 17px uppercase Big Shoulders, Dim Grey, hover to Board White; GitHub carries an 18px line icon. Under 700px only GitHub remains.
- **Icons:** Lucide-style line icons, 18px, stroke 2, round caps and joins, `currentColor`.

## Do's and Don'ts

### Do:
- **Do** put anything that is a feature, a release or a fact on a board row or a ruled table, set in uppercase Big Shoulders.
- **Do** keep amber for the one primary action and the current or hovered row; text on amber is always Lamp Ink.
- **Do** frame screenshots and boards in steel (5px or 7px Brushed Steel, 8px radius).
- **Do** size flap text in whole cells (width 0.66c, gap 0.07c) and pad multi-row boards with blank cells to the longest row.
- **Do** run the cascade once per board on entry and honour reduced motion with the final text.
- **Do** use tabular figures for every number.

### Don't:
- **Don't** add a second accent colour or a red for retired rows; past items go Faint Grey.
- **Don't** use feature card grids, centered glows or gradient light: the board is mechanical and lit by one lamp.
- **Don't** set prose in Big Shoulders or uppercase; sentences stay in system sans.
- **Don't** lift elements on hover or add glow shadows; state changes are colour only.
- **Don't** let flap text reflow inside a cell or stretch cells to fit; on narrow screens rows wrap by word.
- **Don't** animate cells continuously or on scroll beyond the one entry cascade and the row hover replay.

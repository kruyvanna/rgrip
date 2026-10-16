---
name: rgrip
description: A small native Postgres client with a map, presented as a clean, standard developer-tool page.
colors:
  accent: "#7aa2f7"
  on-accent: "#0b1020"
  bg: "#0c0d10"
  raise: "#14161a"
  raise-2: "#1b1d22"
  line: "rgba(255,255,255,.08)"
  line-2: "rgba(255,255,255,.14)"
  text: "#f4f5f7"
  muted: "#9da2ad"
  faint: "#7b808b"
  white: "#ffffff"
typography:
  display:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, Segoe UI, system-ui, sans-serif"
    fontSize: "clamp(40px, 6.4vw, 76px)"
    fontWeight: 620
    lineHeight: 1.02
    letterSpacing: "-0.035em"
  headline:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, Segoe UI, system-ui, sans-serif"
    fontSize: "clamp(30px, 3.6vw, 44px)"
    fontWeight: 600
    lineHeight: 1.08
    letterSpacing: "-0.03em"
  lead:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, Segoe UI, system-ui, sans-serif"
    fontSize: "clamp(17px, 1.7vw, 19.5px)"
    fontWeight: 400
    lineHeight: 1.55
  title:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, Segoe UI, system-ui, sans-serif"
    fontSize: "15.5px"
    fontWeight: 580
    lineHeight: 1.4
    letterSpacing: "-0.005em"
  body:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, Segoe UI, system-ui, sans-serif"
    fontSize: "16.5px"
    fontWeight: 400
    lineHeight: 1.65
  body-sm:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, Segoe UI, system-ui, sans-serif"
    fontSize: "14.5px"
    fontWeight: 400
    lineHeight: 1.5
  button:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, Segoe UI, system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 550
    lineHeight: 1
  label:
    fontFamily: "Geist, -apple-system, BlinkMacSystemFont, Segoe UI, system-ui, sans-serif"
    fontSize: "13.5px"
    fontWeight: 400
    lineHeight: 1.5
    fontFeature: "\"tnum\" 1"
  code:
    fontFamily: "Geist Mono, ui-monospace, SF Mono, Menlo, monospace"
    fontSize: "0.86em"
    fontWeight: 500
rounded:
  code: "5px"
  focus: "6px"
  btn-sm: "8px"
  icon-tile: "9px"
  btn: "10px"
  shot: "14px"
  pill: "99px"
spacing:
  gutter-sm: "20px"
  gutter: "28px"
  stack: "48px"
  split-gap: "64px"
  section-sm: "88px"
  section: "120px"
components:
  button-primary:
    backgroundColor: "{colors.text}"
    textColor: "{colors.bg}"
    typography: "{typography.button}"
    rounded: "{rounded.btn}"
    padding: "0 20px"
    height: "44px"
  button-primary-hover:
    backgroundColor: "{colors.white}"
  button-ghost:
    backgroundColor: "{colors.raise}"
    textColor: "{colors.text}"
    typography: "{typography.button}"
    rounded: "{rounded.btn}"
    padding: "0 20px"
    height: "44px"
  button-ghost-hover:
    backgroundColor: "{colors.raise-2}"
  button-sm:
    rounded: "{rounded.btn-sm}"
    padding: "0 13px"
    height: "32px"
  release-pill:
    backgroundColor: "{colors.raise}"
    textColor: "{colors.muted}"
    typography: "{typography.label}"
    rounded: "{rounded.pill}"
    padding: "5px 10px 5px 5px"
  feature-icon-tile:
    backgroundColor: "{colors.raise}"
    textColor: "{colors.accent}"
    rounded: "{rounded.icon-tile}"
    size: "34px"
  screenshot-frame:
    rounded: "{rounded.shot}"
  inline-code:
    backgroundColor: "{colors.raise-2}"
    typography: "{typography.code}"
    rounded: "{rounded.code}"
    padding: ".08em .38em"
---

# Design System: rgrip

This covers the landing website in `site/` only; the native egui app has its own look.

## Overview

**Creative North Star: "The Quiet Standard"**

The page is the category standard for developer tools, played straight and finished carefully, at the level of Zed and Linear. A centered headline, the real app as the hero, two feature sections, a short list, a download. Nothing is themed: the earlier split-flap board world was dropped because it was hard to read, and familiarity is now the point. The product's screenshots and the one linked map-and-grid clip carry the weight; the page around them stays near-black, quiet and legible.

Density is calm: a 1080px column with generous section spacing (120px), one accent color used sparingly, and depth from tone and hairlines rather than shadow. Motion is limited to two gestures: the hero clip tilts back and lies flat as you scroll, and text rises softly once as it enters. With reduced motion, everything is simply there, flat and still.

**Key Characteristics:**
- Near-black ground with slightly raised surfaces; white-alpha hairlines do the structural work.
- Geist for everything, Geist Mono only for code and keys; tight negative tracking on headings.
- White primary button; the app's blue is the only accent.
- Real app captures in thin bordered frames, no big shadows.
- Lucide-style line icons, no emoji.

## Colors

A cool, nearly neutral near-black palette (hue around 265, chroma under 0.02) with a single desaturated blue taken from the app itself.

### Primary
- **App Blue** (`accent`): the only accent. Used on the release pill's "New" tag (as a tint), feature icon strokes, focus outlines, text selection (45% mix), caret, the skip link and the hero's faint radial glow (13% alpha). Never a button fill and never body text.
- **Blue Ink** (`on-accent`): text on a solid App Blue fill (the skip link). 7.5:1 against App Blue.

### Neutral
- **Night Ground** (`bg`): page background and `theme-color`.
- **Raised Slate** (`raise`): ghost button, release pill, feature icon tiles.
- **Raised Slate Hi** (`raise-2`): ghost button hover, inline code and kbd chips.
- **Hairline** (`line`): section dividers, list rules, top bar and footer borders, icon tile and code borders.
- **Hairline Strong** (`line-2`): edges that must read as objects: screenshot frames, ghost button, release pill.
- **Paper White** (`text`): headings and body text, and the primary button fill. 17.8:1 on Night Ground.
- **Quiet Grey** (`muted`): lead, section copy, list descriptions, nav links. 7.6:1 on Night Ground.
- **Faint Grey** (`faint`): the facts line under the hero CTA and the footer copy only. 4.9:1 on Night Ground; do not put it on anything lighter than Raised Slate.

### Named Rules
**The One Blue Rule.** App Blue is the only hue on the page. It marks small things (a tag, an icon stroke, focus) and never fills a button or a large area; the hero glow is its only ambient use.

**The White Button Rule.** The primary action is Paper White on Night Ground, brightening to pure white on hover. Emphasis comes from value, not hue.

## Typography

**Display Font:** Geist (with -apple-system, system-ui fallbacks), self-hosted variable woff2, weights 100-900
**Body Font:** Geist
**Label/Mono Font:** Geist Mono (with ui-monospace, SF Mono, Menlo), self-hosted

**Character:** One neutral grotesque used at in-between variable weights (550, 580, 620) so headings feel firm without going bold, paired with its own mono for code. Plain, current, and out of the way.

### Hierarchy
- **Display** (620, clamp 40-76px, 1.02, -0.035em, balanced wrap): the hero headline only, two lines.
- **Headline** (600, clamp 30-44px, 1.08, -0.03em, balanced wrap): section headings.
- **Lead** (400, clamp 17-19.5px, 1.55, Quiet Grey, max 600px): the hero paragraph. Section copy is 17px in Quiet Grey at max 30em.
- **Title** (580, 15-15.5px): feature point names and list terms.
- **Body** (400, 16.5px, 1.65): base text. **Body Small** (14.5-15px, 1.5-1.55, Quiet Grey) for feature descriptions and list definitions.
- **Label** (400, 13.5px, tabular numbers): release pill, facts line, footer, small button.
- **Code** (Geist Mono 500, 0.86em): inline code and kbd chips.

### Named Rules
**The Negative Tracking Rule.** Tracking tightens as size grows: -0.035em on display, -0.03em on headlines, -0.01em on the wordmark, about zero for body. Never letter-space small text or set it in uppercase.

## Layout

A single centered column: 1080px max with 28px gutters (20px under 640px); the hero clip widens to 1240px, and the map and import screenshots sit at 880px. Download copy narrows to 620px. Sections are separated by 120px (88px under 900px), with 120px below the last section before the footer.

Hero and download are centered. The two feature sections are a two-column split (copy 1fr, points 1.15fr, 64px gap) with a 2x2 grid of icon points, followed by a full-width screenshot 56px below. The everyday list is a two-column definition list (170px term column, 48px column gap) ruled with hairlines. Under 900px the split and list collapse to one column; under 640px the points stack, list terms sit above definitions, and nav keeps only GitHub and Download.

The top bar is sticky, 60px tall, translucent Night Ground (80%) with backdrop blur and a hairline bottom; anchors offset by 72px.

## Elevation & Depth

Flat. There are no box-shadows anywhere. Depth comes from three steps of tone (Night Ground, Raised Slate, Raised Slate Hi) and white-alpha hairlines; the only blur is the sticky top bar's backdrop. One soft radial glow of App Blue (13% alpha, 1100x640px) sits behind the hero headline. The hero clip gains depth from perspective only: it starts tilted back (rotateX 14deg, scale 0.96) and lies flat over the first 420px of scroll.

### Named Rules
**The Hairline, Not Shadow, Rule.** Separate and frame with a 1px white-alpha border (8% for dividers, 14% for objects). If something seems to need a shadow, it needs a stronger hairline or a tone step instead.

## Shapes

Softly rounded, increasing with object size: 5px code chips, 6px focus rings, 8px small buttons, 9px icon tiles, 10px buttons, 14px screenshot frames and the download icon (10px on phones for the hero), full pill for the release pill and its tag. Screenshots clip to their frame with a near-black (#0a0a0c) backing.

## Components

### Buttons
Firm and plain; value carries the hierarchy.
- **Shape:** gently rounded (10px), 44px tall, 20px side padding, Geist 550 at 15px, optional 18px line icon with an 8px gap.
- **Primary:** Paper White fill, Night Ground text; hover goes to pure white.
- **Ghost:** Raised Slate fill, Paper White text, Hairline Strong border; hover goes to Raised Slate Hi.
- **Small:** 32px tall, 13px padding, 8px radius, 13.5px text; used for Download in the top bar.
- **States:** background and border fade over 0.2s; pressing nudges down 1px. Focus is a 2px App Blue outline offset 3px.

### Release pill
A link to the current release above the headline: Raised Slate, Hairline Strong border, full pill radius, Quiet Grey 13.5px text, with a small "New" tag (App Blue text on an 18% App Blue tint) and a 14px chevron. Hover brightens the text to Paper White and the border to 24% white. This is announcement content, not a decorative eyebrow; one per page, only when there is a release to point to.

### Feature points
A 2x2 grid of icon + title + one line. The icon is a 34px Raised Slate tile with a Hairline border, 9px radius and an App Blue line icon (stroke 1.75). Title in Paper White 580, description in Quiet Grey 14.5px.

### Everyday list
A definition list in two columns, each row ruled by a Hairline top border (last row also bottom), 18px vertical padding, term in 580 weight and definition in Quiet Grey.

### Screenshot frame
Real app captures only. 14px radius, Hairline Strong border, overflow clipped, no shadow. The hero frame holds the autoplaying, looping, muted linked-view clip at 1600/950 (4/3 and zoomed onto the grid-map seam on phones); clips get controls under reduced motion.

### Navigation
Sticky translucent bar: wordmark (24px icon, 600 at 17px, -0.01em) on the left; Quiet Grey 14.5px links that brighten to Paper White on hover, a GitHub line icon, and the small primary Download button on the right.

### Inline code and keys
Geist Mono 500 at 0.86em on Raised Slate Hi with a Hairline border and 5px radius.

### Footer
Hairline top, Faint Grey 13.5px license line on the left, Quiet Grey links on the right (GitHub, Releases, Changelog, Sponsor with a heart line icon).

## Do's and Don'ts

### Do:
- **Do** keep App Blue to small marks: tags, icon strokes, focus, selection.
- **Do** make the primary action the white button and everything secondary a ghost button.
- **Do** frame every app capture with a 1px Hairline Strong border at 14px radius and nothing else.
- **Do** use the single soft rise (14px up, 6px blur, 0.9s, ease cubic-bezier(.16,1,.3,1), 60ms sibling stagger) for entering text, and nothing more; leave content visible without JS and under reduced motion.
- **Do** use Lucide-style line icons (stroke 1.75, round caps) at 14-18px.
- **Do** tighten heading tracking as size grows and balance heading wraps.

### Don't:
- **Don't** add box-shadows, glows on cards, or gradient fills; the hero's radial glow is the only ambient light.
- **Don't** introduce a second accent hue or fill a button with App Blue.
- **Don't** bring back themed devices (split-flap cells, display faces, board frames); the page must read as a familiar developer-tool site.
- **Don't** use emoji, mockup devices or invented screenshots.
- **Don't** set Faint Grey on surfaces lighter than Raised Slate, or use it for anything but fine print.

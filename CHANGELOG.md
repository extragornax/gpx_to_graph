# Changelog

## 2026-04-27

### Added
- **Trip Planner** (`/trip`) — multi-day GPX route splitter:
  - User accounts (register/login) to persist uploaded GPX files
  - Upload a GPX route and split it into multi-day stages
  - Interactive SVG elevation profile with draggable day boundary handles
  - Leaflet map with color-coded day segments and boundary markers
  - Per-day stats table (distance, elevation gain/loss, start/end elevation)
  - Download individual day GPX files
  - Map follows the cursor during drag without resetting zoom
- **GPX Toolkit** (`/toolkit`) — client-side GPX multi-tool with five panels:
  - *Viewer*: Leaflet map + SVG elevation chart + stats grid
  - *Profile*: generate elevation graph via `/generate` with full options
  - *Merge*: combine 2–5 GPX files into one via `/merge` API
  - *Reverse*: flip track direction, preserving all metadata and extensions
  - *Simplify*: Douglas-Peucker track simplification with live tolerance slider
- Shared site-wide navigation bar across all pages (replaced iframe tabs)
- Leaflet map on the ravito page showing route and POI markers
- Unified cream/editorial design system (Fraunces, Bricolage Grotesque, Space Mono)

### Fixed
- API base URLs for services mounted under path prefixes
- Broken CSS and 404s when services are nested under path prefixes

## 2026-04-26

### Added
- Integrated meteo, ravito, trace, strava_stats, and col services into unified server

## 2026-04-24

### Added
- Shareable links: `/generate` output persisted and served via share URLs (30-day TTL)
- Link-preview meta tags and gpx.studio "Open" button on share pages
- Recent-routes sidebar backed by localStorage
- GPX route name surfaced in share pages and recents list

### Fixed
- Share page classes renamed to avoid ad-blocker cosmetic filters
- Shares persist across server restarts
- gpx.studio mixed-content issue resolved

## 2026-04-20

### Added
- GPX merge functionality (merged from gpx-merge project)
- Extensions data handling in GPX parsing
- Upload stats with smoothing on merge

## 2026-04-16

### Added
- Axum web server for hosting the tool online
- Docker deployment

## 2026-04-13

### Added
- Initial GPX-to-graph elevation profile generator

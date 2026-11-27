# Specification Quality Checklist: M2 Regional Tile Map & Layer Stack

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-11-25
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (internal Rust symbols/types omitted from FRs/SCs; locked product/protocol standards documented in Context)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders alongside clear verification contracts
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (observable timings, byte/request metrics, numerical parity, and visual seam checks)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified (0/1 stations, antimeridian `±180°`, polar `>85.0511°`, `data_spatial` → `data_run` fallback, corrupt `.pmtiles`, rapid pan/zoom debouncing, stale/hidden readout)
- [x] Scope is clearly bounded (MVT geometry-only styling bounded; color-to-value inversion deferred to post-M2 while value-provenance seam is established)
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows and fault/offline paths
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] Mandatory FRs/SCs remain valid under both `nsh =0.5.1` and the evaluated `nsh =0.7.0` shell pre-gate

## Notes

- **User decisions recorded (2026-11-25)**:
  1. **Architecture**: Direction C — standard Web Mercator (`EPSG:3857`) `256×256` `z/x/y` tile pyramid backed by a worker-cached single-timestep numeric field + pure-Rust vector overlays, chosen specifically to support hosting additional tile layers and post-M2 `radiust`-style color-to-value inversion (`palette_decoded`).
  2. **Spatial browsing scope**: Auto-fit confirmed station bounds (+ margin) on open, while permitting free pan/zoom across wider regions.
  3. **Basemap & Layer Stack**: Ordered multi-layer stack (`visibility`, `opacity`, `z-order`) shipping with built-in offline Natural Earth coastlines/admin-0 boundaries (Public Domain) plus user-mounted local `.pmtiles` v3 archives (raster `png`/`jpeg`/`webp` and deterministic geometry-only MVT).
  4. **Forecast data source & retention fallback**: Prefer `data_spatial` single-timestep `.om` objects (`variable := ...`); automatically fall back once to `data_run` with a visible slow-path notice when a pinned historical run is older than `data_spatial`'s ~7-day retention.
  5. **NSH shell pin pre-gate**: Evaluate upgrading `nsh-*` from `=0.5.1` to `=0.7.0` during `/speckit.plan` (`S0-NSH-070`), amending Constitution Principle II only after recorded evidence passes.
- Ready for `/speckit.plan`. Main planning spikes: (1) `S0-NSH-070` shell pin evaluation, and (2) `S1-M2` parallel latitude-band `data_spatial` fetch + O1280 cross-tile seam-free interpolation verification.

# Specification Quality Checklist: M1 Stations × Forecast Series

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-11-23
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous (SC-002 and station-set size are marked provisional)
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into specification

## Notes

- **Iteration 2 (2026-11-23)**: A check of the public bucket metadata found that `ecmwf_ifs`
  (9 km) is a reduced Gaussian O1280 grid. duckomo documents Plan B nearest only for regular
  spherical grids. The check also found that 06Z and 18Z runs reach only +144 h. Both facts are
  recorded under "Source facts" and turned into the 2 clarification markers. Iteration 2 also:
  - added failure-path scenarios (cancel, worker kill, stale inputs, offline) and a Chinese
    IME scenario;
  - defined the offline behavior, the matching distance, and the inclusive 0–168 h window
    with expected row counts;
  - marked SC-002 and the station-set size as provisional.
- Named technologies (NSH, duckomo, Open-Meteo OM, the `station_series` schema, Plan B) are
  locked product constraints from the constitution, roadmap, and verified feature 001
  decisions, as in spec 001. They are not implementation choices made by this spec.
- **Iteration 3 (2026-11-23)**: The user resolved both markers:
  - default product: `ecmwf_ifs` 9 km O1280, so M1 must add and validate O1280 nearest-point
    sampling;
  - default run: the latest completely published run of any cycle, truncated and labelled when
    it is a 6-day run.

  Ready for `/speckit.plan`. The main planning risk is the O1280 nearest sampling plan and its
  independent reference.

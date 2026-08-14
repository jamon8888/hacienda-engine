# P4/P5 — Wiring the missing review and compliance endpoints

**Date:** 2026-08-14
**Status:** Implemented (2026-08-14)
**Extends:** `2026-08-01-P4-human-review-api.md` §3 and `2026-08-01-P5-compliance-artefacts-api.md`
§3 — closes the routes each spec's own table names but `hacienda-api/src/routes.rs` never
built.

---

## 1. Problem

Verified directly against `hacienda-api/src/routes.rs` and `hacienda-core`: in both cases the
business logic was already written and tested, and only the HTTP endpoint was missing —
exactly the shape wave 0 of `2026-08-01-hacienda-platform-parity-program.md` §8 predicts
("Exposent du métier déjà écrit et testé. Quelques centaines de lignes de handlers").

**P4.** `ReviewQueue` (`hacienda-core/src/review/queue.rs`) already has `get`, `assign`, and
`stats` methods, each already used by that module's own unit tests. Only `list` (`GET
/v1/review`) and `decide` (`POST /v1/review/{id}/decide`) are routed. `HaciendaFacade::
review_queue_with_auth`/`review_queue_read_with_auth` already return `Option<&ReviewQueue>` —
a handler can call `.get`/`.assign`/`.stats` on the returned reference directly, the same way
`decide_review` already calls `.decide`. No new facade method is needed for P4.

**P5.** `ComplianceGenerator` (`hacienda-core/src/compliance/mod.rs`) already has `model_card`,
`checklist`, and `dora_report` methods. Only `dpia` and the bundled `report` (which includes
`model_card`/`checklist` when enabled in config, but **never** `dora` — see below) are routed.

## 2. `GET /v1/compliance/dora` cannot be a bare `GET` — built as `POST` instead

`ComplianceGenerator::report`'s own doc comment: "A DORA report is only produced when an
`incident` is supplied; DORA reports describe a specific event, so there is nothing
meaningful to emit without one." `compliance_report_with_auth` always calls `report(None)` —
confirmed by reading it — so a DORA report has had **no path to the API at all** until this
change, unlike `model_card`/`checklist`, which were already reachable (bundled inside
`GET /v1/compliance/report`, just not individually addressable).

`PiiIncident` (`summary`, `timeline`, `root_cause`, `detected_at`, optional
`contained_at`/`resolved_at`, `Vec<String>` `actions_taken`/`lessons_learned`) does not fit
into query parameters without an awkward flattening scheme for the two list fields. Built as
`POST /v1/compliance/dora` with `PiiIncident` as the JSON body instead of the bare `GET` an
earlier reading of the P5 spec's route table might suggest — a real, small, deliberate
deviation, recorded here rather than forced into a shape that doesn't fit the data.

`model_card`/`checklist` remain `GET` — genuinely no input, matching the spec's table exactly.

## 3. Capabilities

All three compliance routes: `audit:read`, matching `dpia`/`report`'s existing gating and the
P5 spec's table.

For the two review routes:

- `GET /v1/review/{id}`: `audit:read` — same tier as `GET /v1/review` (`review_queue_read_with_auth`),
  since reading one item is not a more sensitive operation than reading the list it comes
  from. The P4 spec's own table proposes `review:decide` for every review route uniformly;
  this diverges the same way the already-shipped `GET /v1/review` diverges from that table
  (see that route's own doc comment) — read and decide are different privileges here, and
  this endpoint is a read.
- `GET /v1/review/stats`: `audit:read`, same reasoning — an aggregate read, no snippet text.
- `POST /v1/review/{id}/assign`: `review:decide` — a write, same tier as `decide`.

## 4. Tests

| Test | Assertion |
| --- | --- |
| `get_review_item_returns_the_item_by_id` | Facade-level, mirrors `decide`'s existing test shape. |
| `get_review_item_returns_none_for_an_unknown_id` | Not a distinguishable error — matches every other store's "not found is `None`" discipline in this codebase. |
| `assign_review_item_sets_the_reviewer` | |
| `review_stats_reports_pending_count` | |
| `model_card_and_checklist_are_reachable_standalone` | Both resolve without a `report()` round trip. |
| `dora_report_requires_an_incident_body` | The route that could never be reached before this change now can. |
| `dora_route_is_post_not_get` | Pins the §2 deviation so a future "helpfully" restore-to-GET doesn't silently break the incident body. |

## 5. Exit criteria

- All six routes are live, documented in the OpenAPI document (S4's anti-drift tests catch a
  route added without one), and covered by the tests in §4.
- No new facade method was needed for the two P4 read routes or the assign route — confirmed
  by implementation, not just predicted.

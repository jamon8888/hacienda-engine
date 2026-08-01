# Annotation guideline — `fixtures/ner-eval/`

## Status: DRAFT — needs a second annotator's sign-off

This guideline was written and applied by the same engineering agent that drafted
`base-fr.json`, `base-en.json`, and `vertical-finance.json`. Per the implementation plan
(§1.1): "Have a second person (or a second independent pass) re-annotate a 20-case sample
and record inter-annotator agreement. If it is low, the guideline is the bug, not the
model." **That second pass has not happened.** The rulings below are a starting point for
that review, not a settled convention. Do not cite exact-match F1 computed against this
fixture as meaningful until inter-annotator agreement is recorded here.

> **Inter-annotator agreement: not yet recorded.** (Placeholder — fill in span-level
> Cohen's kappa or percent exact-match agreement here once a second annotator has
> re-labelled a 20-case sample from `base-fr.json` + `base-en.json`.)

---

## Rulings

### 1. Titles (`M.`, `Me`, `Dr`, `Mr.`, `Mrs.`) — **excluded** from the person span

A title is not part of the name; it is a role marker. The person span starts at the given
name (or the first name-bearing token).

- Example (fr-005): `"M. Bernard Béranger habite à Aix-en-Provence."` → person span is
  `"Bernard Béranger"` (excludes `"M. "`).
- Example (en-002): `"Please contact Dr. Emily Chen, ..."` → person span is `"Emily Chen"`
  (excludes `"Dr. "`).

Rationale: titles are register/context, not identity. Including them would make span
boundaries dependent on document formality rather than on the name itself, and would
double-count a single mention if a title and a name were ever detected as separate spans by
a downstream category.

### 2. Particles (`de`, `de la`, `d'`, `van der`) — **included** in the person span

A nobiliary or connective particle that is orthographically part of the surname is part of
the span. This is deliberately the opposite ruling from titles: a particle is part of the
name (removing it changes who is being referred to), a title is not (removing it does not).

- Example (fr-001): `"Élodie de La Fontaine"` — the full string, including `"de La"`, is one
  person span.
- Example (fr-002): `"François d'Aubigné"` — `"d'Aubigné"` is one token for annotation
  purposes; the apostrophe does not split the span.
- Example (fr-007): `"Camille de la Tour"` — `"de la"` included.
- Example (en-007): `"Mary van der Berg"` — `"van der"` included.

### 3. Legal-entity suffixes (`SARL`, `SA`, `Inc.`, `Corporation`, `& Sons`, `& Fils`,
`& Partners`, `& Associés`) — **included** in the organisation span

The suffix is part of the registered legal name, not a separate token. Splitting it off
would make the span shorter than the string a human would copy-paste as "the company's
name".

- Example (fr-003): `"Dupont SARL"` is one organisation span, not `"Dupont"` alone.
- Example (en-003): `"Global Dynamics Inc."` is one organisation span, including the
  trailing period that is part of the abbreviation.
- Example (en-009): `"Smith & Sons"` is one organisation span.

### 4. Nested spans (a person name inside an organisation name) —
**not annotated as nested; the outer (organisation) span wins, no separate person span**

When an organisation's legal name is derived from a founder's or partners' names (a common
pattern in law firms, family businesses, and eponymous companies), this fixture's base
categories do **not** produce two overlapping spans (one `organization` covering the whole
string, one `person` covering the embedded name). Only the `organization` span is
annotated. Rationale: GLiNER2, like most span-extraction NER, is not evaluated here on
overlapping-span recall (`score()`'s greedy one-to-one per-label matcher assumes
non-overlapping gold within a label but does not forbid overlap across labels — this rule
is a corpus-authoring decision, not a scorer limitation), and conflating "is this string a
person's name" with "is this string an organisation's name" inside the same span produces
label noise a downstream redaction pipeline cannot act on sensibly (do you redact it once,
as an org, or twice?).

- Example (fr-008): `"Le cabinet Jean Dupont & Fils a été fondé en 1950."` → **one**
  `organization` span, `"Jean Dupont & Fils"`. No separate `person` span for `"Jean
  Dupont"`, even though it is visibly a person's name.
- Example (en-009): `"Smith & Sons"` — same rule, `organization` only.
- This is the fixture's deliberate "organisation names that overlap person names" case
  called for in the implementation plan (§1.1) — the point of including it is to see
  whether the model separately fires `person` inside the span, `organization` for the
  whole span, both, or neither, not to force one "correct" nested answer into the gold set
  the model must match token-for-token.

### 5. Byte-vs-char offsets on accented text — **byte offsets, always**

All `start`/`end` values are **UTF-8 byte offsets**, matching xberg's
`Entity { start: u32, end: u32, .. }`. They are not Unicode scalar (`char`) offsets and not
grapheme-cluster offsets. Several cases exist specifically to make this distinction
observable:

- `fr-001`: `"Élodie"` — `É` is a 2-byte UTF-8 sequence (`0xC3 0x89`), so the byte offset of
  `"Élodie"`'s start is 1 byte higher than its char-index would suggest once anything before
  it also contains a multi-byte character; more directly, `"Élodie de La Fontaine"`'s *end*
  offset is 2 bytes longer than its character count because of the `É`.
- `fr-009`: `"Amélie Nguyễn-Dupuis"` — `é` (2 bytes) and `ễ` (a Vietnamese-orthography
  character formed from `e` + combining marks, `U+1EC5`, 3 bytes in UTF-8) both appear;
  this case exists to make sure a naive `chars().nth(n)`-based span extractor (which would
  compute the *wrong* offset here) is not accidentally what gets tested.
- `fr-010`: `"Saint-Étienne"` — a second `É` case, this time inside a hyphenated place name.

Any tool that annotates this fixture by counting characters instead of encoding to UTF-8
bytes first will produce offsets that are off by however many multi-byte characters precede
the span. The schema-validity test in `hacienda-core/tests/ner_eval.rs` catches this
class of error today by asserting `text.as_bytes().get(start..end)` is `Some` and
`str::from_utf8` succeeds on the slice — a byte-offset mistake that lands mid-codepoint
fails that assertion immediately, before any model ever runs.

---

## Open questions for the second annotator

- Should `fin-*` cases in `vertical-finance.json` include surrounding context words (e.g.
  `"code guichet 00456"` in `fin-003`) as part of any span, or are they correctly excluded
  as of this draft (only the account number digits themselves are spanned)? Current
  ruling: excluded — only the identifier value itself is a span, not its label word.
- Should multi-part person names that are **not** joined by a particle (e.g. two given
  names with no connector) ever be split into two spans? Current ruling, implicit in every
  case above: no — a full name is always one span, given names and surname together.
- None of the current cases test a title immediately followed by a particle-led surname
  with no given name shown (e.g. `"Me de la Tour a signé..."` with no first name at all).
  If that pattern turns out to matter for real documents, add a case and rule on it before
  trusting exact-match F1 on titles.

# Annotation Guideline — `fixtures/ner-eval/`

Exact-match F1 measures agreement with a stated convention, not "correctness" in the
abstract. An unstated convention makes the number meaningless — two annotators who
disagree about whether a title belongs in a person span will disagree about F1 even if
both correctly located every entity. This document is the convention. It was written
**before** any case in `base-fr.json`, `base-en.json`, or `vertical-finance.json` was
annotated, and every fixture in this directory follows it.

If a future case seems to need an exception, change this document and re-annotate the
affected cases in the same change — do not annotate an exception silently.

## 1. Titles and honorifics — excluded from person spans

Titles (`M.`, `Mme`, `Me`, `Dr`, `Madame`, `Monsieur`, `Mr.`, `Mrs.`, `Dr.`, …) are
**not** part of the person span. The span starts at the first token of the given name or
surname.

- `M. Étienne du Pont a demandé...` → span covers `Étienne du Pont`, not `M. Étienne du
  Pont`.
- `Me Sophie Vasseur plaide...` → span covers `Sophie Vasseur`.
- `Mrs. Eleanor Whitmore-Blake confirmed...` → span covers `Eleanor Whitmore-Blake`.

Rationale: a title is a role marker, not part of the identity string, and pseudonymised
output that preserves "Dr" or "M." as a literal is more useful than one that redacts it
along with the name.

## 2. Nobiliary/patronymic particles — included in person spans

French particles (`de`, `du`, `des`, `de la`, `d'`) that are part of a family name **are**
included in the person span, because French naming convention treats them as part of the
surname, not as a separate preposition.

- `Grégoire de la Tour` → full span `Grégoire de la Tour`, including `de la`.
- `Béatrice d'Estaing` → full span `Béatrice d'Estaing`, including `d'`.
- `Madame Isabelle de Montmorency` → title `Madame` excluded (rule 1), particle `de`
  included → span `Isabelle de Montmorency`.

## 3. Legal-entity suffixes — included in organisation spans

Suffixes that are part of a registered or commonly used trading name (`SARL`, `SA`,
`SAS`, `LLC`, `Inc.`, `LLP`, `& Fils`, `& Associés`, `Frères`, `& Sons`) **are** included
in the organisation span.

- `Château Margaux SA` → full span including `SA`.
- `Renault-Peugeot Consulting SARL` → full span including `SARL`.
- `Blackstone Capital Partners Inc.` → full span including `Inc.`.

Rationale: the suffix disambiguates the entity from an individual or informal group of
the same base name, and a redaction/pseudonymisation token that drops it produces a
string that no longer identifies which registered entity was referenced.

## 4. Leading definite articles — excluded from organisation spans

A leading grammatical article (`Le`, `La`, `L'`, `Les`, `The`) immediately before an
organisation's name is **excluded** from the organisation span when it functions as an
article of French/English grammar rather than as part of the trademark itself.

- `La Fondation Jean Moulin soutient...` → span `Fondation Jean Moulin`, excluding `La `.
- `Le Cabinet Lefèvre représente...` → span `Cabinet Lefèvre`, excluding `Le `.
- `The Johnson Group represents...` → span `Johnson Group`, excluding `The `.
- `La société Dupont & Associés a déposé...` → `La société` is a common-noun descriptor,
  not part of the name at all; span is `Dupont & Associés`.

If an article is graphically part of the registered trademark itself (e.g. a publication
titled *Le Monde*), it would be included instead — no case in this fixture set requires
that exception, so it is not exercised here. If one is ever added, say so explicitly next
to the case.

## 5. Nested spans — not used

When an organisation's name is built from what would, out of context, read as a person's
name, annotate **only** the maximal organisation span. Do not additionally emit a nested
person span for the embedded name.

- `Fondation Jean Moulin` → one `organization` span covering the whole name. No separate
  `person` span for `Jean Moulin`.
- `The Robert Kennedy Foundation` → one `organization` span (`Robert Kennedy Foundation`,
  per rule 4). No separate `person` span for `Robert Kennedy`.
- `Groupe Bernard Arnault` → one `organization` span. No separate `person` span for
  `Bernard Arnault`.

Rationale: nested/overlapping gold spans would require the metrics module to support
multi-span-per-offset gold sets and a containment rule, which is not needed for the
Tier 0 zero-shot label set this harness measures, and would make "exact match" ambiguous
(match against which of the two overlapping gold spans?). If a future vertical needs
nested entities, this rule and the metrics module's matching logic must both change
together.

## 6. Byte offsets, not character offsets

`start`/`end` are **UTF-8 byte offsets**, matching Rust `str` byte indices
(`text.as_bytes()[start..end]`), **not** Unicode codepoint or character indices. This
matters as soon as any accented character precedes an entity.

Worked example: in `"Merci de transmettre le dossier à Éléonore de La Fontaine avant
vendredi."`, the entity `"Éléonore de La Fontaine"` starts at byte offset **35**, not
codepoint offset 35 — `à` before it is a 2-byte UTF-8 sequence (`0xC3 0xA0`) but a single
character, so the codepoint count and byte count diverge by one at that point already.
The entity text itself is 24 **characters** but 26 **bytes**: `É` and `é` are each
2-byte UTF-8 sequences (`0xC3 0x89` and `0xC3 0xA9`), so `Éléonore` alone is 8 characters
but 10 bytes. Every offset in this fixture set was computed by encoding the text as UTF-8
first and indexing the byte string, then verified by slicing those same bytes and
decoding back to confirm the recovered string matches exactly (see
`/scratchpad/gen_fixtures.py`'s `byte_span` helper, used to generate every case in this
directory — not hand-computed).

## 7. Address spans

An address span covers the full civic address as printed — street number, street name,
postal code, and city — when they appear contiguously in one string.

- `"Le siège social est situé au 12 rue de la République, 69002 Lyon."` → span
  `12 rue de la République, 69002 Lyon`.

A bare city name mentioned without an accompanying street address is annotated as its
own (shorter) address span covering just the city.

- `"Maison Hermès International a ouvert une nouvelle boutique à Lyon."` → span `Lyon`
  only (no street address is given in the sentence).

## 8. Email and phone spans

Email and phone spans cover exactly the contact string as printed, including a leading
`+` and international prefix for phone numbers when present (e.g. `+33 6 12 34 56 78`),
and excluding trailing sentence punctuation (a period ending the sentence is never part
of the span).

## Ruling summary (quick reference)

| Question | Ruling |
|---|---|
| Title (`M.`, `Me`, `Dr`, …) in the person span? | No — excluded |
| Particle (`de`, `de la`, `d'`) in the person span? | Yes — included |
| Legal-entity suffix (`SARL`, `SA`, `LLC`, …) in the org span? | Yes — included |
| Leading article (`Le`, `La`, `The`, …) in the org span? | No — excluded (unless graphically part of the trademark; not exercised here) |
| Nested spans (person inside an org name)? | Not used — annotate the maximal span only |
| Offsets: byte or char? | **Byte** (UTF-8, Rust `str` indices) |

## Second-pass review — not yet done

`superpowers/plans/2026-07-31-vertical-model-specialisation-implementation.md` §1.1 also
calls for a second, independent annotator to re-annotate a 20-case sample and for
inter-annotator agreement to be recorded. **That has not happened.** These fixtures are
the output of a single pass by one annotator (see `README.md`'s "Known limitation"
section) and should not be treated as a validated measurement instrument until a second
qualified annotator has reviewed them.

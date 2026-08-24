# Studio Export as a Portable Knowledge Base

**Date:** 2026-08-24
**Status:** Proposed
**Builds on:** `2026-07-31-vertical-model-specialisation-design.md` (Proposed) — §2.1's tensor
measurements and §4.1's zero-shot-labels-are-free finding are load-bearing here
**Touches:** `apps/hacienda-studio/lib/zip-export.ts`, `lib/registry.ts`, `lib/annotate.ts`,
`worker/pipeline.ts`

---

## 1. Problem Statement

Studio's zip export already emits more than redacted markdown: one file per entity, a glossary,
a batch registry, and three knowledge-graph serialisations. The stated goal (`buildBundleReadme`,
`lib/zip-export.ts:90-140`) is that a Claude Desktop session opening the bundle can answer
cross-document questions without reading every document.

It cannot, for four reasons this spec establishes as measured facts rather than opinions:

1. The relationships in the export are **fabricated** — co-occurrence relabelled as employment.
2. In `pseudonymize` mode — the mode this branch exists to ship — the entity layer is **empty of
   every person and organisation**, by construction.
3. Entity identity is **not stable across runs**, so no link into the bundle survives a re-export.
4. The bundle's shape is tuned for a reader that can hold it all in context. Claude Desktop is not
   that reader.

Separately, the question that prompted this spec — *"we use GLiNER2, can we get semantic linking
out of it?"* — has a more interesting answer than expected, and a sharper limit. Section 2
establishes both.

This spec answers three questions in order:

1. What can GLiNER2 actually give us, on the path Studio can reach today? (§2, §5)
2. What does "folder as knowledge base" mean for a filesystem-MCP reader specifically? (§4)
3. What is worth building before any model work at all? (§5.1, §8)

---

## 2. Measured Ground Truth

Everything in §2.2–§2.6 was read from source on 2026-08-24. §2.1 is from upstream documentation.
§2.7 is inherited measurement, marked as such.

### 2.1 What GLiNER2 supports upstream

GLiNER2 is not a NER model with extras. It is a multi-task extraction model with four heads
composable in a **single forward pass** via a schema:

| Task | API | Output |
| --- | --- | --- |
| Entities | `extract_entities(text, labels\|{label: description})` | spans, confidence, offsets |
| Classification | `classify_text(text, {task: [labels]})` | single- or multi-label, `cls_threshold` |
| Structured extraction | `extract_json(text, {obj: ["field::type::description"]})` | typed fields, `str`/`list`, enum choices `[a\|b\|c]`, per-field thresholds, regex validators |
| Relations | `extract_relations(text, ["works_for", ...])` | `{head, tail}` pairs with confidence and spans |

```python
schema = (extractor.create_schema()
    .entities({"person": "Individual names", "company": "Organization names"})
    .classification("doc_type", ["contract", "invoice", "minutes"])
    .relations(["works_for", "party_to", "governed_by"])
    .structure("agreement").field("effective_date", dtype="str"))
result = extractor.extract(text, schema)      # one pass, all four
```

Two consequences matter for this design:

- **Relation extraction is a real trained head**, not something that has to be inferred from
  co-occurrence or delegated to an LLM. §3.1's defect is therefore fixable in principle, not only
  suppressible.
- **Document classification is a real head**, so `doc_type` and topic tags do not have to be
  derived from label histograms or regex vocabulary lists like `classifyDocumentVertical`
  (`worker/pipeline.ts:404-412`).

Reference checkpoints are `fastino/gliner2-base-v1` (205 M) and `gliner2-large-v1` (340 M).

### 2.2 What Studio can reach today: `extract_ner` and nothing else

`xberg-gliner`'s Candle backend — the only GLiNER2 implementation reachable from the browser —
ports a deliberate subset. From `crates/xberg-gliner/src/candle/heads/mod.rs:1-6`:

> `token_gather`, `schema_gather`, `scorer` are parameter-free utilities. `span_rep`, `count_pred`,
> `count_lstm` are parametric (Task 5b). The `classifier` head from anno is **intentionally NOT
> ported**; this crate ships `extract_ner` parity only.

`Gliner2Candle` exposes exactly one extraction method, `extract_ner(text, labels, threshold)`
(`candle/model.rs:97`). The wasm binding narrows it further: `NerModel.detect(text, opts)` reads
only `categories` from `opts` (`crates/xberg-wasm/src/bridge/ner_model.rs:115-120`).

**Classification, structured extraction, and relation extraction are unreachable from Studio
today.** Not slow, not degraded — absent. Any design that assumes them is specifying upstream work
in `xberg-gliner`, and must say so.

### 2.3 The span-width ceiling: 8, not 12, not 20

`crates/xberg-gliner/src/candle/heads/mod.rs:15-18`:

```rust
/// Maximum span width baked into the v2 Candle heads' trained weights
/// (`span_rep`'s reshape, `scorer`'s axis sizing). Model-architecture-fixed.
pub(crate) const MAX_WIDTH: usize = 8;
```

This value was challenged during review and re-verified against four independent sources on
2026-08-24. **All four agree on 8:**

| Source | Value |
| --- | --- |
| `~/.cargo` checkout @ pinned rev `9bfbc10` | `MAX_WIDTH = 8` |
| `.cargo-wasm-home` checkout @ same rev | `MAX_WIDTH = 8` |
| `xberg-io/xberg` **main**, fetched live (HEAD `aa47d873`, 2026-08-24T07:19Z — newer than our pin) | `MAX_WIDTH = 8` |
| hacienda's own compliance artifact, `hacienda-core/src/compliance/model_card.rs:92` | `MAX_WIDTH=8` |

The model card already publishes this as a documented limitation (`model_card.rs:116`: "MAX_WIDTH=8
token limitation may miss very long entity spans"), so 8 is not merely an upstream implementation
detail — it is an assertion hacienda makes in a DORA/DPIA-facing document.

**The likely source of the "20" recollection is the v1/v2 split**, and it is worth recording so this
is not re-litigated. `xberg-gliner` contains two GLiNER implementations with two different width
regimes:

| Path | Constant | Value | Configurable? |
| --- | --- | --- | --- |
| **v1 (ONNX)** | `Parameters::max_width` (`config.rs:55`) | default **12** | yes, `with_max_width()` |
| **v1 (ONNX)** | `MAX_SPAN_WIDTH` (`config.rs:25`) | **128** — validation ceiling only | n/a |
| **v2 Candle** — what Studio runs | `heads::MAX_WIDTH` (`candle/heads/mod.rs:18`) | **8** | **no** |

Only the v1 path has a tunable width. The v2 Candle path's 8 is baked into `span_rep`'s reshape and
`scorer`'s axis sizing — it is a property of the trained weight *shapes*, so raising it is a
retraining question, not a configuration one. Studio reaches only v2. No value of 20 exists on
either path.

**Fork coverage — resolved.** `Cargo.lock` pins `github.com/jamon8888/xberg.git`, whose Contents API
returns 404 to a fine-grained PAT. Git protocol access works, so the fork was cloned blobless and
swept exhaustively:

- The pinned rev `9bfbc10` exists on exactly one branch, `origin/fix/test-documents-submodule-pin`,
  where `MAX_WIDTH = 8`.
- The fork's `main` (`cfdb77345`) is **divergent, not ahead** — the pinned rev is not an ancestor of
  it, and the two lineages differ by 179 commits.
- On `main` the Candle backend has been **extracted into its own crate**,
  `crates/xberg-gliner-candle/`, which is why the old path resolves to nothing there. At its new
  home, `crates/xberg-gliner-candle/src/heads/mod.rs`, the constant is **still 8**, and the
  `classifier`-not-ported comment is still present verbatim.
- Sweeping all 28 remote branches at the new path: **26 carry the file, all with `MAX_WIDTH = 8`.
  Zero refs anywhere hold any other value.** Three commits in history ever touched the constant, and
  its current value on every ref is 8.

There is no `20` on any branch, any tag, or public upstream. **The claim is settled.**

**Words vs subword tokens — resolved, and the model card is wrong.** The unit is **words**, not
subword tokens. `decode_span_scores` (`candle/decode.rs:45-100`) indexes the scorer tensor as
`[MAX_COUNT, num_words, MAX_WIDTH, num_labels]`, iterates `width_idx ∈ 0..MAX_WIDTH` with
`end_word = start + width_idx + 1`, and maps to bytes via `words[start].start() ..
words[end_word-1].end()`. Those `words` come from `V2Splitter::split` (`v2/splitter.rs:29-35`),
a regex tokenizer (`\w+(?:[-_]\w+)*|\S` plus URL/email alternatives) — whitespace- and
punctuation-delimited words, with no BPE involved.

So the real limit is **1 to 8 words per span**. `hacienda-core/src/compliance/model_card.rs:92`
("Entities truncated at 8 subword tokens") understates the model's actual reach by roughly a factor
of two: 8 words of French legal prose is commonly 15–25 subword tokens. For a DORA/DPIA artifact
this errs conservative, but it is still an inaccurate published claim and should be corrected
(§8 step 2b).

That factor is also the most likely origin of the "20" recollection — 8 words expressed in subword
tokens lands close to 20 for this corpus. Both numbers can be simultaneously "remembered correctly"
while describing different units; only one of them is the constant in the code.

This is the single most important constraint on "semantic labels". The obvious idea — ask for
zero-shot labels like `"obligation"`, `"termination condition"`, `"condition precedent"` — asks a
span extractor capped at 8 words to return clauses. A contractual obligation is a sentence or a
paragraph. **Clause-level semantics are not expressible as GLiNER2 spans on this path**, at any
threshold, with any label wording.

What *does* fit in 8 words: entity-scale and short-phrase-scale facts — a governing-law
jurisdiction, a payment term (`"net 30 days"`), a monetary cap, a deadline, a contract reference, a
role title. Tier 1 (§5.2) is scoped to exactly these and no further.

### 2.4 The threshold floor: 0.5, not caller-settable

`crates/xberg/src/text/ner/candle.rs:22` pins `const DEFAULT_THRESHOLD: f32 = 0.5`, passed at
`:170`/`:179`. `NerModel.detect` takes no threshold argument at all. Studio can filter returned
entities upward, never admit anything below 0.5.

This matters more for semantic labels than for PII: a PII-tuned checkpoint asked for
`"governing law"` will frequently score a correct span below 0.5, and Studio cannot see it to
decide. Tier 1 is therefore a **precision-side** mechanism, and its recall ceiling is fixed
upstream. The identical constraint was recorded for verticals in
`2026-07-31-vertical-model-specialisation-design.md` §4.2; it has not moved.

### 2.5 The custom-label path is already wired, and already dead

`config.nerCustomLabels` is merged into the detect call (`worker/pipeline.ts:735-739`, added by
commit `0aa62f3`):

```ts
nerResults = await nerEngine.ner(markdown, {
  categories: [...config.nerCategories, ...config.nerCustomLabels],
});
```

37 lines later, every detection outside the fixed 10-value `NerCategory` union is discarded:

```ts
// worker/pipeline.ts:775
if (!(config.nerCategories as string[]).includes(e.category.toLowerCase())) continue;
```

and again at `:811` for the PII path. **Custom labels are scored by the model and then thrown
away.** The "Comprehensive PII" preset that `:735`'s comment says the merge exists for produces
nothing today.

The cost of this is zero-but-inverted: GLiNER2 encodes labels jointly with the text, so labels
already added to the array are already paid for in the forward pass. Tier 1's marginal inference
cost over today is the *label encoding only*, not a second pass. This is the same economics §4.1
of the vertical spec established for verticals.

### 2.6 What the export contains today

`assembleZip` (`lib/zip-export.ts:151-207`) writes:

```text
README.md                 human-facing prose (buildBundleReadme)
GLOSSARY.md               all entities, grouped by type, ungated by size
_manifest.json            file list + per-file entity counts
entities-registry.json    full registry incl. inferred relationships
documents/<path>.md       frontmatter + entity-linked prose + "## Entities"
entities/<type>-<slug>.md one file per entity + backlinks
kg-export/                neo4j.cypher, networkx.json, rdf.ttl
```

Document→entity links and entity→document backlinks both exist (`lib/annotate.ts:38-48`). There is
**no document→document link anywhere in the bundle.**

### 2.7 Inherited measurement: the heads we ship and never run

`2026-07-31-vertical-model-specialisation-design.md` §2.1 measured the checkpoint's safetensors
header: 227 tensors, 307,098,645 params, of which "Heads (`span_rep`, `count_*`, `classifier`) +
relation embeddings" account for 29,958,165 params (~60 MB at F16).

Cross-referenced against §2.2's port list, this means the `classifier` head and the relation
embeddings **are downloaded to every user's browser, held resident in wasm linear memory, and never
executed**. Not re-verified here (the weights live in IndexedDB, not on disk); flagged for
confirmation as step 0 of §8 because it bounds how much of Tier 2 is a porting job versus a
retraining job.

---

## 3. Four Defects in the Current Export

### 3.1 The knowledge graph asserts employment it has not observed

`BatchEntityRegistry.inferRelationships` (`lib/registry.ts:131-224`), called per document at
`worker/pipeline.ts:1103`, emits typed relationships from bare co-occurrence:

```ts
// registry.ts:143-160
if (e1.type === "person" && e2.type === "organization") {
  this.addRelationship(e1.id, e2.id, "works_for", `Co-occurs in document`, 0.7, "inferred");
}
```

Every person is asserted to work for every organisation named in the same document, at a hardcoded
confidence of 0.7. Every organisation pair gets reciprocal `partner_of` at 0.5. The loop is
O(n²) over a document's entities, so a 40-person, 5-organisation board pack yields ~200 false
employment claims and ~20 false partnerships from one file.

These are not internal heuristics. They are written to `entities-registry.json`, to
`neo4j.cypher` as `CREATE (a)-[:WORKS_FOR {confidence: 0.7}]->(b)`, to `networkx.json`, and to
`rdf.ttl`. `README.md` then instructs the reader to "Prefer these over re-deriving relationships
from the prose" (`lib/zip-export.ts:135-137`).

A downstream Claude Desktop session has no way to distinguish an observed relation from a
co-occurrence relabelled as one, and the bundle explicitly tells it to trust the graph over the
text. **This is the most serious defect in the export and it is not a knowledge-base-feature gap —
it is incorrect output being shipped today.** It should be fixed independently of everything else
in this spec.

### 3.2 `pseudonymize` mode empties the knowledge base of people and organisations

Three facts compose into one:

1. `nerCategoryToPiiCategory` maps `person → "person"` and `organization → "organization"`
   (`worker/pipeline.ts:350-353`), so every detected person and organisation is also a PII finding.
2. `filterExportableEntities` drops any entity with *any* span overlapping *any* PII finding, when
   `redactPiiInOutput` is set (`worker/pipeline.ts:451-457`).
3. `redactionMode: "pseudonymize"` "only takes effect when `redactPiiInOutput` is also set"
   (`lib/types.ts:133`).

Therefore, in pseudonymize mode: `entities/` contains no people and no organisations, `GLOSSARY.md`
lists none, `entities-registry.json` registers none, and all three `kg-export/` files are
correspondingly empty of them. The bundle's entire cross-document value proposition evaporates in
the exact mode the branch `feat/pseudonymization-ui-audit-reveal` exists to deliver.

The filter itself is correct for `mask`, `hash`, and `remove` — §A2's reasoning ("exporting a
redacted document beside a knowledge graph naming every entity would defeat the point") holds
there. It is wrong for `pseudonymize`, because a pseudonym token is a **stable, non-identifying,
reversible-only-with-the-key handle**: `[PERSON:session:a41f]` names the same person in every
document without disclosing who. The graph structure is exactly what pseudonymisation is designed
to preserve, and Studio currently discards it.

### 3.3 Entity identity does not survive a re-export

`const id = ent-${String(this.entities.size + 1).padStart(3, "0")}` (`lib/registry.ts:81`) — IDs are
assignment-ordinal. Adding one file to a corpus and re-exporting renumbers every entity that sorts
after the first new one. `entityFileName` (`lib/annotate.ts:28`) keys filenames on
`${type}-${slug}` from the **first mention's** surface form, so a re-run where a different document
is processed first can rename the file too.

Anything a user builds on top of a bundle — Obsidian notes, a Claude Desktop project, their own
cross-references — breaks silently on the next export. `aliases` is declared on `RegistryEntity`
(`registry.ts:11`) and initialised to `[]` (`registry.ts:91`) but **nothing ever writes to it**, so
"Acme", "Acme SAS" and "ACME S.A.S." remain three entities with three fragmented backlink sets and
three files.

### 3.4 The bundle is shaped for a reader that can hold it all at once

`GLOSSARY.md` has no size discipline (`lib/zip-export.ts:60-88`): every entity, every type. At the
~35 bytes/row the format produces, a 2,000-entity corpus is a ~70 KB file that must be read whole
to find one row. Frontmatter serialises entities as a single-line JSON blob
(`worker/pipeline.ts:482`), which `grep` cannot usefully match against. `entities-registry.json` is
pretty-printed JSON, so a `grep` for an entity ID returns one fragment of an unparseable object,
which pushes the reader to load the entire file instead.

§4 explains why this is the wrong trade for the actual consumer.

---

## 4. What a Filesystem-MCP Reader Actually Does

The consumer is Claude Desktop with filesystem access. Its access model is:
`list_directory`, `read_file`, `search_files` (name globbing), and grep over contents. **There is no
vector index and no retrieval ranking.** Navigation is: read an entry point → glob or grep → follow
a path → read.

Four design consequences follow, and they are not the ones that "make the export RAG-ready"
suggests:

1. **Filenames are the index.** A predictable, type-prefixed, stable filename is worth more than any
   amount of in-file structure, because it is the only thing `search_files` can match.
2. **Every file is a context-budget decision.** A file that must be read whole to yield one fact is
   a tax. Tiering large indexes is not cosmetic; it is what makes the bundle usable at corpus sizes
   above a few hundred entities.
3. **Line-oriented beats document-oriented for machine data.** `.jsonl` lets grep return one
   complete, parseable record. Pretty-printed JSON does not.
4. **Links must carry their reason.** `[Acme](../entities/organization-acme-sas.md)` says where to
   go, never whether it is worth going. A related-document link annotated *"shares Acme SAS, Jean
   Dupont"* lets the reader decide without opening the file.

There is one more lever, and it generalises beyond Claude: **agent runtimes auto-load a
conventionally-named instruction file from the working directory.** Claude Desktop and Claude Code
read `CLAUDE.md`; OpenAI Codex and a growing set of other agents read `AGENTS.md`. The bundle
currently ships its navigation guidance in `README.md`, which is read only if something prompts it.

Shipping **both** `CLAUDE.md` and `AGENTS.md` at the bundle root converts advisory prose into
instructions the reader actually receives, on whichever runtime opens the folder — the cheapest
single improvement available, and it requires no pipeline change at all.

The two files should carry **identical content generated from one source string**, not two
hand-maintained texts. The routing table and the redaction contract are exactly the material that
must not drift between runtimes: a bundle that explains the pseudonym scheme to Claude but not to
Codex is a bundle where one of the two silently treats `[PERSON:session:a41f]` as noise. A single
`buildAgentInstructions()` writing the same bytes to both paths makes drift structurally impossible
and costs one extra `zip.file()` call.

If a third convention appears, it is one more line at the same call site — which is the reason to
generate rather than author these.

---

## 5. Architecture: Three Tiers

The tiering mirrors `2026-07-31-vertical-model-specialisation-design.md` §4 deliberately: model
work is the last tier, entered only when the tier below is measurably insufficient.

```text
Tier 2  Multi-task     classification + relation heads    requires xberg-gliner port    NOT YET
Tier 1  Zero-shot      short-phrase semantic labels       requires deleting :775        NEXT
Tier 0  Structural     layout, identity, linking, CLAUDE.md    requires nothing         SHIP NOW
```

### 5.1 Tier 0 — Structural (no model involvement)

Everything here is derivable from data the pipeline already produces. No inference, no new labels,
no upstream dependency. This is where most of the "feels like a knowledge base" delta lives.

**5.1.1 `CLAUDE.md` + `AGENTS.md` as the routing table.** Auto-loaded by Claude and by
Codex-family agents respectively; emitted from one `buildAgentInstructions()` string to both paths
so they cannot drift (§4). Replaces prose with a decision table, and — critically for this branch —
explains the pseudonym contract:

```markdown
## Answering questions from this bundle

| Question | Read first |
| --- | --- |
| Who/what is X | `entities/<type>-<slug>.md` |
| Which documents mention X | same file, `## Appears in` |
| What entities exist | `GLOSSARY.md` (top 50) → `indexes/by-type/*.md` |
| Documents about <topic> | `indexes/by-topic/<topic>.md` |
| Cross-document / relations | `_index/entities.jsonl` (grep by id) |

Do not read every file in `documents/` to answer a cross-document question.

## Redaction contract
Processed in `pseudonymize` mode. `[PERSON:session:a41f]` is a stable token: the same
token denotes the same individual in every document in this bundle. It cannot be
resolved to a real identity without the key vault, and you should not attempt to.
Reason about tokens as first-class identities.
```

Without that last paragraph the reader treats tokens as noise; with it, the corpus stays fully
analysable while remaining redacted.

**5.1.2 Document→document edges.** The missing hop of §2.6. Computed from shared entity sets,
IDF-weighted so an entity appearing in 40 of 42 documents contributes ~nothing while one shared by
exactly two is a strong signal. Emitted with the reason attached:

```markdown
## Related documents
- [amendment-1.md](../2024/amendment-1.md) — shares Acme SAS, [PERSON:session:a41f], €4.2M
- [board-minutes-mar.md](../minutes/board-minutes-mar.md) — shares Acme SAS
```

Pure set arithmetic over `BatchEntityRegistry.docEntityMap`, which already exists.

**5.1.3 Stable, content-derived identity.** Replace ordinal `ent-NNN` with
`hash(type|canonical_name|vertical)` — `lib/content-hash.ts` is already in the tree. Populate
`aliases` by normalised-prefix and legal-suffix-stripping merge (`SAS`, `S.A.S.`, `SARL`, `Ltd`,
`GmbH`), longest surface form winning `display_name`. Fixes §3.3 in both halves.

**5.1.4 Entity files become dossiers.** `buildEntityFile` (`lib/zip-export.ts:32-55`) currently
emits metadata and a link list — it says *where* an entity appears, never *what is said about it*.
Add per-mention quoted context (±120 chars, taken from post-redaction markdown so nothing leaks),
ranked co-occurring entities, and observed date range. One file read then answers "what does this
corpus say about Acme?", which is the query a knowledge base exists to serve.

**5.1.5 Tiered, greppable indexes.**

```text
CLAUDE.md                       routing table (5.1.1)
GLOSSARY.md                     top 50 by mention count, links out
indexes/by-type/<type>.md       full listing per type
indexes/by-topic/<topic>.md     Tier 1 output; absent at Tier 0
indexes/timeline.md             date entities → chronology
_index/entities.jsonl           one entity per line
_index/documents.jsonl          one document per line
```

**5.1.6 Frontmatter as YAML, not embedded JSON.** Multi-line keys, `entity_ids`, `related`,
`topics`, `doc_type`. Greppable, and Obsidian-compatible if `[[wikilinks]]` are emitted alongside.

**5.1.7 Honest relationships.** Retype `inferRelationships`' output as `co_occurs_with` with a
proximity-derived score (same sentence ≫ same paragraph ≫ same document) replacing the hardcoded
0.7/0.5, and drop the `works_for`/`partner_of`/`contact_email` labels until a head can observe
them. Fixes §3.1 without waiting for Tier 2.

**5.1.8 Pseudonym-keyed entity files.** When `redactionMode === "pseudonymize"`, exempt entities
from `filterExportableEntities` and key them on their token instead of their surface form:
`entities/person-a41f.md`, titled `[PERSON:session:a41f]`, with full backlinks and co-occurrence.
Filename and content both stay non-identifying; the graph survives. Fixes §3.2, and is the item on
this list with the most leverage for this branch specifically.

### 5.2 Tier 1 — Zero-shot semantic labels

Delete the `:775`/`:811` filters (§2.5) and route non-`NerCategory` detections into a second bucket
that never reaches the PII/redaction path — only the semantic layer. Constraints from §2.3 and §2.4
bound the label set hard:

- **Short-phrase labels only.** `"governing law"`, `"payment term"`, `"effective date"`,
  `"contract reference"`, `"liability cap"`, `"role title"`. Not `"obligation"`, not
  `"termination condition"` — those exceed `MAX_WIDTH = 8` and cannot be returned.
- **Precision-side only.** Threshold is pinned at 0.5 upstream; Studio can raise, never lower.
  Recall is what it is.
- **Aggregated, never inlined.** Semantic spans feed a `## Key facts` block and
  `indexes/by-topic/`. They do not become inline links — a document where every phrase is a link is
  unreadable, and it would inflate the diff against `rawMarkdown` that `resolveExportContent`
  depends on (`lib/export-resolve.ts:24-38`).
- **`doc_type` stays derived, not predicted.** Without the classifier head (§2.2), document type
  comes from the semantic-label histogram plus `classifyDocumentVertical`. Calling that
  "classification" would overstate it; the frontmatter key should record it as derived.

**Expected quality, stated honestly:** the loaded checkpoint is `gliner2-guardrails-pii-f16`, a
PII-specialised fine-tune. The GLiNER Guard family's specialisation targets NER for safety and
privacy; whether general zero-shot precision on non-PII labels is retained is undocumented
upstream. Tier 1 must ship behind the §8 step 1 evaluation harness and must carry confidence into
the output, or it will produce confident noise in a compliance product.

### 5.3 Tier 2 — Multi-task schema

Everything §2.1 offers and §2.2 blocks. Entered only on evidence that Tier 1's ceiling is the
binding constraint. Two sub-items, independently valuable:

- **Classifier head port.** `xberg-gliner` declines it explicitly (§2.2). Porting it yields real
  `doc_type` and multi-label topic classification with `cls_threshold`, replacing §5.2's derived
  approximation. If §2.7 confirms the weights already ship, this is a porting job, not a training
  job.
- **Relation head port.** The principled fix for §3.1: observed `{head, tail}` pairs with
  confidence and spans, replacing co-occurrence entirely. This is the only path to a knowledge
  graph that asserts what the text actually says.

Both are upstream changes to a git-tag dependency, with the same delivery friction
`2026-07-31`'s §5.2 documented for unmerged LoRA deltas. **Do not build either speculatively.**

---

## 6. Provenance and Audit

The vertical spec's §7 argument applies unchanged and extends: once the export carries derived
semantic claims, the bundle must record what produced them.

- Every semantic label, topic, and `doc_type` in frontmatter carries its confidence and the label
  set that was requested. A label absent from the request is not evidence of absence in the text,
  and the bundle must not let that inference be made silently.
- `_manifest.json` gains the model identity (checkpoint digest), the label sets used, the redaction
  mode, and the tier that produced each derived field.
- Tier 0's relationship retyping (§5.1.7) is itself audit-relevant: bundles already exported with
  `works_for` edges are in the wild, and the manifest should make the two generations
  distinguishable.

---

## 7. Output Contract

```text
CLAUDE.md                        routing table + redaction contract      Tier 0
AGENTS.md                        byte-identical to CLAUDE.md (§4)        Tier 0
README.md                        human-facing (unchanged in role)        exists
GLOSSARY.md                      top-N index, links out                  Tier 0 (retier)
_manifest.json                   + model identity, label sets, tier      Tier 0 (extend)
entities-registry.json           + stable ids, aliases, honest edges     Tier 0 (fix)
_index/entities.jsonl            greppable entity records                Tier 0
_index/documents.jsonl           greppable document records              Tier 0
indexes/by-type/<type>.md        full per-type listings                  Tier 0
indexes/timeline.md              date-entity chronology                  Tier 0
indexes/by-topic/<topic>.md      semantic topic groupings                Tier 1
documents/<path>.md              + YAML frontmatter, ## Related          Tier 0
                                 + ## Key facts                          Tier 1
entities/<type>-<slug>.md        dossier w/ quoted context               Tier 0
entities/<type>-<token>.md       pseudonym-keyed variant                 Tier 0
kg-export/                       unchanged formats, corrected edges      Tier 0 (fix)
```

---

## 8. Sequencing and Decision Gates

| Step | Work | Gate to proceed |
| --- | --- | --- |
| 0 | Confirm §2.7: dump the cached checkpoint's tensor names; verify `classifier`/relation weights ship | none — bounds Tier 2 scope |
| 1 | Evaluation harness: labelled fr/en dev set, per-label P/R at the pinned 0.5 threshold | none — prerequisite for Tier 1's gate |
| 2 | **Fix §3.1**: retype inferred edges to `co_occurs_with` + proximity score | none — corrects shipping output |
| 2b | **Fix §2.3**: correct `model_card.rs:92`/`:116` from "8 subword tokens" to "8 words" | none — corrects a published compliance claim |
| 3 | **Fix §3.2**: pseudonym-keyed entity files, exempt from the PII filter | none — unblocks this branch's own mode |
| 4 | **Fix §3.3**: content-derived ids, alias merging | none |
| 5 | `CLAUDE.md`+`AGENTS.md`, YAML frontmatter, `## Related documents`, tiered indexes, `.jsonl` | none |
| 6 | Entity dossiers with quoted context | step 3 (context must be post-redaction) |
| 7 | Tier 1: delete `:775`/`:811` filter, semantic label sets, `## Key facts` | step 1 shows usable precision at ≥0.5 |
| 8 | Model identity + label sets in `_manifest.json` | ships with step 7 |
| 9 | Tier 2: port classifier head upstream | step 7 insufficient **and** step 0 confirms weights |
| 10 | Tier 2: port relation head upstream | step 9 landed; step 2 shown insufficient |

Steps 2–6 are independent of every unresolved question in this document and of all model work.
They can start immediately, and steps 2 and 3 are corrections to output being shipped today rather
than features.

---

## 9. Open Questions

1. **Does the guardrails checkpoint retain usable zero-shot precision off-domain?** Unanswerable
   until step 1. It decides whether Tier 1 ships at all, and whether Tier 2 is a porting job or a
   retraining job. Upstream documentation for the GLiNER Guard family does not state whether
   non-NER heads survive specialisation.
2. **Is the 0.5 threshold floor material for semantic labels?** §2.4 makes Tier 1 precision-only.
   Step 1 should measure how much correct output sits below 0.5 before anyone requests the upstream
   change to plumb a caller-supplied threshold.
3. **Should the pseudonym exemption (§5.1.8) be unconditional?** A bundle keyed on tokens is safe by
   construction, but only while the key vault is separate. If a future export ever co-packages the
   vault, the exemption becomes a leak. The invariant needs to be stated somewhere enforceable, not
   just observed.
4. **What is the right corpus-size trigger for tiering?** §5.1.5 asserts top-50 for `GLOSSARY.md`
   without evidence. The real threshold is a context-budget question that should be measured against
   a realistic bundle, not guessed.
5. **Does `## Key facts` belong in `documents/*.md` at all?** It changes the exported document
   relative to `rawMarkdown`, which interacts with the K2/I4 override paths in
   `lib/export-resolve.ts`. A sidecar (`documents/<name>.facts.md`) avoids that coupling at the cost
   of one more file per document.
6. **Who owns the upstream `xberg-gliner` changes?** Steps 9 and 10 are changes to a git-tag
   dependency, the same delivery constraint that `2026-07-31` §5.2 flagged and that remains
   unresolved.

---

## 10. Summary

The question was whether GLiNER2 can give the export semantic linking. The answer has two halves,
and the second is the one that reorders the work:

- **GLiNER2 can do far more than Studio uses** — classification, structured extraction, and *real
  relation extraction*, composable in one forward pass. The fabricated `works_for` edges in §3.1
  exist alongside a model architecture that has a trained head for exactly that relation.
- **Studio can reach none of it.** `xberg-gliner` ports `extract_ner` only and declines the
  classifier head explicitly; spans are capped at 8 words; the threshold is pinned at 0.5. Semantic
  extraction beyond short phrases is an upstream project, not a Studio feature.

Meanwhile the export ships fabricated relationships, discards every person and organisation in
pseudonymize mode, and cannot keep an identity stable across two runs. **None of those three are
model problems, and all three are worse than the missing semantic layer.**

Fix the export, then measure whether the model has anything left to add.

---

## Sources

- [GLiNER2: An Efficient Multi-Task Information Extraction System with Schema-Driven Interface (arXiv:2507.18546)](https://arxiv.org/abs/2507.18546)
- [fastino-ai/GLiNER2 — Unified Schema-Based Information Extraction](https://github.com/fastino-ai/GLiNER2)
- [GLiNER Guard: Unified Encoder Family for Production LLM Safety and Privacy (arXiv:2605.05277)](https://arxiv.org/pdf/2605.05277)
- [fastino/gliner2-multi-v1 · Hugging Face](https://huggingface.co/fastino/gliner2-multi-v1)

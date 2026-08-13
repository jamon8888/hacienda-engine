# Hacienda et Hacienda Studio — guide de contexte pour Claude Code

Document de référence en français destiné à être chargé comme contexte par Claude Code
(ou tout autre harnais lisant `AGENTS.md`) pour comprendre rapidement le domaine, les
frontières entre composants, et les pièges connus de ce dépôt. Il complète `CLAUDE.md` —
il ne le remplace pas. En cas de divergence entre ce document et le code, **le code fait
foi** (voir la note "documents de plan aspirationnels" en fin de fichier).

## 1. Qu'est-ce que Hacienda ?

Hacienda est un pipeline de conformité documentaire GDPR/DORA/AI Act : extraction de
texte multi-format, détection de PII, rédaction, revue humaine, chaîne d'audit
inviolable, et génération d'artefacts de conformité — le tout adossé au moteur
d'extraction Rust `xberg`.

```text
détection PII → fusion de spans → rédaction → chaîne d'audit
```

Cet ordre de pipeline est fixe et ne doit jamais être inversé (rédiger avant fusion
corrompt les offsets de caractères des entités qui se chevauchent).

## 2. Layout des crates (workspace Cargo)

```text
Cargo.toml → members = [
  "hacienda-core",        # logique métier privée : PII, rédaction, conformité, audit
  "hacienda",              # façade de distribution publique : re-exporte hacienda-core + xberg
  "hacienda-api",          # API REST Axum
  "hacienda-cli",          # CLI `hacienda`
  "crates/hacienda-wasm",  # même moteur compilé wasm32, consommé par Studio
  "crates/hacienda-rag",   # couche RAG (collections, retrieval, scoring)
]
```

Règle de dépendance stricte : **la logique de pipeline vit dans `hacienda-core`**, jamais
dans `hacienda` (façade) ni dans `hacienda-api`/`hacienda-cli`. Si vous devez ajouter un
comportement métier et que le seul endroit logique semble être l'API ou le CLI, c'est un
signal qu'il manque une méthode sur `HaciendaFacade` (`hacienda-core/src/facade.rs`).

Modules de `hacienda-core/src/` : `pii/`, `redaction/`, `compliance/`, `audit/`,
`review/`, `glossary/`, `auth/`, `store/`, `jobs/`, plus `facade.rs` (point d'entrée
public : `process`, `scan_text`, `redact_text`, `reveal_token_with_auth`,
`audit_entries`, `verify_audit`, `compliance_report_with_auth`, `glossary_snapshot_with_auth`,
et leurs variantes `_with_auth` pour l'application des capacités).

## 3. Pipeline PII et rédaction

- **Détection** : 42 patterns regex + modèle ML GLiNER2 optionnel (42 types d'entités).
  Le chemin NER est additif au regex — un modèle manquant ou non chargeable ne doit
  jamais casser le pipeline regex-only.
- **Fusion** : les spans détectés (regex + NER) sont fusionnés en spans
  non-chevauchants (`pii/merge.rs`) **avant** toute rédaction.
- **Rédaction** — 5 modes, un seul réversible :
  - `Mask`, `Hash`, `Remove` — irréversibles, à ne jamais présenter comme réversibles.
  - `Pseudonymize` — le seul mode réversible, AES-256-SIV. Format de token fixe :
    `[CATEGORY:key_id:base32_ciphertext]` — format à préserver pour la ré-identification
    en aval. Clés via `HACIENDA_PSEUDONYM_ACTIVE_KEY` + clés retirées pour la rotation.
    Sans clé, l'échec doit être bruyant, jamais une dégradation silencieuse vers un mode
    plus faible.
  - `Custom` — termes/patterns fournis par l'appelant (`RedactionConfig::custom_terms` /
    `custom_patterns`).
- **Normalisation** avant pseudonymisation (NFKC, casse, espaces, chiffres seuls pour les
  téléphones) pour que des mentions équivalentes résolvent au même token — sans
  sur-normaliser au point de faire collisionner des entités distinctes.
- **Audit** : chaîne append-only hachée blake3 (`audit/`) — jamais de mutation ou de
  troncature d'entrées existantes en ajoutant un nouvel événement.

## 4. Les trois surfaces d'accès — et leurs frontières

| Surface | Crate | Transport | Notes |
|---|---|---|---|
| CLI | `hacienda-cli` | process local | `extract`, `scan`, `config show`, `serve` |
| API REST | `hacienda-api` | HTTP (Axum) | jamais gRPC |
| Studio | `apps/hacienda-studio` + `crates/hacienda-wasm` | navigateur, aucun réseau | pas de client de l'API REST |

### CLI (`hacienda-cli`)

Sous-commandes réelles (voir `hacienda-cli/src/cli.rs`) :

```text
hacienda extract <INPUT...> [--mode mask|hash|pseudonymize] [--no-redact --i-accept-unredacted-pii]
                 [--vault DIR] [--audit-out PATH] [--concurrency N] [--format text|json]
hacienda scan <INPUT...> [--threshold F32] [--format text|json]   # détection seule, jamais de texte de document
hacienda config show [--format text|json]
hacienda serve [--bind ADDR]   # défaut 127.0.0.1:8787, loopback only
```

- `extract` n'a **pas** de mode de rédaction par défaut — `--mode` doit être choisi
  explicitement, jamais silencieusement décidé pour l'utilisateur.
- `scan` est détection uniquement : ne réécrit rien, ne fuite jamais le texte des
  entités par défaut.
- `serve` refuse un bind non-loopback sauf si l'authentification est activée dans la
  configuration — il n'y a pas de flag pour contourner cette règle, volontairement.
- Sous-commandes audit/review/compliance/glossary **n'existent pas** côté CLI — ne pas
  ajouter de stub qui donnerait l'impression qu'elles fonctionnent.
- Le vault produit par `--vault` (Track I2) est délibérément plus mince que celui de
  Studio : pas de `entities/`, `GLOSSARY.md`, ni `kg-export/`, car le CLI n'a pas de
  pipeline d'entités généraliste pour les peupler honnêtement.
- `--concurrency` contrôle uniquement le worker pool PII de hacienda, pas
  `extraction.concurrency.max_threads` de xberg (visible séparément via
  `hacienda config show`). Mesuré sur un corpus de 300 documents : au-delà de 1, le gain
  observé n'atteint pas 2x au nombre de cœurs — ne pas présumer que ce flag seul fait
  scaler le débit.

### API REST (`hacienda-api`)

Toutes les routes sauf `/health`, `/version`, `/info`, `/openapi.json` exigent la
capacité `Capability::DocumentsProcess`. Familles de routes actuelles :

```text
/v1/documents, /v1/documents/async, /v1/documents/{id}, /v1/documents/{id}/versions,
  /v1/documents/{id}/diff, /v1/documents/{id}/diff/{diff_job_id}
/v1/jobs, /v1/jobs/{id}, /v1/jobs/{id}/result
/v1/pii/scan, /v1/pii/redact, /v1/pii/reveal, /v1/pii/config
/v1/audit, /v1/audit/verify
/v1/review, /v1/review/{id}/decide
/v1/compliance/dpia, /v1/compliance/report
/v1/glossary
/v1/auth/keys, /v1/auth/keys/{id}, /v1/auth/config
/v1/rag/collections, /v1/rag/collections/{name},
  /v1/rag/collections/{name}/documents, /v1/rag/collections/{name}/retrieve
/v1/presets, /v1/presets/{id}
```

Toute nouvelle route doit être ajoutée au test de réflexion "guarded-routes", pas
seulement à la table de routes — sinon rien ne garantit qu'elle exige la bonne capacité.

Le document d'entrée est **toujours des octets base64 inline** — jamais un chemin ou une
URL fournis par l'appelant (prévention SSRF). Le job store est en mémoire (le backend
durable n'est pas encore construit) : les jobs sont perdus au redémarrage du process, ne
concevez pas de fonctionnalité qui suppose leur durabilité.

### Hacienda Studio (`apps/hacienda-studio`)

Workspace navigateur zero-egress : React 18 + Vite + shadcn + CodeMirror 6, exécuté
entièrement côté client dans un Web Worker (`worker/pipeline.ts`).

**Ne jamais câbler Studio sur `/v1/pii/scan`/`/v1/pii/redact`** — ces routes servent un
déploiement avec un modèle de confiance différent (serveur). Studio appelle directement
`crates/hacienda-wasm` (même moteur regex que le CLI/API, compilé `wasm32-unknown-unknown`,
exposé via `lib/pii-engine.ts`), pas la façade d'un autre binding.

État de convergence CLI/API ↔ Studio (cible : un seul moteur, `hacienda-core` en wasm32,
partagé) :

- **PII regex : unifié.** Même moteur, un seul corpus de fixtures partagé
  (`fixtures/pii-corpus.json`) asserté à la fois par `cargo test` et `vitest`.
- **Chaîne d'audit : unifiée dans le principe, pas dans la durabilité.** Même
  `AuditStore` blake3-chained, backé par IndexedDB (`hacienda-core/src/audit/store_idb.rs`,
  wasm32-only) au lieu d'un fichier. Survit à un rechargement de page, pas à un
  nettoyage du profil navigateur. Pas d'export actuel vers un vault durable.
- **Pseudonymisation réversible : Rust seulement.** Studio ne l'expose pas encore. Si
  implémentée un jour côté TypeScript/WebCrypto, elle doit produire exactement le même
  format de token — ne jamais inventer un second format.
- **NER : backend unifié, détecteur non unifié.** `worker/pipeline.ts` utilise le
  `NerModel` neuronal GLiNER2 de `@xberg-io/xberg-wasm` via `asset-loader.ts`, avec repli
  sur un bridge regex/`compromise.js` (`lib/ner-bridge.ts`) si le modèle échoue à
  charger. Le NER neuronal de `hacienda-core` (`ner-candle`) reste exclu de tous les
  builds par défaut, navigateur ou serveur — le CLI/API n'ont **aucun** NER neuronal
  aujourd'hui.
- **Vault : layout unifié, profondeur non unifiée.** Studio émet `entities/`,
  `GLOSSARY.md`, `kg-export/` en plus du layout commun (`documents/`, `_manifest.json`,
  `README.md`) ; le CLI non, par absence de pipeline d'entités généraliste.
- **Concurrence : même levier, défauts différents, délibérément.** Studio traite les
  fichiers séquentiellement (une seule instance WASM chargée, pas construite pour
  l'inférence concurrente ; du vrai parallélisme demanderait plusieurs Web Workers
  partageant le modèle — hors scope).

Ne pas "aider" en portant un détecteur Rust vers TypeScript : si une couverture PII
manque côté Studio, la correction consiste à compiler davantage de `hacienda-core` vers
wasm32, pas à le réimplémenter en TypeScript.

Modèle NER : poids GLiNER2 non quantifiés ≈ 1,23 Go — pas encore invoqué depuis le
bundle navigateur en pratique ; traiter le NER neuronal côté navigateur comme non résolu
tant que la livraison du modèle n'est pas tranchée, ne pas présumer qu'il est câblé.

## 5. Spécialisation verticale (LoRA / NER métier)

La spécialisation NER par domaine (`business_law`, `financial_services`, `m&a`) est un
adaptateur LoRA par vertical fusionné au chargement sur le modèle de base GLiNER2 —
jamais une spécialisation cuite dans les poids de base.

Pipeline de labellisation, dans l'ordre :

1. `apps/hacienda-studio/lib/verticals/<vertical>.yaml` — source de vérité unique de la
   taxonomie d'entités, partagée entre le frontend et le pipeline de labellisation. Ne
   jamais laisser diverger les jeux de labels entre les deux.
2. `labeling/taxonomy_gate.py` — rejette tout label proposé par un LLM hors taxonomie.
   Ne jamais contourner cette porte pour "avoir plus de données d'entraînement".
3. `labeling/offset_resolver.py` — résout les mentions verbatim en offsets de
   caractères de façon déterministe ; une mention non résolvable est abandonnée, jamais
   devinée.
4. `labeling/consistency.py` — exige ≥ 2/3 d'accord sur 3 échantillonnages
   d'auto-cohérence avant qu'un label soit accepté pour l'entraînement ; 1/3 d'accord
   route vers une revue humaine, n'est pas jeté.
5. `dataset/assemble.py` — convertit les spans caractères en spans de tokens GLiNER2,
   avec une assertion d'aller-retour obligatoire. Une assertion qui échoue signale un bug
   d'assemblage, pas un document d'entrée invalide.

Les splits train/val/test préservent les frontières de documents (aucune fuite au
niveau chunk) ; le test set est composé uniquement de documents entièrement revus par un
humain. Aucun poids de modèle n'est committé dans le dépôt —
`scripts/convert_gliner2_f16.py` gère la conversion F32→F16 pour la distribution hors
Git. `hacienda-core/tests/lora_adapter_contract.rs` est le test de contrat de chargement
d'adaptateur : il doit échouer bruyamment (jamais paniquer) sur un adaptateur ou un
modèle absent/malformé.

## 6. RAG (`crates/hacienda-rag`)

Couche de collections, retrieval et scoring exposée via `/v1/rag/collections*`. Modules :
`store.rs`, `query.rs`, `filter.rs`, `scoring.rs`, `capability.rs`, `backends/`. Traiter
comme une extension du pipeline documentaire, pas comme un second système de recherche —
elle consomme le même flux d'extraction/chunking que le reste de `hacienda-core`.

## 7. Pièges connus (à ne pas réintroduire)

- **Deux "moteurs PII" en apparence, un seul en réalité** : `hacienda-core` cible aussi
  `wasm32-unknown-unknown` ; Studio et le CLI/API partagent le même code de détection
  regex. Ne pas dupliquer un détecteur pensant en avoir besoin pour le navigateur.
  La vraie limite actuelle est la taille du bundle wasm (~50 Mo), pas le portage.
- **La documentation de plan (`superpowers/`, `docs/superpowers/`) est aspirationnelle**
  — elle décrit parfois des API comme construites alors qu'elles ne le sont pas encore.
  Toujours vérifier contre le code source (y compris les dépôts sœurs `xberg`,
  `xberg-pii-ecosystem` clonés en CI) avant de s'appuyer sur un plan pour une analyse
  d'écart ou une implémentation.
- **Studio n'exporte pas (encore) sa chaîne d'audit** dans un vault durable — ne pas
  l'affirmer dans l'UI ou la documentation utilisateur.
- **Le CLI n'a pas de sous-commandes conformité/audit/revue/glossaire** — absence
  délibérée, pas une lacune à combler par un stub.

## 8. Où regarder en premier selon la tâche

| Tâche | Point d'entrée |
|---|---|
| Ajouter/modifier une règle regex PII | `hacienda-core/src/pii/patterns.rs`, `engine.rs` |
| Toucher au format de token pseudonymisé | `hacienda-core/src/redaction/pseudonym.rs` |
| Ajouter une route API | `hacienda-api/src/routes.rs` + test guarded-routes |
| Ajouter une sous-commande CLI | `hacienda-cli/src/cli.rs`, `commands.rs` |
| Toucher au pipeline WASM/Studio | `crates/hacienda-wasm`, `apps/hacienda-studio/worker/pipeline.ts` |
| Ajouter un type d'entité vertical | `apps/hacienda-studio/lib/verticals/*.yaml` + `labeling/` + `dataset/` |
| Comprendre l'API publique Rust | `hacienda-core/src/facade.rs` (`HaciendaFacade`) |

Pour les conventions transverses (nommage Rust, tracing, sécurité FFI, etc.), se référer
à `CLAUDE.md` / `AGENTS.md` — ce document couvre le domaine métier hacienda, pas les
conventions d'ingénierie générales du polyrepo xberg.

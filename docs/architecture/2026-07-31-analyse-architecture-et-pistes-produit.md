# Hacienda — Analyse d'architecture et pistes d'évolution produit

**Date :** 2026-07-31
**Périmètre :** `jamon8888/hacienda-engine` à `17e9f8e`, avec `xberg` v1.0.2 comme dépendance amont
**Question posée :** l'architecture actuelle permet-elle à des entreprises de construire des solutions de *document intelligence* via API, SDK et intégrations (RAG) ? Faut-il la revoir ?

---

## 1. Résumé exécutif

Hacienda est aujourd'hui **un moteur de conformité documentaire, pas encore une plateforme de document intelligence**. La distinction n'est pas sémantique : elle décide de ce qui est vendable en l'état et de ce qui reste à construire.

Ce qui est réellement là et de bonne facture :

- un pipeline PII regex + NER, avec fusion de spans et 5 modes de rédaction ;
- une **pseudonymisation réversible AES-256-SIV** avec rotation de clés — c'est le différenciateur technique le plus fort du dépôt ;
- une **chaîne d'audit segmentée blake3**, inviolable, avec recovery après crash ;
- un **modèle de capacités** (`documents:process`, `pii:reveal`, `audit:read`…) où la table de routes est l'unique source de vérité, testée pour que l'ajout d'une route sans décision d'accès soit structurellement impossible ;
- un **Studio navigateur zéro-egress** (~7 000 lignes TS) qui produit des bundles RAG-ready (entités, glossaire, export KG Neo4j/NetworkX/RDF).

Ce qui manque pour la promesse « les entreprises construisent leurs solutions IA dessus » :

| Brique attendue | État réel |
| --- | --- |
| RAG côté serveur | **Partiel.** Chunking et embeddings existent dans `xberg` au tag épinglé, simplement désactivés ici. La couche de stockage vectoriel a **cessé d'être publiée en amont avant la GA**, mais reste disponible sous MIT dans sa dernière version — à reprendre comme code de Hacienda plutôt qu'à réécrire (§9.1, §9.3) |
| SDK 14 langages | **Annoncés ✅ dans le README, aucun généré** — `alef.toml` pointe vers 4 fichiers sources inexistants et un dossier `packages/` absent. Et c'est peut-être le mauvais chantier : le produit est un serveur d'API, donc des clients HTTP générés depuis OpenAPI coûtent bien moins cher (§9.12.1) |
| Serveur MCP | **Absent ici**, mais à moitié gratuit : `xberg` embarque un serveur MCP derrière la feature `mcp` (§9.11.1). Restent à écrire les outils PII/conformité, qui sont le différenciateur |
| Intégrations RAG (LangChain, LlamaIndex…) | **Absentes** — alors que l'amont en publie huit, versionnées en verrou avec le cœur. C'est la voie RAG la moins chère (§9.11.2) |
| Multi-tenancy | **Partielle** — isolation par `owner` au niveau des jobs seulement ; pas de tenant de premier ordre, et rien à reprendre en amont (§9.5) |
| Persistance | **Fichier uniquement** — aucun backend Postgres/S3, ce qui plafonne le déploiement à un nœud |
| Observabilité | **Docker-compose Prometheus/Grafana/Alertmanager présent, zéro instrumentation dans le code** — aucun `/metrics`, aucun OTel |
| API REST | **7 routes** (`/v1/documents`, `/v1/documents/async`, `/v1/jobs/{id}`, `/v1/pii/*`) — pas d'audit, pas de review, pas de compliance exposés |

**Verdict :** il ne faut pas réécrire. Le noyau est sain et les décisions structurantes (fail-closed, suppression du texte de span *dans le core* et non au transport, un `append` audit par document) sont défendables devant un auditeur. Mais il faut **une évolution additive assumée en trois vagues**, et surtout **arrêter d'annoncer dans le README ce qui n'existe pas** — c'est le risque commercial le plus immédiat du dépôt.

---

## 2. Inventaire factuel

### 2.1 Composition du workspace

| Crate / app | Lignes | Rôle |
| --- | ---: | --- |
| `hacienda-core` | 16 199 | Tout le métier : PII, rédaction, audit, review, glossaire, compliance, auth, jobs, façade |
| `hacienda-api` | 1 963 | Transport HTTP Axum, table de routes, DTO |
| `hacienda-cli` | 1 733 | Binaire `hacienda` : `extract`, `scan`, `config show`, `serve` |
| `crates/hacienda-wasm` | 369 | Surface navigateur (scan/redact + store IndexedDB) |
| `hacienda` | 33 | **Crate de distribution pure** — ne fait que réexporter `xberg` + `hacienda_core` |
| `apps/hacienda-studio` | ~7 000 | Workspace React/Vite zéro-egress |

La répartition est révélatrice : `hacienda-core` concentre 82 % du code Rust. C'est un monolithe modulaire, ce qui est le bon choix à ce stade — mais il n'a pas de couture pour le découpage en services quand il faudra scaler.

### 2.2 Le rôle exact de xberg

```toml
xberg = { git = "https://github.com/xberg-io/xberg.git", tag = "v1.0.2",
          features = ["redaction", "ner", "tokio-runtime"] }
```

Hacienda consomme **trois choses seulement** de xberg : `extract_batch`, `text::ner`, `types::entity`. Le commentaire du `Cargo.toml` est explicite et bien raisonné : activer `embeddings`, `reranker`, `chunking`, `captioning` tirait ONNX Runtime, `ndarray`, `liter-llm` et la famille ICU dans le graphe « pour rien », faute d'appelant.

C'est une **excellente décision d'hygiène de build** — et c'est **exactement le mur** contre lequel la promesse RAG bute. Le chunking et les embeddings sont dans xberg (4 stratégies, 4 presets, FastEmbed — cf. `.ai-rulez/skills/chunking-embeddings/`), mais Hacienda les a débranchés. Le seul endroit où le chunking tourne réellement, c'est **le navigateur**, via `WasmChunkingConfig` dans `apps/hacienda-studio/worker/pipeline.ts`.

Autrement dit : **Studio est RAG-ready, le serveur ne l'est pas.** L'asymétrie est inversée par rapport à ce qu'attend un acheteur entreprise.

Surtout — **Hacienda ne consomme qu'une seule crate d'un dépôt qui en publie quatorze**, plus 14 packages SDK déjà générés. Une couche RAG (`xberg-rag`) et une couche tenants (`xberg-doc-store`) ont existé dans ce dépôt jusqu'à la rc.5, puis ont été **retirées de l'open source avant la GA**. La §9 documente ce qui reste réellement disponible au tag épinglé et ce que cela implique.

### 2.3 Ce que la façade fait réellement

`HaciendaFacade::process_batch_with_auth` (hacienda-core/src/facade.rs:440) est le cœur :

```text
extract (xberg) → detect_concurrently (JoinSet borné) → pour chaque document :
    observe_glossary  → record_audit (1 append/doc) → submit_for_review → remplace content par redacted_text
```

Points de qualité notables :

- `detect_concurrently` (facade.rs:814) borne le parallélisme par `config.concurrency` **tout en garantissant l'ordre d'entrée** — audit et review restent séquentiels et déterministes quel que soit l'ordre de complétion ;
- la suppression du texte de span est faite **dans le core** (facade.rs:561), avec une justification explicite : « chaque futur appelant — FFI, CLI, un second transport HTTP — n'a pas à la réimplémenter, l'un d'eux oubliera » ;
- `SpanText::Include` exige `Capability::PiiReveal` **et** écrit une entrée d'audit `Reveal` par span, avec le même `span_hash` blake3 que celui écrit à la rédaction — ce qui permet à un auditeur de joindre « cette valeur a été rédigée ici » à « et ce principal l'a lue là ». C'est une réponse directe à l'AI Act Art. 12 et au RGPD Art. 30.

Ce niveau de soin est rare et constitue un actif réel.

---

## 3. Évaluation par axe : API, SDK, RAG

### 3.1 API — solide dans sa forme, très incomplète dans sa surface

La table de routes (`hacienda-api/src/routes.rs:54`) est le meilleur pattern du dépôt : chemin, décision d'accès et handler dans la même entrée, avec un test qui pilote de vraies requêtes HTTP pour vérifier que toute route non publique répond 401 sans jeton. Le commentaire de test est instructif — un skip sur les chemins paramétrés cachait un bug où `/v1/jobs/{id}` répondait 403 à *tout le monde*, propriétaire compris.

Mais la surface exposée s'arrête à 7 routes. **Rien de ce qui fait la valeur conformité n'est accessible par API** :

- pas de `GET /v1/audit/entries`, `/v1/audit/verify`, `/v1/audit/export` — alors que `AuditStore` a `entries()`, `verify()`, `seals()`, `export` CSV/JSON/JSONL ;
- pas de `/v1/review/*` — alors que `ReviewQueue` est complet (submit/assign/decide/list/stats) avec un store durable event-sourcé ;
- pas de `/v1/compliance/report` — alors que `ComplianceGenerator` produit DPIA, Model Card, DORA, AI Act, checklists.

**Le métier est écrit, il n'est simplement pas branché au transport.** C'est le meilleur ratio valeur/effort du dépôt : quelques centaines de lignes de handlers exposeraient des fonctionnalités déjà testées.

Manquent aussi, côté production : pas de `/metrics`, pas de rate limiting, pas de pagination, pas d'idempotency key, pas de webhooks, pas de versioning de schéma OpenAPI au-delà du préfixe `/v1`.

### 3.2 SDK — l'écart le plus grave, et il est de nature commerciale

Le README affiche 14 langages avec ✅ (Python, Node, WASM, Ruby, PHP, Go, Java, C#, Elixir, Dart, Kotlin, Swift, Zig, C FFI) et des commandes `pip install hacienda`, `npm install @hacienda/hacienda`.

La réalité vérifiée :

- `packages/` **n'existe pas** (exclu du workspace, jamais créé) ;
- `alef.toml` déclare comme sources `hacienda/src/cli.rs`, `hacienda/src/api.rs`, `hacienda-core/src/mcp/server.rs`, `hacienda-core/src/cli_overrides.rs` — **les quatre sont absents** ;
- `[workspace.sync].extra_paths` liste 12 manifestes de packages (`pom.xml`, `pubspec.yaml`, `Package.swift`…) dont aucun n'existe.

`task alef:generate` ne peut donc pas produire les bindings sans corriger d'abord `alef.toml`. Un prospect qui tente `pip install hacienda` après lecture du README repart avec une impression durable. **C'est à corriger avant toute démarche commerciale**, indépendamment de la roadmap technique : soit on génère, soit on passe les lignes en 🚧.

Le tableau devrait refléter la réalité : WASM est le seul binding réellement construit (`crates/hacienda-wasm`, testé sur wasm32 avec un garde-fou sur `Utc::now()`).

### 3.3 RAG — absent côté serveur, mais la conception existe déjà dans Studio

Ce que Studio produit (worker/pipeline.ts:209) est en fait une **bonne architecture RAG** :

- `documents/` — un markdown par document, frontmatter YAML, entités liées ;
- `entities/` — un fichier par entité distincte **du batch entier**, avec backlinks vers chaque document qui la mentionne ;
- `entities-registry.json` — une ligne par entité, pas une par occurrence, avec relations inférées ;
- `kg-export/` — Cypher Neo4j, NetworkX JSON, RDF/Turtle ;
- `GLOSSARY.md` — l'index d'entrée.

C'est du **GraphRAG léger**, et c'est précisément ce qu'un client entreprise veut : répondre à des questions inter-documents sans relire tout le corpus. Le README du bundle le dit correctement : « c'est ce qui rend ce bundle RAG-ready plutôt que simplement lisible ».

Le problème : cette valeur n'est disponible que si l'utilisateur ouvre un navigateur et glisse ses fichiers. **Il n'y a aucun chemin API pour l'obtenir.** Le CLI a une version délibérément amoindrie (`--vault`, sans `entities/`, sans `GLOSSARY.md`, sans KG — le commentaire est honnête : « le CLI n'a pas de pipeline NER d'entités généraliste, donc il n'y a pas de graphe à émettre honnêtement »).

Et il manque le dernier maillon : **pas de vecteurs, pas de store vectoriel, pas d'endpoint de recherche.** Le calcul (chunking, embeddings) est disponible dans `xberg` au tag épinglé et n'attend qu'une feature ; le stockage et la requête, eux, sont à construire — l'amont les a sortis de l'open source (§9.1). Le pipeline s'arrête donc à « corpus structuré et rédigé », jamais à « corpus interrogeable ».

### 3.4 Multi-tenancy et persistance — le plafond de scalabilité

L'isolation existante est réelle mais minimale : `Job.owner` porte l'identifiant de principal, et `hacienda-api/src/handlers/jobs.rs` retourne 404 (pas 403) quand un principal demande le job d'un autre — bonne défense IDOR (OWASP A01), avec des tests `two_tenant_app`.

Mais il n'y a **pas de tenant de premier ordre**, et rien à reprendre en amont : le `TenantCtx` d'`xberg-doc-store` a quitté l'open source avant la GA (§9.5). Conséquences actuelles :

- une clé de pseudonymisation est globale au process (`HACIENDA_PSEUDONYM_ACTIVE_KEY`) — deux clients partagent le même espace de tokens, donc **la même valeur chez deux clients produit le même token** : fuite d'information inter-tenant par corrélation ;
- une chaîne d'audit par nœud, pas par tenant — l'export pour un client contient les entrées de tous ;
- pas de quotas, pas de métriques de facturation, pas de config par tenant.

Côté persistance : `FileAuditStore`, `FileReviewStore`, `InMemoryJobStore`. Le design des segments d'audit anticipe correctement le multi-nœud (chaque writer possède son segment, chaînés entre eux par une seconde chaîne de seals — cf. CHANGELOG), mais **aucun backend partagé n'est implémenté**. `InMemoryJobStore` signifie qu'un redémarrage perd tous les jobs en cours, et que deux répliques ne se voient pas. `docker-compose.yml` déclare pourtant `replicaCount`-style ressources et une stack de monitoring complète : **l'infrastructure décrite suppose une architecture que le code ne supporte pas encore.**

### 3.5 Dépendance xberg — actif et risque

Le commentaire du `Cargo.toml` est remarquablement lucide : le pin sur un tag plutôt qu'un `path = "../xberg"` a été fait parce que le checkout voisin « porte des modifications non commitées, dont une qui ajoute un module `xberg::pii` réexportant les crates `pii_*` dont ce dépôt s'est justement affranchi ». Construire contre lui rendait la sortie de Hacienda fonction du travail non staged de quelqu'un.

Reste que `xberg` est un dépôt Git tiers, non vendoré (`cargo metadata --offline` échoue), qui fournit **l'extraction 97 formats et l'inférence NER** — c'est-à-dire la moitié basse de la promesse produit. Trois conséquences :

1. **Disponibilité de build** : toute indisponibilité de `github.com/xberg-io/xberg` casse CI et onboarding. Un `cargo vendor` ou un miroir interne est une assurance à faible coût.
2. **Cadence produit** : chaque format, chaque amélioration OCR dépend d'un cycle de release amont.
3. **Diligence acheteur** : un acheteur entreprise demandera qui contrôle xberg, sous quelle licence, avec quel engagement de support. À clarifier avant le premier gros contrat.

---

## 4. Faut-il revoir l'architecture ?

**Oui, mais additivement. Pas de réécriture.**

Les fondations qui doivent rester intactes :

- la façade comme point d'entrée unique (`process`, `scan_text`, `redact_text`) ;
- les traits de store (`AuditStore`, `ReviewStore`, `JobStore`) — ils sont déjà la couture qui permettra Postgres/S3 sans toucher au métier ;
- la table de routes comme source unique chemin/accès/handler ;
- les garanties appliquées dans le core, jamais au transport.

Les trois changements structurels à faire, par ordre de blocage :

### 4.1 Introduire un `TenantCtx` comme dimension de premier ordre

C'est le changement le plus intrusif et **il doit être fait tôt** — le rétro-ajouter après des données en production signifie migrer chaînes d'audit et espaces de tokens, ce qui est douloureux par construction.

À écrire dans `hacienda-core` : l'amont a retiré le sien de l'open source (§9.5), mais son modèle rc.5 reste une bonne référence de conception — un contexte passé en paramètre à chaque méthode de store plutôt qu'un champ implicite.

Portée : `Caller` porte un `TenantCtx` ; les traits de store de `hacienda-core` le prennent en paramètre de scope ; le `KeyResolver` résout par tenant (chaque tenant a son propre espace de tokens de pseudonymisation) ; les segments d'audit sont nommés par `(tenant, node)`.

Le `NodeId` des segments (`audit/segment.rs:43`) est déjà décrit comme servant « les déploiements multi-tenant » — l'intention est là, l'implémentation ne l'est pas.

### 4.2 Backends de store partagés

Implémenter `PostgresAuditStore`, `PostgresJobStore`, `PostgresReviewStore` derrière les traits existants, plus un `S3BlobStore` pour les documents et artefacts. Sans cela, l'API ne peut pas tourner à plus d'une réplique, et `InMemoryJobStore` rend `/v1/documents/async` inutilisable en production.

Le travail est cadré : les traits sont déjà async, retournent `Result`, et le CHANGELOG note explicitement qu'« un backend injoignable dit maintenant qu'il l'est au lieu de renvoyer une réponse vide ».

### 4.3 Séparer plan de contrôle et plan de données

Aujourd'hui `hacienda-cli serve` lance un binaire unique qui fait tout. La cible :

```text
┌─────────────────────────────────────────────────────────────────┐
│  PLAN DE CONTRÔLE  (stateless, scale horizontal)                │
│  API REST · MCP · Auth/tenants · Jobs · Quotas · Webhooks       │
└───────────────────────────┬─────────────────────────────────────┘
                            │
┌───────────────────────────┴─────────────────────────────────────┐
│  PLAN DE DONNÉES  (workers, scale par charge)                   │
│  extract(xberg) → NER → merge → redact → chunk → embed → index  │
│                       │                                          │
│                       └── audit chain (par tenant, append-only) │
└───────────────────────────┬─────────────────────────────────────┘
                            │
┌───────────────────────────┴─────────────────────────────────────┐
│  PERSISTANCE  Postgres (audit/review/jobs) · S3 (docs)          │
│               pgvector|Qdrant (vecteurs) · KMS (clés)           │
└─────────────────────────────────────────────────────────────────┘
```

Le point important : **la chaîne d'audit reste dans le plan de données**, jamais dans un service séparé. La garantie « une entrée par span rédigé, écrite avant que le résultat ne revienne » n'est défendable devant un régulateur que si l'écriture est dans le même chemin transactionnel que la rédaction.

---

## 5. Roadmap technique en trois vagues

### Vague 1 — « Rendre vrai ce qui est promis » (4–6 semaines)

Effort faible, débloque le commercial.

| Chantier | Détail |
| --- | --- |
| **Aligner le README** | Passer les 13 bindings non générés en 🚧, ou les générer. Corriger `alef.toml` (4 sources fantômes). Non négociable. |
| **Exposer le métier existant** | `/v1/audit/{entries,verify,export}`, `/v1/review/*`, `/v1/compliance/report`. Le code est écrit et testé, il manque les handlers. |
| **Observabilité réelle** | `/metrics` Prometheus + spans OTel sur `extract`/`ner`/`merge`/`redact`/`audit`. La stack docker-compose existe déjà et ne scrape rien. |
| **Générer 3 clients HTTP** | Étoffer `/openapi.json` (la couture existe déjà, dérivée de `ROUTE_TABLE` et protégée par un test), puis générer Python/TypeScript/Go par codegen. Bien moins cher que réparer alef pour 14 cibles natives, et aligné sur ce qu'est le produit — un serveur d'API (§9.12.1). |
| **Activer `xberg/mcp`** | Le serveur MCP d'extraction est une feature, pas un chantier (§9.11.1). Y greffer ensuite `pii_scan`, `pii_redact`, `pii_explain`. Déplacé de la vague 2. |
| **Un *document loader* LangChain** | La voie RAG la moins chère : rédaction + audit dans le chargeur, le client garde son moteur (§9.11.2). |
| **Vendorer xberg** | Miroir interne ou `cargo vendor`, pour ne plus dépendre de la disponibilité d'un dépôt tiers en CI. |

### Vague 2 — « Rendre déployable en entreprise » (2–3 mois)

| Chantier | Détail |
| --- | --- |
| **`TenantId` de premier ordre** | Caller, stores, KeyResolver, segments d'audit. À faire avant toute donnée en production. |
| **Backends Postgres + S3** | Derrière les traits existants. Débloque le multi-réplique. |
| **Clés dans un KMS** | Aujourd'hui `HACIENDA_PSEUDONYM_KEY_<ID>` en variable d'environnement (64 octets hex). Un acheteur régulé exigera Vault/KMS/HSM. Le trait `KeyResolver` est déjà la bonne couture. |
| ~~Serveur MCP~~ | **Déplacé en vague 1** — la base est une feature `xberg/mcp` (§9.11.1), pas un développement. |
| **Rate limiting, quotas, métriques de facturation** | Prérequis de toute offre SaaS. |

### Vague 3 — « Devenir une plateforme RAG » (voir l'évaluation chiffrée en §9.10)

| Chantier | Détail |
| --- | --- |
| **Activer chunking + embeddings** | Features `xberg/chunking` et `xberg/embeddings`, présentes au tag épinglé, derrière un flag optionnel pour ne pas imposer ONNX aux consommateurs qui ne s'en servent pas. |
| **Posséder le trait `VectorStore`** | Reprendre la surface de contrat et les backends MIT de la rc.5 dans `hacienda-core`, avec attribution — ~3 900 lignes sans couplage à la version de xberg (§9.3). |
| **Décorateur `HaciendaVectorStore<S>`** | Rédaction avant vectorisation, audit à la récupération, générique sur n'importe quel backend. C'est là que vit le différenciateur — voir §9.4. |
| **`POST /v1/index` + `POST /v1/search`** | Au-dessus du décorateur, sur un backend Postgres/pgvector à écrire. |
| **Porter le bundle Studio côté serveur** | Registre d'entités, glossaire, export KG — ce qui existe en TS dans `worker/pipeline.ts` remonté en Rust et exposé en API. C'est le vrai différenciateur : **du RAG dont chaque chunk est rédigé, traçable et réversible par un porteur de clé.** |
| **Verticales serveur** | Les taxonomies M&A et services financiers sont en YAML dans Studio. Les remonter en ressource serveur, versionnée, extensible par le client. |

---

## 6. Pistes business

### 6.1 Le positionnement à tenir

Le marché du RAG d'entreprise est saturé. Le marché du **RAG conforme** ne l'est pas.

La phrase à porter : **« Vos documents deviennent interrogeables par une IA sans qu'aucune donnée personnelle ne quitte votre périmètre — et vous pouvez le prouver. »**

> **Révision après lecture des specs Enterprise (§9.12.3).** La moitié « interrogeable » n'est plus différenciante : Xberg Enterprise vend déjà extraction, RAG et rédaction de base. Le poids doit porter sur **« et vous pouvez le prouver »** — pseudonymisation réversible, chaîne d'audit vérifiable, artefacts réglementaires, revue humaine, zéro-egress. C'est un périmètre plus étroit, mais c'est celui où Hacienda est seule.

Chaque mot est adossé à du code existant :

- « sans qu'aucune donnée ne quitte » → Studio zéro-egress, NER local (`model_dir`, jamais un hub id : « la détection tourne on-premise et ne doit pas atteindre le réseau à l'inférence ») ;
- « le prouver » → chaîne blake3 vérifiable, entrées `Reveal` corrélables aux entrées de rédaction par `span_hash` ;
- « interrogeables » → aujourd'hui à 70 % (les bundles KG existent, les vecteurs non).

Personne dans le RAG mainstream ne peut répondre à « qui a lu quelle donnée personnelle, quand, et sous quelle autorisation ». Hacienda le peut déjà, au niveau du span.

### 6.2 Packaging en quatre lignes

| Offre | Contenu | Cible | Prix indicatif |
| --- | --- | --- | --- |
| **Studio** | App navigateur, zéro-egress, bundles + KG | Cabinets d'avocats, M&A, due diligence | Par siège, 50–150 €/mois |
| **Engine (self-hosted)** | Binaire + Docker + Helm, licence commerciale | Banques, assurances, santé, secteur public | Licence annuelle 30–150 k€ |
| **Cloud API** | API managée, SDK, MCP, facturation à l'usage | ISV, éditeurs SaaS, intégrateurs | Par page ou par document traité |
| **Compliance Pack** | DPIA, Model Card, DORA, AI Act, registre RGPD auto-générés | Acheté par le DPO, pas par le CTO | Add-on 20–50 k€/an |

Le Compliance Pack mérite une attention particulière : **c'est le seul module dont l'acheteur n'est pas la DSI.** Un générateur de DPIA qui se remplit tout seul à partir de la config réelle du système (et non d'un template Word) est un argument de vente auprès d'un DPO qui passe des semaines à les rédiger. Le `ComplianceGenerator` existe déjà ; il n'est simplement pas exposé.

### 6.3 Verticales — commencer là où le code est déjà

Les taxonomies `m&a.yaml` et `financial_services.yaml` de Studio indiquent le bon instinct. Ordre de priorité suggéré, par ratio valeur/effort :

1. **M&A / due diligence** — data rooms, pression réglementaire, budgets élevés, taxonomie déjà écrite. Le bundle KG répond directement à « quelles entités apparaissent dans plusieurs documents » qui est *la* question d'une due diligence.
2. **Services financiers / DORA** — DORA est applicable, les rapports d'incident sont obligatoires, `compliance/dora.rs` existe.
3. **Santé** — profil HIPAA mentionné dans la config ; marché plus lent, cycles de vente longs.
4. **Secteur public / marchés publics** — l'AI Act crée une obligation, l'argument souveraineté (Rust, on-premise, zéro-egress, pas d'API américaine) est fort en Europe.

### 6.4 Modèle open source

Apache-2.0 aujourd'hui sur l'ensemble. Un découpage cohérent avec la roadmap :

- **Open (Apache-2.0)** : `hacienda-core` PII/rédaction, CLI, SDK, WASM. C'est le moteur d'adoption ;
- **Source-available / commercial** : multi-tenancy, backends Postgres/S3, KMS, Compliance Pack, SSO/SAML, Studio Enterprise.

C'est le partage classique open-core, et il tombe naturellement sur les lignes de la vague 2 — ce qui est un bon signe pour l'architecture : la frontière commerciale coïncide avec une frontière technique réelle.

### 6.5 Go-to-market

- **MCP est un canal de distribution, pas une fonctionnalité.** Un serveur MCP « redact + explain » installé dans Claude Desktop met le produit dans le flux de travail quotidien d'un analyste. Coût faible, exposition disproportionnée.
- **Studio est le meilleur outil d'avant-vente du dépôt.** Zéro installation, zéro egress, résultat visible en deux minutes. Un prospect en secteur régulé peut l'essayer sur de vrais documents sans passer par sa DSI — c'est rarissime et c'est un avantage à exploiter délibérément.
- **L'audit externe est un investissement commercial, pas un coût de conformité.** Un rapport tiers sur la chaîne blake3 et le schéma de pseudonymisation vaut plus, en cycle de vente entreprise, que six mois de fonctionnalités.

---

## 7. Risques

| Risque | Gravité | Mitigation |
| --- | --- | --- |
| README annonçant 14 SDK inexistants | **Élevée** — crédibilité, et exposition juridique si contractualisé | Corriger cette semaine |
| Dépendance à un dépôt xberg tiers non vendoré | Élevée | Vendorer/mirrorer ; clarifier gouvernance et licence |
| **Le fournisseur amont est aussi un concurrent** — Xberg Enterprise vend extraction, RAG, rédaction, audit d'activité et métrage (§9.12.2) | **Élevée** | Ne pas concurrencer sur la commodité ; concentrer le produit sur la couche de preuve, absente chez eux (§9.12.3) |
| Espace de tokens de pseudonymisation partagé entre tenants | Élevée | `TenantId` dans le `KeyResolver` avant toute mise en production multi-client |
| `InMemoryJobStore` en production | Moyenne | Backend Postgres (vague 2) |
| Aucune instrumentation malgré une stack de monitoring déclarée | Moyenne | `/metrics` + OTel (vague 1) |
| Écart Studio (riche) / serveur (pauvre) | Moyenne | Porter le pipeline de bundle côté serveur (vague 3) |
| **Une brique amont cesse d'être publiée d'une version à l'autre** — arrivé à `xberg-rag` et `xberg-doc-store` avant la GA. Risque de disponibilité, non juridique : MIT protège définitivement le publié | **Élevée** | Précédent établi : `cargo vendor` garantit que la version dont dépend le produit reste accessible (§9.7) |
| Le fork `jamon8888/xberg` (rc.5) entre dans le graphe de dépendances à la place du v1.0.2 épinglé | Moyenne | On en **extrait** du code une fois, avec attribution ; on ne le **dépend** jamais (§9.9) |
| Reprise de code MIT sans conserver la notice de copyright de Kreuzberg, Inc. | Moyenne | Notice dans chaque fichier repris + `THIRD_PARTY_LICENSES.md` ; `deny.toml` outille déjà la vérification (§9.3) |
| Clés de pseudonymisation en variables d'environnement | Moyenne | Intégration KMS/Vault (vague 2) |

---

## 8. Décisions à trancher

Ces cinq points conditionnent le reste et relèvent d'un arbitrage produit, pas technique :

1. **Cloud managé ou self-hosted d'abord ?** Le self-hosted correspond à l'ADN actuel (zéro-egress, on-premise) et raccourcit le cycle de vente en secteur régulé. Le cloud est plus scalable mais contredit l'argument de souveraineté. *Recommandation : self-hosted d'abord, cloud en vague 3.*
2. **Quelle frontière open/commercial ?** Voir §6.4.
3. **Une verticale ou une plateforme horizontale ?** Une plateforme horizontale sans référence client se vend mal. *Recommandation : M&A d'abord, horizontal ensuite.*
4. **xberg reste-t-il un fournisseur amont ou faut-il l'internaliser ?** Question de contrôle stratégique sur la moitié basse du produit.
5. ~~**RAG complet ou RAG-ready ?**~~ **Tranchée par l'exemple amont** (§9.11.2) : huit intégrations first-party LangChain / LlamaIndex / CrewAI / Spring AI / n8n montrent que le RAG-ready se livre par intégrations de frameworks, sans posséder de moteur vectoriel. Un *document loader* qui rédige et audite suffit à la promesse « RAG conforme ». Le store vectoriel devient une décision de deuxième temps, motivée par une demande client réelle.

---

## 9. Intégrer les briques xberg sans forker — état vérifié

### 9.1 Le fait déterminant : la couche RAG a cessé d'être publiée

Vérification faite directement contre `xberg-io/xberg` (joignable en git simple depuis cette session, même si l'outillage GitHub ne l'expose pas) :

| Réf | Date | Version | `xberg-rag` | `xberg-doc-store` |
| --- | --- | --- | --- | --- |
| `jamon8888/xberg` `main` (fork) | 2026-07-17 | 1.0.0-rc.5 | **présent** | **présent** |
| `xberg-io/xberg` tag `v1.0.2` *(épinglé ici)* | 2026-07-29 | 1.0.2 | **absent** | **absent** |
| `xberg-io/xberg` `main` | 2026-07-31 | 1.0.6 | **absent** | **absent** |

Les membres du workspace amont aujourd'hui sont 14 crates — `xberg`, `xberg-cli`, `xberg-ffi`, les backends OCR, les crates de binding — plus les packages Dart/Swift et le harnais de benchmark. Aucune trace de `xberg-rag`, `xberg-rag-node`, `xberg-doc-store` ni `xberg-gliner-candle`. La seule occurrence de « rag » restante est `crates/xberg/src/chunking/rag.rs` : des aides au découpage pour RAG, pas la couche de stockage vectoriel.

**Ces crates ont cessé d'être publiées entre la rc.5 et la GA 1.0.0.** Ce n'est pas une conjecture : la documentation de `xberg-rag` à la rc.5 annonçait déjà la destination — *« the engine contract that the commercial products build on: Xberg Pro and Xberg Enterprise each implement `VectorStore` externally »*. Le contrat est parti avec les produits qui l'implémentent.

**Précision importante : il ne s'agit pas d'un changement de licence.** `xberg` est MIT (Kreuzberg, Inc.) à la rc.5 comme à la 1.0.6 d'aujourd'hui, et le dépôt public reste généreux. Ce qui a changé, c'est ce qui est *publié* : les versions déjà diffusées de `xberg-rag` et `xberg-doc-store` restent MIT pour toujours, mais il n'y en aura pas de nouvelles côté open source. La distinction compte, et la §9.3 en tire la conséquence.

Kreuzberg, Inc. applique donc exactement la stratégie open-core recommandée en §6.4 : extraction, OCR et NER restent ouverts ; **la couche RAG et la multi-tenance sont devenues le produit payant**. C'est le fait le plus important de cette analyse pour la stratégie de Hacienda, et il est développé en §9.7.

### 9.2 Ce qui reste réellement consommable au tag épinglé

Au v1.0.2, ce que Hacienda peut déclarer sans rien forker :

- `xberg` — extraction 97 formats, NER, rédaction, **chunking** (`xberg/chunking`, avec `chunking/rag.rs`), embeddings, reranking, **et un serveur MCP complet** derrière la feature `mcp`. Tout cela est bien là et sous MIT.
- les crates de binding (`xberg-ffi`, `xberg-node`, `xberg-py`, `xberg-wasm`, `xberg-jni`, `xberg-php`) et 11 packages SDK générés.
- huit **intégrations de frameworks RAG** publiées sur PyPI, npm et Maven Central.

Le détail de ces trois dernières briques — et ce qu'elles changent — est en **§9.11** ; elles pèsent plus lourd que le reste de cette section.

Ce qui n'est **pas** disponible : le trait `VectorStore`, l'IR de filtres/requêtes, le registre de stores, les backends vectoriels, `TenantCtx`, `RehydrationStore`.

Conséquence directe : **le premier pas RAG reste possible et bon marché.** Activer `xberg/chunking` et `xberg/embeddings` — en features optionnelles, pour ne pas imposer ONNX aux consommateurs qui ne s'en servent pas — donne des chunks et des vecteurs. C'est la couche de *stockage et de requête* qui manque, pas la couche de calcul.

### 9.3 Trois voies pour la couche vectorielle, et une recommandation

D'abord, écarter une confusion : **la licence n'est pas le problème.** `xberg` est MIT (Kreuzberg, Inc.) à la rc.5 comme à la 1.0.6, et `xberg-rag` héritait de cette licence par `license.workspace = true`. MIT est irrévocable pour toute version publiée : le code de la rc.5 est utilisable, modifiable, redistribuable et commercialisable, définitivement. Ce qui a cessé, c'est la **publication amont** de ces crates — pas les droits sur ce qui a déjà été publié.

La bonne question n'est donc pas « avons-nous le droit ? » (oui) mais « que coûte la reprise, et de quoi héritons-nous ? ».

**Voie A — reprendre la snapshot rc.5.** Le point décisif est le **découplage** : la surface de contrat de `xberg-rag` ne référence pas xberg du tout.

```text
store.rs · types.rs · filter.rs · query.rs · registry.rs · capability.rs   → 0 import xberg
backends/{memory,sqlite,graphqlite}.rs                                     → 0 import xberg
error.rs                                                                   → 1 seule référence,
                                                                             sous #[cfg(feature = "pipeline"/"streaming")]
```

Dépendances non optionnelles : `async-trait`, `serde`, `serde_json`, `thiserror`, `tracing`. Rien d'autre.

Autrement dit, **~3 900 lignes de code MIT compilent indépendamment de la version de xberg** : le trait (114 l.), l'IR de types (272 l.), de filtres (371 l.) et de requêtes (238 l.), le registre (139 l.), et trois backends fonctionnels — mémoire (526 l.), SQLite + sqlite-vec (1 523 l.), graphqlite (513 l.). Le couplage au cœur que je redoutais n'existe que dans les features `pipeline` et `streaming`, dont Hacienda n'a pas besoin : l'orchestration d'ingestion, elle l'a déjà dans sa façade.

**Voie B — licencier Xberg Pro ou Enterprise.** Le chemin conçu par l'éditeur. Il apporte vraisemblablement le pgvector tenant-scoped. **Impossible à évaluer aujourd'hui** : dépôt privé, conditions et mode de distribution inconnus. Question n°1 à poser à Kreuzberg (§9.8).

**Voie C — réécrire le contrat de zéro dans `hacienda-core`.**

**Recommandation : A et C fusionnent, et c'est la meilleure option.** Vendorer la surface de contrat et les backends rc.5 **dans `hacienda-core`, comme code de Hacienda**, pas comme dépendance. Ce n'est pas un fork — un fork suppose un amont à suivre, et il n'y en a plus ; c'est une **reprise de code MIT devenue nôtre**, exactement ce que la licence autorise et prévoit.

Cela donne à la voie C son résultat (nous possédons la couche, indépendante d'un péage amont) sans en payer le coût (~3 900 lignes déjà écrites et testées, dont un backend SQLite non trivial). Les objections que je formulais contre la voie A — « aucun amont, aucune mise à jour, divergence permanente » — ne tiennent pas ici : ne pas avoir d'amont **est** le but recherché.

Trois obligations qui vont avec :

1. **Attribution.** MIT impose de conserver la notice de copyright de Kreuzberg, Inc. dans les fichiers repris, et de la faire apparaître dans `THIRD_PARTY_LICENSES.md`. Le `deny.toml` du dépôt outille déjà la vérification de licences.
2. **Ne reprendre que ce qui est découplé.** La surface de contrat et les backends, oui. `pipeline.rs` et `stream.rs`, non — ils compilaient contre une rc.5 et Hacienda a sa propre orchestration.
3. **Documenter la provenance.** Un en-tête de module indiquant l'origine (xberg-rag rc.5, MIT) et la raison de la reprise, pour qu'un futur mainteneur ne cherche pas un amont inexistant.

### 9.4 La forme de nos additions reste la même : le décorateur

Que le trait vienne de nous ou d'Enterprise, l'apport de Hacienda se code de la même façon — non pas un backend de plus, mais une politique appliquée par-dessus n'importe quel backend :

```text
HaciendaVectorStore<S: VectorStore>
  ├── upsert_document → rédige/pseudonymise le chunk, PUIS délègue à S
  ├── retrieve        → délègue à S, PUIS écrit une entrée d'audit par span révélé
  └── tout le reste   → délégation transparente
```

Propriétés :

- générique sur `S`, donc valable pour un store mémoire, SQLite, pgvector, ou celui du client ;
- strictement additive : rien à patcher en amont ;
- la garantie vit **dans le core**, cohérent avec la discipline déjà appliquée (`facade.rs:561`, suppression du texte de span faite en core « parce qu'un des appelants oubliera »).

C'est ce qui rend le produit défendable : un client choisit son store vectoriel **sans pouvoir contourner la rédaction ni l'audit**.

### 9.5 Multi-tenance et réversibilité : à construire ici, finalement

La §3.4 et la §4.1 de la première version de ce document annonçaient que `TenantCtx` et `RehydrationStore` existaient en amont et qu'il suffisait de les adopter. **C'est faux au tag épinglé** — ils étaient dans `xberg-doc-store`, parti avec le reste. Ces deux briques sont donc bien à construire dans `hacienda-core`, comme la §4.1 le prévoyait initialement.

Le modèle de la rc.5 reste une bonne référence de conception :

- `TenantId` / `ActorId` / `TenantCtx` — un contexte porté en paramètre par chaque méthode de store, plutôt qu'un champ implicite ;
- `RehydrationStore` — **ciphertext seulement**, jamais de passphrase ni de clé dérivée dans le trait ; identifiant assigné par le backend ; `None` renvoyé pour un document non visible du tenant, au lieu d'une erreur qui divulguerait son existence. Cette dernière propriété est la même défense IDOR que `Job.owner` applique déjà ici.

Quant aux deux schémas cryptographiques, l'arbitrage évoqué plus haut disparaît en tant que contrainte externe : sans `RehydrationStore` amont, Hacienda garde son `Pseudonymiser` AES-256-SIV comme unique mécanisme. Le déterminisme — même valeur, même token — est ce qui fait fonctionner le liage d'entités inter-documents des bundles Studio, et rien n'oblige plus à le concilier avec une carte chiffrée par document. Si la réversibilité en masse devient un besoin client, elle s'ajoute comme backend de persistance de la table de tokens, sans toucher à la frappe.

### 9.6 SDK : reprendre la configuration, pas les packages

Les 14 packages SDK amont sont bien présents au tag épinglé, générés par alef 0.30.0. Le `alef.toml` de Hacienda est en 0.44.0 et **pointe vers quatre fichiers sources inexistants** — c'est la vraie raison pour laquelle aucun binding n'est généré ici.

Copier `packages/` depuis xberg serait un fork déguisé, à resynchroniser à chaque release. La bonne manœuvre est de **reprendre la forme de configuration qui marche en amont** et de la pointer vers les fichiers réels de Hacienda. Comme la crate `hacienda` réexporte `xberg`, un utilisateur du SDK Python de Hacienda obtient les deux surfaces dans un seul package.

Note tirée de la rc.5 : `xberg-rag` était marqué *« Rust-only — deliberately not exposed through the language bindings »*. Le RAG n'a donc jamais transité par alef en amont. **Du RAG dans nos SDK serait une addition propre à Hacienda**, pas une redite — et un différenciateur réel.

### 9.7 Ce que cette découverte change stratégiquement

Hacienda est aujourd'hui positionnée **en aval d'un fournisseur qui monétise exactement la couche dont elle a besoin**. Ce n'est pas fatal, mais cela doit être décidé plutôt que subi :

- la ligne de partage à tenir reste **xberg = la capacité, Hacienda = le contrôle** — un nouveau format, une amélioration OCR, une optimisation NER se remontent en amont ; la chaîne d'audit, le modèle de capacités, les artefacts de conformité et la politique de rédaction restent ici ;
- mais **la couche vectorielle n'est plus « une capacité amont »** : c'est un produit concurrent en puissance. La posséder est autant une décision commerciale que technique ;
- le risque « dépendance à un dépôt tiers » de la §7 se précise. Il ne s'agit pas d'un risque juridique — MIT protège définitivement ce qui a été publié — mais d'un risque de **disponibilité future** : l'amont a montré, une fois, qu'une brique peut cesser d'être publiée d'une version à l'autre. Rien n'interdit que cela se reproduise sur un composant que Hacienda consomme réellement. Le `cargo vendor` recommandé en vague 1 cesse donc d'être une simple assurance de CI : c'est ce qui garantit que la version dont dépend le produit reste disponible quoi qu'il arrive en amont.

### 9.8 Ce qu'il reste à vérifier, et comment

Le dépôt « Xberg Enterprise » n'a pas pu être inspecté. `add_repo` refuse les ajouts inter-organisations, l'outillage GitHub de cette session est limité à `jamon8888`, et un sondage git des noms plausibles sous `xberg-io` (`xberg-enterprise`, `enterprise`, `xberg-pro`, `xberg-cloud`, `xberg-server`) ne renvoie rien — le dépôt est privé ou porte un autre nom.

Pour l'examiner : **ouvrir une session Claude Code avec `xberg-io/<nom-exact>` comme source initiale**, ce qui est l'unique contournement que l'outil indique lui-même. Cela suppose que le compte y ait accès. Il me faut le nom exact du dépôt.

Les questions à trancher une fois dedans, par ordre d'impact :

1. **Licence et mode de distribution.** Crate privée ? dépôt git privé ? artefact binaire ? De cela dépend si Enterprise peut être une dépendance de Hacienda et ce qu'il faut provisionner en CI.
2. **Enterprise contient-il bien `xberg-rag` et `xberg-doc-store` ?** Si oui, la voie B devient réelle et la vague 3 se raccourcit nettement.
3. **Le contrat `VectorStore` y est-il stable et documenté**, ou interne au produit ? Un contrat interne ne se décore pas.
4. **Quelle gouvernance sur l'OSS ?** Qui décide de ce qui reste ouvert, sous quel préavis. La question se pose maintenant qu'un arrêt de publication est un précédent établi.

En revanche, deux questions que je posais ici sont **résolues** et n'ont plus à attendre l'accès à Enterprise : le serveur MCP est bien dans l'OSS (`crates/xberg/src/mcp/`, en Rust — pas le répertoire TypeScript `mcp-server/` de la rc.5), et les intégrations de frameworks RAG sont publiques et publiées. Voir §9.11.

### 9.9 Le fork `jamon8888/xberg` : référence, jamais dépendance

Il est à **1.0.0-rc.5 (2026-07-17)** quand ce dépôt épingle **v1.0.2 (2026-07-29)** et que l'amont est à **1.0.6**. Le brancher en `path =` ou en `[patch]` compilerait contre du code antérieur à ce que la CI construit, sans aucun message d'erreur — exactement le scénario que le commentaire du `Cargo.toml` décrit pour le checkout voisin.

Mais sa valeur est désormais bien plus qu'archivistique : **c'est la source du code que la §9.3 recommande de reprendre.** Il conserve `xberg-rag` et `xberg-doc-store` sous MIT, dans la seule version où ils aient jamais été publiés. À ce titre il doit être **conservé et documenté comme tel** — c'est un actif, pas un résidu.

La distinction à tenir : on en **extrait** du code, une fois, qui devient du code de Hacienda avec sa notice de copyright. On ne le **dépend** jamais — ni `path =`, ni `[patch]`, ni `git =`.

### 9.10 Évaluation de la reprise : que prendre, que laisser, à quel coût

Évaluation faite sur le code rc.5 lui-même, pas sur sa documentation.

#### Trois paliers, trois verdicts

| Palier | Contenu | Lignes | Tests | Dépendances nouvelles | Verdict |
| --- | --- | ---: | ---: | --- | --- |
| **1 — surface de contrat** | `store` (114), `types` (272), `filter` (371), `query` (238), `registry` (139), `capability` (42), `error` (117) | **1 293** | 20 | **aucune** — `async-trait`, `serde`, `serde_json`, `thiserror`, `tracing` sont déjà au workspace | **Prendre** |
| **2 — backend mémoire** | `backends/memory.rs` | **526** | 5 | aucune | **Prendre** |
| **3 — backends SQLite** | `backends/sqlite.rs` (1 523), `backends/graphqlite.rs` (513) | **2 036** | 18 | `rusqlite` + `sqlite-vec` — **dépendances C** | **Différer** |

Le palier 1 est le seul indispensable : c'est lui qui porte le trait sur lequel le décorateur se branche. Le palier 2 vient gratuitement et donne de quoi tester le décorateur sans infrastructure. Le palier 3 introduit une chaîne de compilation C dans un dépôt qui n'en avait pas — à ne payer que si SQLite embarqué est un besoin produit réel, ce qui n'est pas établi.

Note sur `graphqlite` : il est gaté sur la même feature `sqlite` (pas la sienne) et apporte traversée type Cypher, détection de communautés Louvain et PageRank. C'est directement pertinent pour l'angle graphe de connaissances des bundles Studio — mais très en avance sur le besoin actuel. À noter comme option, pas à reprendre maintenant.

#### Frictions vérifiées

| Friction | Sévérité réelle |
| --- | --- |
| **Édition 2024 / `rust-version = "1.91"` amont contre édition 2021 ici** | **Faible.** C'est la friction que j'attendais bloquante ; elle ne l'est pas. Aucun `gen`, aucun `unsafe_op_in_unsafe_fn`, et les seuls retours `impl Trait` sont dans `stream.rs` — qu'on ne prend pas. Le code devrait compiler en 2021 ; à confirmer par une compilation réelle, c'est la première chose à faire. |
| **`thiserror` 2.0.18 amont contre 1.0 ici** | **Faible.** Les dérives utilisées (`#[error(...)]`, `#[error(transparent)]`, `#[from]`) sont identiques dans les deux lignes. Occasion de monter Hacienda en 2.0, ce qui est souhaitable par ailleurs. |
| **`RagError` est un type d'erreur autonome (117 l.)** | **Moyenne.** À raccorder : soit une variante `HaciendaError::Rag(#[from] RagError)`, soit un ré-hébergement des variantes. C'est le seul vrai travail d'intégration du palier 1. |
| **`rusqlite` + `sqlite-vec`** | **Moyenne, mais évitable** en différant le palier 3. Revue `deny.toml` nécessaire — l'allowlist accepte MIT, donc pas d'obstacle de principe. |
| **Aucun répertoire `tests/`** | **Faible.** Les 43 tests sont en modules `#[cfg(test)]` inline : ils viennent avec le code repris, ce qui est le bon comportement. Il n'y a en revanche pas de suite d'intégration au niveau crate. |
| **Documentation dérivée du manifeste** | **Signal, pas friction.** Le doc de `lib.rs` annonce `ahash` en dépendance ; le manifeste ne le contient pas. Ne pas se fier aux commentaires sans vérifier le code. |

#### Signaux de qualité

Le code est meilleur que ce que sa disparition pourrait laisser croire, et il est écrit dans la même maison de style que `hacienda-core` :

- **chaque méthode du trait documente ses erreurs**, la sûreté vis-à-vis des threads est énoncée, et les choix de conception sont justifiés dans le doc plutôt que laissés implicites ;
- `filter.rs` impose des **plafonds de complexité durs** — profondeur 8, 64 nœuds, 4 prédicats `text_match`, 1 024 octets de requête — avec la raison écrite : *« so a malicious or accidental filter cannot blow up a backend »*. C'est du durcissement anti-déni de service, pas une IR de confort ;
- les champs de filtre sont **whitelistés** (`validate_doc_field` / `validate_chunk_field`), donc pas d'injection de champ arbitraire ;
- la surface est **WASM-safe par construction** : `#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]`. Cela compte directement pour Studio, qui pourrait partager la même IR de requêtes que le serveur.

Deux lignes de commentaire valent une conclusion à elles seules. D'abord dans `store.rs` :

> *« The trait is deliberately **single-tenant**: one instance is one trust domain. Multi-tenancy is layered on top by the caller (e.g. one scoped instance per tenant, or a decorator that sets row-level-security context before delegating). »*

L'amont recommande donc exactement la forme proposée en §9.4 — un décorateur au-dessus d'un trait mono-tenant. Notre couche de rédaction/audit s'inscrit dans le dessin prévu, elle ne le contourne pas.

Ensuite dans `filter.rs` :

> *« Ported (de-tenanted) from the enterprise `vectorstore` crate; Postgres-specific `*_tsv` columns are not part of the neutral surface. »*

Autrement dit, **la surface OSS est elle-même un portage dé-tenanté de la crate Enterprise**. Cela confirme qu'Enterprise embarque bien une implémentation Postgres/pgvector, et surtout que ce que nous reprendrions est *architecturalement aligné* sur ce qu'Enterprise implémente. Si la voie B aboutit plus tard, un adaptateur est plausible plutôt qu'une réécriture.

#### Ce qu'il ne faut pas prendre

`pipeline.rs` et `stream.rs`, et toutes les features `pipeline-*` / `streaming`. Deux raisons : ce sont les seuls fichiers couplés à xberg — donc les seuls dont la compatibilité dépendrait de la version du cœur — et Hacienda possède déjà son orchestration d'ingestion dans la façade, avec sa concurrence bornée, son audit par document et sa file de revue. Reprendre une seconde orchestration créerait deux chemins pour la même chose.

#### `xberg-doc-store` : verdict différent

594 lignes au total, dont `tenant.rs` 59 et `rehydration.rs` 80. À cette taille, la reprise n'apporte presque rien : l'essentiel est le **dessin**, pas le code. Et un `TenantCtx` de Hacienda doit de toute façon s'articuler avec `Caller`, `Capability` et les segments d'audit, qui n'existent pas en amont.

**Recommandation : lire, s'en inspirer, écrire le nôtre.** Les deux propriétés à conserver sont énoncées en §9.5 — contexte passé en paramètre plutôt qu'implicite, et `None` plutôt qu'une erreur pour un objet hors du tenant.

#### Coût

| Lot | Estimation |
| --- | --- |
| Paliers 1 + 2 : reprise, portage édition 2021, raccord `RagError`, attribution, `deny.toml`, CI | **2–4 jours** |
| Palier 3 (SQLite/graphqlite) si retenu | +3–5 jours, dont revue des dépendances C |
| **Décorateur `HaciendaVectorStore<S>`** — le vrai travail, et il est nôtre | 1–2 semaines |
| `TenantCtx` écrit ici | 1–2 semaines |
| Backend pgvector | 2–3 semaines |
| Endpoints `/v1/index` + `/v1/search` | 1 semaine |

#### Verdict

**Reprendre les paliers 1 et 2 — 1 819 lignes, 25 tests, zéro dépendance nouvelle — est une bonne affaire nette.** Le rapport valeur/risque est inhabituellement favorable : le code est de qualité production, testé, découplé de la version de xberg, sous une licence qui l'autorise sans réserve, et il implémente précisément le contrat sur lequel le différenciateur de Hacienda vient se greffer.

La condition de sortie est simple et vérifiable en une journée : **une compilation propre en édition 2021 avec les tests amont qui passent**. Si cela échoue, la conception reste utilisable comme référence et on retombe sur une écriture guidée — sans avoir rien perdu d'autre qu'une journée.

Différer le palier 3 et écrire nous-mêmes l'équivalent de `xberg-doc-store`.

### 9.11 Ce que l'amont livre déjà : SDK, MCP, intégrations RAG, plugins agents

La §9.2 recensait ce qui reste consommable au tag épinglé et sous-estimait largement l'inventaire. Vérification faite sur le tag `v1.0.2` lui-même, quatre briques changent des conclusions prises ailleurs dans ce document.

#### 9.11.1 Serveur MCP — il est dans la crate dont Hacienda dépend déjà

C'est la découverte qui invalide le plus directement un constat du §1. Le serveur MCP n'est pas un projet séparé : il vit dans `crates/xberg/src/mcp/` — `server.rs`, `prompts.rs`, `resources.rs`, `schema.rs`, `params.rs`, `allowed_hosts.rs` — avec un test de contrat dédié (`tests/contract_mcp.rs`). Il est gaté par une simple feature :

```toml
mcp      = ["tower-service", "dep:rmcp", "tokio-runtime"]
mcp-http = ["mcp", "api"]
```

Huit outils sont exposés — `extract`, `extract_batch`, `detect_mime_type`, `cache_stats`, `cache_clear`, `cache_manifest`, `cache_warm`, `get_version` — et le README amont annonce l'ensemble comme **9 outils, 3 prompts, 4 ressources**. C'est *exactement* la forme que la spec `2025-07-24-xberg-pii-ecosystem-design.md` décrivait pour Hacienda (9 outils, 4 ressources, 3 prompts).

Le serveur est aussi distribué comme entrée de registre MCP : `server.json` à la racine déclare `io.github.xberg-io/xberg`, en image OCI `ghcr.io/xberg-io/xberg-cli`, transport stdio, lancé par `xberg mcp --transport stdio`. *(Le fichier porte `version: 5.0.0-rc.10` alors que la crate est en 1.0.2 — incohérence à éclaircir avant de s'appuyer dessus.)*

**Conséquence pour Hacienda.** Le §1 classe le serveur MCP en « absent, à écrire ». C'est vrai pour les outils PII/audit/compliance, qui sont le différenciateur — mais **faux pour la moitié extraction**, disponible par activation d'une feature sur une dépendance déjà déclarée. La bonne trajectoire n'est donc pas d'écrire un serveur MCP de zéro : c'est d'activer `xberg/mcp` et d'y **ajouter** les outils propres à Hacienda (`pii_scan`, `pii_redact`, `pii_explain`, `compliance_report`, `audit_verify`). Cela déplace le chantier MCP de la vague 2 vers la vague 1, et divise son coût.

#### 9.11.2 Intégrations RAG — first-party, publiées, versionnées en phase avec le cœur

`integrations/` regroupe huit intégrations de frameworks, chacune publiée sur le registre de son langage et **versionnée en verrou avec la release du cœur** :

| Intégration | Package | Registre |
| --- | --- | --- |
| LangChain (Python) | `langchain-xberg` | PyPI |
| LangChain (Node) | `langchain-xberg` | npm |
| LlamaIndex — readers | `llama-index-readers-xberg` | PyPI |
| LlamaIndex — node parser | `llama-index-node-parser-xberg` | PyPI |
| LlamaIndex (Node) | `llamaindex-xberg` | npm |
| CrewAI | `crewai-xberg` | PyPI |
| txtai | `txtai-xberg` | PyPI |
| SurrealDB | `surrealdb-xberg` | PyPI |
| Spring AI | `io.xberg:spring-ai-xberg` | Maven Central |
| n8n | `@xberg-io/n8n-nodes-xberg` | npm |

**C'est le constat le plus important de cette sous-section, et il rouvre la question RAG.** Le §3.3 et la §9.3 traitent le RAG comme un problème de *stockage vectoriel* — trait, IR, backends — parce que c'est la couche qui a cessé d'être publiée. Mais l'amont démontre qu'il existe une seconde voie, bien moins coûteuse : **être consommable depuis les frameworks RAG que les clients utilisent déjà**, sans posséder de store du tout.

Un `langchain-hacienda` qui expose un *document loader* rédigeant la PII, produisant des chunks déjà rédigés et une entrée d'audit par document, répond à la promesse « RAG conforme » sans écrire une seule ligne de moteur vectoriel. Le client garde son LangChain, son LlamaIndex, son pgvector — et gagne la rédaction et la traçabilité. C'est aussi la §8, décision n°5, tranchée par l'exemple : *« RAG-ready par API d'abord, vecteurs si la demande le justifie »*.

Le fait que ces intégrations soient maintenues en verrou de version avec le cœur (`task version:sync`, `scripts/sync_integration_versions.py`, publication en étage de `publish.yaml`) est également un patron à copier tel quel — c'est la discipline qui empêche une intégration de dériver de la bibliothèque qu'elle enveloppe.

#### 9.11.3 SDK — 11 packages au tag épinglé

`packages/` contient au `v1.0.2` : `csharp`, `dart`, `elixir`, `go`, `java`, `kotlin-android`, `php`, `python`, `ruby`, `swift`, `zig` — plus Node et WASM via `crates/xberg-node` et `crates/xberg-wasm`. La §9.6 reste donc valable et se renforce : la configuration alef amont **fonctionne réellement et produit des artefacts**, ce qui en fait un modèle vérifiable et non théorique pour réparer l'`alef.toml` de Hacienda.

#### 9.11.4 Plugins agents — un canal de distribution déjà outillé

`plugin/` livre un bundle multi-agents : `.claude-plugin`, `.codex-plugin`, `.cursor-plugin`, `.factory-plugin`, `.opencode`, `.hermes`, `gemini-extension.json`, `kimi.plugin.json`, et sept *skills* (`batch-extraction`, `chunking`, `extracting-keywords`, `extracting-tables`, `extracting-with-ocr`, `picking-a-format`, `xberg`). L'installation passe par `/plugin marketplace add xberg-io/xberg`.

Cela confirme l'intuition de la §6.5 — « MCP est un canal de distribution, pas une fonctionnalité » — et montre que le canal est déjà pavé. Un plugin Hacienda proposant « rédige avant de m'envoyer ce document » dans Claude Code, Cursor ou Codex met le produit dans le geste quotidien d'un analyste, pour un coût de packaging.

#### 9.11.5 Ce que cette section change

| Constat antérieur | Correction |
| --- | --- |
| §1 : « Serveur MCP — **absent**, aucun code » | Vrai pour les outils PII/conformité ; **la moitié extraction est une feature à activer** sur une dépendance déjà déclarée |
| §3.3 / §9.3 : RAG = construire une couche vectorielle | **Deuxième voie, bien moins chère** — s'intégrer à LangChain / LlamaIndex / CrewAI / Spring AI, comme l'amont le fait |
| §6.5 : « MCP est un canal de distribution » | Confirmé, et le canal est **déjà outillé** en amont — 8 cibles d'agents et 7 skills |
| §8, décision n°5 : RAG complet ou RAG-ready ? | **Tranchée par l'exemple** : le RAG-ready par intégrations est la voie éprouvée |

Cela ne change rien à la §9.1 : la couche de stockage vectoriel a bien cessé d'être publiée. Cela change en revanche son *urgence*. Les intégrations de frameworks apportent la valeur RAG plus vite et à moindre risque ; posséder un store vectoriel devient une décision de deuxième temps, motivée par une demande client réelle plutôt que par la nécessité de combler un trou.

### 9.12 `xberg-io/sdks` : deux modèles de SDK, et l'API Enterprise à découvert

La §9.11.3 comptait les 11 packages de `xberg-io/xberg` et concluait que la configuration alef amont était le modèle à suivre. C'était incomplet : **il existe un second dépôt, `xberg-io/sdks`, qui suit un modèle entièrement différent** — et il révèle au passage ce que la §9.8 disait inaccessible.

#### 9.12.1 Deux dépôts, deux natures de SDK

| | `xberg-io/xberg` → `packages/` | `xberg-io/sdks` |
| --- | --- | --- |
| Nature | **Bindings natifs** vers la bibliothèque Rust (FFI, JNI, WASM…) | **Clients HTTP** vers l'API du produit commercial |
| Langages | 11 (csharp, dart, elixir, go, java, kotlin-android, php, python, ruby, swift, zig) | 4 (python, typescript, go, dart) |
| Génération | alef, depuis les sources Rust | **openapi-python-client / openapi-typescript / oapi-codegen, depuis OpenAPI 3.1** |
| Versionnage | en verrou avec le cœur | **indépendant** (0.3.1) |
| Cible | l'utilisateur qui embarque la bibliothèque | l'utilisateur qui appelle un déploiement Enterprise ou Pro |
| Licence | MIT | MIT |

Le README de `sdks` l'énonce sans ambiguïté : *« Official client SDKs for the extraction API served by Xberg Enterprise and Xberg Pro. One package per language, one dual-target client… Generated from the upstream OpenAPI 3.1 specifications. »*

**Conséquence directe pour Hacienda, et elle est importante.** Le README de ce dépôt promet 14 bindings natifs, et l'`alef.toml` cassé est présenté partout comme le chantier SDK. Mais le produit réel de Hacienda est **un serveur d'API** : `hacienda-cli serve`, une table de routes, `/openapi.json`. Ce que ses clients consommeront, c'est HTTP — pas une bibliothèque Rust liée en statique.

Or `hacienda-api/src/handlers/openapi.rs` construit déjà un document OpenAPI 3.1 **dérivé de `ROUTE_TABLE`**, avec un test (`openapi_path_set_equals_route_table`) qui interdit la dérive. Le document est aujourd'hui squelettique — les chemins n'ont ni opérations, ni schémas de corps, ni réponses typées — mais **la couture est posée et la garantie anti-dérive existe déjà**.

Étoffer ce document, puis générer trois clients HTTP par codegen, est **considérablement moins coûteux que de réparer alef pour 14 cibles natives**, et sert mieux le produit tel qu'il est. Cela ne condamne pas les bindings natifs : ils gardent leur sens pour Studio (WASM) et pour un embarqueur qui veut du zéro-egress en process. Mais ce sont deux offres distinctes, et **le §5 vague 1 les confondait**.

#### 9.12.2 Ce que les specs révèlent de Xberg Enterprise

Les deux specs OpenAPI sont **dans ce dépôt public** : `spec/api/openapi.yaml` (Enterprise, 24 endpoints, `https://api.xberg.io`) et `spec/pro/openapi.yaml` (Pro, 20 endpoints). La §9.8 posait quatre questions en attendant un accès au dépôt privé ; la surface fonctionnelle, elle, est désormais lisible.

**Xberg Enterprise expose :**

- `POST /v1/extract`, `GET /v1/jobs`, `/v1/presets`, `/v1/uploads/presign` + `/confirm` ;
- **toute la couche RAG** — `/v1/rag/collections` (CRUD), `/documents`, `/retrieve`, `/reindex`, `/migrate-embeddings` ;
- **de la rédaction PII** — `PiiCategory` (email, phone, ssn, credit_card…), `RedactionFinding`, `RedactionReport`, et `RedactionStrategy` = `mask | hash | token_replace | drop` ;
- `GET /v1/audit`, `GET /v1/usage` (facturation à l'usage), versionnement et `diff` de documents ;
- authentification `bearer_auth`, cloisonnement par *project* (`/v1/projects/{id}/rag-config` côté Pro).

**Il faut en tirer la conclusion franche : Xberg Enterprise n'est pas seulement un fournisseur amont, c'est un produit concurrent sur une grande partie du périmètre visé par ce document.** Extraction, RAG, jobs, rédaction de base, journal d'audit, métrage d'usage : tout cela existe, est vendu, et est déjà documenté.

#### 9.12.3 Ce qu'Enterprise ne fait pas — et c'est précisément le différenciateur

La lecture des specs est aussi rassurante que dérangeante. Quatre absences, vérifiées par recherche dans les 317 Ko de la spec Enterprise :

| Capacité | Enterprise | Hacienda |
| --- | --- | --- |
| **Pseudonymisation réversible** | **Absente** — zéro occurrence de `pseudonym`. `token_replace` émet un `replacement_token`, mais aucun endpoint de révélation ni de réhydratation | AES-256-SIV déterministe, réversible par porteur de clé, rotation additive |
| **Audit infalsifiable au niveau du contenu** | **Non** — `AuditEntry` est `{id, actor, action, resource_type, metadata, created_at}` : un journal d'activité (`"job.submit"`, `"api_key.revoke"`). Aucune chaîne de hachage, aucun endpoint de vérification | Chaîne blake3 segmentée, une entrée par span rédigé, `span_hash` joignant rédaction et révélation, `verify()` |
| **Artefacts de conformité** | **Absents** — zéro occurrence de `gdpr` ; DPIA, Model Card, DORA, AI Act introuvables | DPIA, Model Card, DORA, AI Act, checklists générés |
| **Zéro-egress** | **Non** — API hébergée sur `api.xberg.io` | Studio traite dans le navigateur ; le moteur tourne sur site |
| **File de revue humaine** | Absente de la spec | Queue durable, event-sourcée (AI Act Art. 14) |

Autrement dit : **Enterprise couvre la commodité, pas la preuve.** Il rédige, mais sans réversibilité ni chaîne d'audit vérifiable ; il journalise l'activité, mais ne prouve rien sur le contenu ; il ne produit aucun artefact réglementaire.

#### 9.12.4 Ce que cela impose au positionnement

Le §6.1 proposait « vos documents deviennent interrogeables par une IA sans qu'aucune donnée personnelle ne quitte votre périmètre — et vous pouvez le prouver ». Les specs Enterprise **valident la seconde moitié de la phrase et invalident la première comme différenciateur** : rendre des documents interrogeables est désormais une commodité vendue par l'amont.

Trois conséquences, par ordre d'importance :

1. **Ne pas concurrencer sur l'extraction ni sur la plomberie RAG.** C'est le terrain d'Enterprise, il est mieux outillé, et Hacienda le consomme déjà. La §9.11.2 reste la bonne voie : s'intégrer aux frameworks, ne pas construire un moteur.
2. **Concentrer le discours sur la couche de preuve** — pseudonymisation réversible, chaîne d'audit vérifiable, artefacts réglementaires, revue humaine, zéro-egress. C'est un périmètre plus étroit que « document intelligence », mais c'est le seul où Hacienda est aujourd'hui seule, et il s'adresse à un acheteur (DPO, direction des risques) qui n'est pas celui d'Enterprise.
3. **Traiter Enterprise comme un partenaire possible autant qu'un concurrent.** Un client qui a déjà Enterprise et à qui il manque la preuve réglementaire est un prospect naturel — le décorateur de la §9.4 se pose au-dessus de *n'importe quel* backend, y compris le sien. C'est aussi ce qui rend la voie B de la §9.3 moins attirante : licencier le moteur d'un acteur dont on veut occuper la couche supérieure mérite réflexion.

#### 9.12.5 Ce que cette section change

| Constat antérieur | Correction |
| --- | --- |
| §5 vague 1 : « générer 3 SDK via alef » | **Deux offres distinctes.** Clients HTTP par codegen OpenAPI d'abord — moins cher et aligné sur le produit ; bindings natifs ensuite, pour Studio et l'embarquement |
| §9.8 : « il faut accéder au dépôt privé pour évaluer Enterprise » | **Partiellement résolu** — la surface fonctionnelle est publique via `spec/api/openapi.yaml`. Restent ouvertes les questions de licence, de distribution et de gouvernance |
| §9.7 : « l'amont monétise la couche dont vous avez besoin » | **Plus net que cela** : l'amont vend un produit concurrent sur extraction + RAG + rédaction + audit d'activité + métrage |
| §6.1 : positionnement « interrogeable + prouvable » | **La moitié « interrogeable » n'est plus différenciante.** Le discours doit porter sur la preuve |

### 9.13 Effet net sur la feuille de route

| Chantier | Estimation initiale | Révisée |
| --- | --- | --- |
| Chunking + embeddings | à réactiver | **inchangé** — bien présent au tag épinglé, en features optionnelles |
| Trait `VectorStore` + IR de requêtes | à concevoir | **~1 300 lignes MIT à reprendre**, sans couplage à la version de xberg |
| Backends vectoriels | fournis en amont | **~2 600 lignes MIT à reprendre** (mémoire, SQLite+sqlite-vec, graphqlite) ; pgvector reste à écrire |
| `TenantCtx` | fourni en amont | **à écrire ici**, sur le modèle rc.5 |
| Réversibilité | deux schémas à concilier | **un seul** — notre `Pseudonymiser` ; contrainte disparue |
| SDK | à construire | **corriger `alef.toml`** ; la config amont reste le modèle |
| Serveur MCP | un serveur amont existe | **confirmé** — `crates/xberg/src/mcp/`, feature `mcp`, 9 outils / 3 prompts / 4 ressources (§9.11.1) |
| Intégrations RAG de frameworks | non envisagées | **voie prioritaire** — 8 intégrations amont à imiter, valeur RAG sans moteur vectoriel (§9.11.2) |

La vague 3 se raccourcit donc, mais pas pour la raison que j'avais avancée d'abord. Ce n'est pas de l'intégration d'une dépendance amont : c'est une **reprise ponctuelle de code MIT qui devient le nôtre**. Le résultat est meilleur que l'intégration — Hacienda possède la couche au lieu de la louer à un fournisseur qui la monétise par ailleurs — pour un coût qui reste très inférieur à une écriture de zéro. Reste à écrire, pour de bon : le décorateur, `TenantCtx`, un backend pgvector, et les endpoints.

---

## 10. Ce qu'il faut faire en premier

> **Suite donnée.** Le découpage en specs exécutant la parité Enterprise complète, la couche
> de preuve et les LoRA métiers est dans
> `superpowers/specs/2026-08-01-hacienda-platform-parity-program.md` — dix-neuf specs en quatre
> pistes, avec invariants de programme, ordre de livraison et recommandations sur les cinq
> décisions ouvertes. Il exécute la parité, donc à l'inverse de la recommandation §9.12.4, qui
> y est notée comme réserve levée — sa décision D5 y revient et recommande une parité sélective.

Si une seule chose doit être faite cette semaine : **aligner le README sur la réalité et exposer par API le métier déjà écrit** (audit, review, compliance). Le premier point protège la crédibilité ; le second transforme ~4 000 lignes de code testé et invisible en surface produit vendable, pour quelques centaines de lignes de handlers.

Tout le reste peut suivre le séquencement des trois vagues.

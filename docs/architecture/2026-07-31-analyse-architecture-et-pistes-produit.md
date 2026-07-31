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
| RAG côté serveur (chunking, embeddings, reranking) | **Absent** — les features xberg correspondantes sont explicitement désactivées dans `Cargo.toml` |
| SDK 14 langages | **Annoncés ✅ dans le README, aucun généré** — `alef.toml` pointe vers 4 fichiers sources inexistants et un dossier `packages/` absent |
| Serveur MCP | **Absent** — spécifié (9 outils, 4 ressources, 3 prompts) mais aucun code |
| Multi-tenancy | **Partielle** — isolation par `owner` au niveau des jobs seulement ; pas de tenant de premier ordre |
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

### 2.3 Ce que la façade fait réellement

`HaciendaFacade::process_batch_with_auth` (hacienda-core/src/facade.rs:440) est le cœur :

```
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

Et il manque le dernier maillon dans tous les cas : **pas de vecteurs, pas de store vectoriel, pas d'endpoint de recherche.** Le pipeline s'arrête à « corpus structuré et rédigé », jamais à « corpus interrogeable ».

### 3.4 Multi-tenancy et persistance — le plafond de scalabilité

L'isolation existante est réelle mais minimale : `Job.owner` porte l'identifiant de principal, et `hacienda-api/src/handlers/jobs.rs` retourne 404 (pas 403) quand un principal demande le job d'un autre — bonne défense IDOR (OWASP A01), avec des tests `two_tenant_app`.

Mais il n'y a **pas de tenant de premier ordre** :

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

### 4.1 Introduire `TenantId` comme dimension de premier ordre

C'est le changement le plus intrusif et **il doit être fait tôt** — le rétro-ajouter après des données en production signifie migrer chaînes d'audit et espaces de tokens, ce qui est douloureux par construction.

Portée : `Caller` porte un `TenantId` ; les traits de store prennent un `TenantId` en paramètre de scope ; le `KeyResolver` résout par tenant (chaque tenant a son propre espace de tokens de pseudonymisation) ; les segments d'audit sont nommés par `(tenant, node)`.

Le `NodeId` des segments (`audit/segment.rs:43`) est déjà décrit comme servant « les déploiements multi-tenant » — l'intention est là, l'implémentation ne l'est pas.

### 4.2 Backends de store partagés

Implémenter `PostgresAuditStore`, `PostgresJobStore`, `PostgresReviewStore` derrière les traits existants, plus un `S3BlobStore` pour les documents et artefacts. Sans cela, l'API ne peut pas tourner à plus d'une réplique, et `InMemoryJobStore` rend `/v1/documents/async` inutilisable en production.

Le travail est cadré : les traits sont déjà async, retournent `Result`, et le CHANGELOG note explicitement qu'« un backend injoignable dit maintenant qu'il l'est au lieu de renvoyer une réponse vide ».

### 4.3 Séparer plan de contrôle et plan de données

Aujourd'hui `hacienda-cli serve` lance un binaire unique qui fait tout. La cible :

```
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
| **Générer 3 SDK** | Python, Node, Go via alef. Trois vrais valent mieux que quatorze annoncés. |
| **Vendorer xberg** | Miroir interne ou `cargo vendor`, pour ne plus dépendre de la disponibilité d'un dépôt tiers en CI. |

### Vague 2 — « Rendre déployable en entreprise » (2–3 mois)

| Chantier | Détail |
| --- | --- |
| **`TenantId` de premier ordre** | Caller, stores, KeyResolver, segments d'audit. À faire avant toute donnée en production. |
| **Backends Postgres + S3** | Derrière les traits existants. Débloque le multi-réplique. |
| **Clés dans un KMS** | Aujourd'hui `HACIENDA_PSEUDONYM_KEY_<ID>` en variable d'environnement (64 octets hex). Un acheteur régulé exigera Vault/KMS/HSM. Le trait `KeyResolver` est déjà la bonne couture. |
| **Serveur MCP** | Spécifié depuis 2025 (9 outils, 4 ressources, 3 prompts). C'est le mode d'intégration IA le plus demandé aujourd'hui, et `pii_explain` répond directement à l'AI Act Art. 13. |
| **Rate limiting, quotas, métriques de facturation** | Prérequis de toute offre SaaS. |

### Vague 3 — « Devenir une plateforme RAG » (3–4 mois)

| Chantier | Détail |
| --- | --- |
| **Réactiver chunking + embeddings** | Features xberg `chunking`/`embeddings`, derrière un feature flag pour ne pas imposer ONNX aux consommateurs qui ne l'utilisent pas. Le compromis de build documenté dans `Cargo.toml` reste valable — il faut juste le rendre optionnel plutôt qu'absent. |
| **`POST /v1/index` + `POST /v1/search`** | Store vectoriel derrière un trait `VectorStore` (pgvector d'abord, Qdrant/Weaviate ensuite). |
| **Porter le bundle Studio côté serveur** | Registre d'entités, glossaire, export KG — ce qui existe en TS dans `worker/pipeline.ts` remonté en Rust et exposé en API. C'est le vrai différenciateur : **du RAG dont chaque chunk est rédigé, traçable et réversible par un porteur de clé.** |
| **Verticales serveur** | Les taxonomies M&A et services financiers sont en YAML dans Studio. Les remonter en ressource serveur, versionnée, extensible par le client. |

---

## 6. Pistes business

### 6.1 Le positionnement à tenir

Le marché du RAG d'entreprise est saturé. Le marché du **RAG conforme** ne l'est pas.

La phrase à porter : **« Vos documents deviennent interrogeables par une IA sans qu'aucune donnée personnelle ne quitte votre périmètre — et vous pouvez le prouver. »**

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
| Espace de tokens de pseudonymisation partagé entre tenants | Élevée | `TenantId` dans le `KeyResolver` avant toute mise en production multi-client |
| `InMemoryJobStore` en production | Moyenne | Backend Postgres (vague 2) |
| Aucune instrumentation malgré une stack de monitoring déclarée | Moyenne | `/metrics` + OTel (vague 1) |
| Écart Studio (riche) / serveur (pauvre) | Moyenne | Porter le pipeline de bundle côté serveur (vague 3) |
| Clés de pseudonymisation en variables d'environnement | Moyenne | Intégration KMS/Vault (vague 2) |

---

## 8. Décisions à trancher

Ces cinq points conditionnent le reste et relèvent d'un arbitrage produit, pas technique :

1. **Cloud managé ou self-hosted d'abord ?** Le self-hosted correspond à l'ADN actuel (zéro-egress, on-premise) et raccourcit le cycle de vente en secteur régulé. Le cloud est plus scalable mais contredit l'argument de souveraineté. *Recommandation : self-hosted d'abord, cloud en vague 3.*
2. **Quelle frontière open/commercial ?** Voir §6.4.
3. **Une verticale ou une plateforme horizontale ?** Une plateforme horizontale sans référence client se vend mal. *Recommandation : M&A d'abord, horizontal ensuite.*
4. **xberg reste-t-il un fournisseur amont ou faut-il l'internaliser ?** Question de contrôle stratégique sur la moitié basse du produit.
5. **RAG complet (vecteurs + recherche) ou RAG-ready (bundles pour un moteur tiers) ?** Le second est bien moins coûteux et suffit à beaucoup de clients qui ont déjà LlamaIndex ou LangChain. *Recommandation : RAG-ready par API d'abord, vecteurs si la demande le justifie.*

---

## 9. Ce qu'il faut faire en premier

Si une seule chose doit être faite cette semaine : **aligner le README sur la réalité et exposer par API le métier déjà écrit** (audit, review, compliance). Le premier point protège la crédibilité ; le second transforme ~4 000 lignes de code testé et invisible en surface produit vendable, pour quelques centaines de lignes de handlers.

Tout le reste peut suivre le séquencement des trois vagues.

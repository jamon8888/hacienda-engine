# Programme de specs — Parité Xberg Enterprise + couche de preuve + LoRA métiers

**Date :** 2026-08-01
**Statut :** Proposé — découpage à valider avant rédaction des specs filles
**Portée :** `hacienda-engine`. Ce document ne spécifie rien lui-même : il **découpe** le
programme en specs indépendantes, fixe leurs frontières, leurs interfaces et leurs critères
de sortie.
**S'appuie sur :** `2026-07-28-hacienda-cli-api-integration-design.md` (proxy rédacteur),
`2026-07-31-vertical-model-specialisation-design.md` (PR #42/#43),
`2026-07-29-business-law-gliner2-lora-design.md` (PR #35),
`docs/architecture/2026-07-31-analyse-architecture-et-pistes-produit.md` §9.12

---

> **⚠ Alerte (2026-08-13), amendant P1 directement — corrigée.** `2026-08-13-P7-structured-field-redaction-gap.md`
> a trouvé, par reproduction, un écart vivant dans du code déjà livré : `POST /v1/documents`,
> `hacienda extract` et `hacienda-mcp` redigeaient correctement `ExtractedDocument.content` mais
> laissaient `tables`, `pages[].content` et sept autres champs structurés **en clair** dans la
> même réponse. P1 ne le couvrait pas — son garde porte sur le stockage, pas sur le corps d'une
> réponse HTTP synchrone. **Statut : corrigé** — `HaciendaFacade::redact_structured_fields`
> couvre désormais les dix champs identifiés, récursivement dans les membres d'archives ;
> reproduction et test automatisé confirment zéro occurrence résiduelle. Voir P7 §8. Voir aussi
> `2026-08-13-hacienda-xberg-capability-parity-program.md` §0, le programme frère qui a
> découvert ce point en investiguant la parité de capacités xberg.

## 1. Objectif et réserve

**Objectif demandé :** que hacienda propose tout ce que fait Xberg Enterprise, **plus** la
couche de preuve, **plus** le chargement d'adaptateurs LoRA métiers.

**Réserve, énoncée une fois puis levée.** L'analyse §9.12.4 recommandait de *ne pas*
concurrencer l'amont sur la commodité (extraction, plomberie RAG) et de concentrer l'effort
sur la couche de preuve, seul terrain où hacienda est aujourd'hui seule. La décision prise
est la parité complète. Ce document l'exécute intégralement. Deux conséquences à assumer
explicitement plutôt qu'à découvrir :

- le programme devient large — quatre pistes, dix-neuf specs — là où la couche de preuve
  seule en demandait cinq ;
- hacienda se place en concurrence frontale d'un fournisseur dont elle consomme le moteur.
  La piste **F** (§7) traite cette tension comme un objet de conception, pas comme un
  impensé.

**Principe directeur non négociable.** La parité ne doit jamais s'obtenir en dupliquant les
endpoints d'Enterprise à côté de la couche de preuve : chaque chemin capable d'émettre du
contenu documentaire traverse `HaciendaFacade`. C'est la thèse du proxy rédacteur
(`2026-07-28`, §1) portée au niveau plateforme. Une collection vectorielle contenant du texte
non rédigé annulerait la totalité de la proposition de valeur.

---

## 2. Invariants de programme

Ces quatre invariants s'appliquent à **toutes** les specs filles. Chacune doit énoncer
comment elle les respecte, et sa suite de tests doit en contenir au moins un test négatif.

| # | Invariant | Test négatif attendu |
| --- | --- | --- |
| **I1** | **Aucun texte non rédigé ne franchit une frontière de persistance.** Vecteurs, index, blobs, caches, journaux : tout contenu stocké a traversé le point d'application de la §5.1 | Écriture directe dans un store contournant le décorateur → refusée à la compilation ou au test |
| **I2** | **Toute opération sur du contenu produit une entrée d'audit avant de rendre son résultat.** Un échec d'écriture d'audit échoue l'opération ; il n'est jamais avalé | Store d'audit en échec → l'appel API retourne 5xx, pas 200 |
| **I3** | **Tout est cloisonné par tenant.** Aucune API ne peut atteindre une ressource d'un autre tenant ; l'absence se signale par 404, jamais 403 | Requête inter-tenant → 404 et aucune divulgation d'existence |
| **I4** | **La table de routes reste l'unique source de vérité** chemin / capacité / handler, et le document OpenAPI en dérive | Route ajoutée sans décision d'accès → échec du test `every_guarded_route_reflected_in_auth_state` |

---

## 2 bis. État de rédaction des specs filles

Huit specs sont rédigées : celles des vagues 0 et 1, c'est-à-dire l'horizon réellement
actionnable. Les autres restent définies par ce document — portée, interfaces, critères de
sortie — et seront rédigées quand leurs entrées existeront.

| Spec | Fichier | État |
| --- | --- | --- |
| S1 | `2026-08-01-S1-tenancy-and-projects.md` | **Rédigée** |
| S4 | `2026-08-01-S4-api-contract-and-clients.md` | **Rédigée** |
| P1 | `2026-08-01-P1-redaction-enforcement-point.md` | **Rédigée** |
| P2 | `2026-08-01-P2-audit-exposure-and-verification.md` | **Rédigée** |
| P3 | `2026-08-01-P3-pseudonymisation-as-a-service.md` | **Rédigée** |
| P4 | `2026-08-01-P4-human-review-api.md` | **Rédigée** |
| P5 | `2026-08-01-P5-compliance-artefacts-api.md` | **Rédigée** |
| E0 | `2026-08-01-E0-redacting-document-loader.md` | **Rédigée** |
| S2, S3 | — | À rédiger : dépendent du choix de backend (Postgres, KMS) |
| V1–V4 | — | À rédiger **après fusion de PR #42/#43**, dont elles prolongent les mesures |
| E1–E5 | — | À rédiger une fois P1 livré et éprouvé par E0 |

Rédiger maintenant les specs V ou E reviendrait à écrire contre des interfaces qui n'existent
pas encore et des mesures qui ne sont pas faites — c'est exactement ce que le §2 de la spec
#42 reproche aux documents de planification antérieurs.

## 3. Carte du programme

```text
┌──────────────────────────────────────────────────────────────────────┐
│  PISTE V — Verticales & LoRA         PR #35 · #41 · #42 · #43         │
│  V1 taxonomies · V2 routage adaptateurs · V3 empreinte · V4 provenance│
└────────────────────────────┬─────────────────────────────────────────┘
                             │ alimente la détection
┌────────────────────────────┴─────────────────────────────────────────┐
│  PISTE P — Couche de preuve  (le différenciateur, non contournable)   │
│  P1 point d'application · P2 audit · P3 pseudonymes · P4 revue        │
│  P5 artefacts de conformité                                           │
└────────────────────────────┬─────────────────────────────────────────┘
                             │ enveloppe TOUT contenu
┌────────────────────────────┴─────────────────────────────────────────┐
│  PISTE E — Parité Enterprise                                          │
│  E0 document loader (valide le garde) · E1 extraction+presets (min.)  │
│  E2 documents/versions/diff · E3 uploads · E4 RAG · E5 métrage (min.) │
└────────────────────────────┬─────────────────────────────────────────┘
                             │ repose sur
┌────────────────────────────┴─────────────────────────────────────────┐
│  PISTE S — Socle                                                      │
│  S1 tenants · S2 persistance · S3 jobs · S4 contrat API & clients     │
└──────────────────────────────────────────────────────────────────────┘
```

L'ordre de lecture est celui des dépendances : **S → E → P → V**. L'ordre de *livraison* ne
l'est pas — la §8 le détaille, parce que P2 et P5 exposent du métier déjà écrit et se livrent
avant la plupart de E.

---

## 4. Piste S — Socle

### S1 — Tenants, projets et autorisation

**Problème.** L'isolation actuelle se limite à `Job.owner`. Enterprise cloisonne par
*project* (`/v1/projects/{id}/rag-config`). Sans tenant de premier ordre, l'espace de tokens
de pseudonymisation est partagé entre clients : deux tenants ayant la même valeur obtiennent
le même token, ce qui est une fuite par corrélation.

**Portée.** Type `TenantCtx { tenant, actor, project }` ; `Caller` le porte ; tous les traits
de store le prennent en paramètre de scope ; `KeyResolver` résout **par tenant** ; les
segments d'audit sont nommés `(tenant, node)` ; quotas et limites par tenant.

**Hors portée.** SSO/SAML, facturation (→ E5), console d'administration.

**Interfaces produites.** `TenantCtx`, `TenantStore`, `Quota`, extension de `Capability`.

**Critères de sortie.** I3 vérifié sur chaque store ; deux tenants avec la même valeur PII
produisent des tokens **différents** ; une clé retirée d'un tenant ne déchiffre rien d'un
autre.

**Note.** Ne pas retarder : rétro-ajouter un tenant après mise en production impose de migrer
les chaînes d'audit et de re-dériver les tokens. C'est le seul chantier réellement bloquant.

### S2 — Persistance partagée

**Problème.** `FileAuditStore`, `FileReviewStore`, `InMemoryJobStore` plafonnent le
déploiement à un nœud. Enterprise sert une API multi-répliques.

**Portée.** Backends Postgres pour `AuditStore` / `ReviewStore` / `JobStore` derrière les
traits existants ; `BlobStore` (S3/MinIO) pour documents et artefacts ; migrations ;
`KeyResolver` adossé à un KMS (Vault, AWS KMS) en remplacement des variables d'environnement.

**Hors portée.** Store vectoriel (→ E4), sauvegarde/restauration.

**Critères de sortie.** Deux répliques servent le même tenant sans divergence de chaîne
d'audit ; `verify()` passe après bascule de réplique ; aucun matériel de clé en variable
d'environnement.

### S3 — Jobs durables et exécution asynchrone

**Problème.** `/v1/documents/async` s'appuie sur un store en mémoire : un redémarrage perd
les jobs. Enterprise expose `/v1/jobs`, `/v1/rag/jobs/{id}`, et des jobs de migration
d'embeddings.

**Portée.** File durable, `transition(id, from, to)` en compare-and-swap déjà défini,
workers, reprise après panne, idempotency-key, backpressure, DLQ.

**Critères de sortie.** Tuer un worker en cours de job le fait reprendre exactement une fois ;
un job soumis deux fois avec la même clé d'idempotence n'exécute qu'une fois.

### S4 — Contrat API et clients générés

**Problème.** Le `/openapi.json` actuel est squelettique : les chemins n'ont ni opérations,
ni schémas de corps, ni réponses typées. Le README promet 14 bindings natifs qui n'existent
pas, alors que le produit est un serveur d'API dont les clients parleront HTTP
(analyse §9.12.1).

**Portée.** Étoffer le document OpenAPI 3.1 dérivé de `ROUTE_TABLE` — opérations, schémas,
réponses, erreurs, sécurité — puis générer des clients Python / TypeScript / Go par codegen,
versionnés indépendamment du cœur. Le test anti-dérive existant est étendu : toute route sans
schéma de réponse échoue la CI.

**Hors portée.** Bindings natifs alef (offre distincte, pour Studio et l'embarquement en
process) ; ils gardent leur sens mais ne sont pas sur ce chemin critique.

**Critères de sortie.** Un client généré exerce chaque endpoint dans les tests d'intégration ;
`alef.toml` est soit réparé, soit ses quatre sources fantômes supprimées et le README aligné.

---

## 5. Piste P — Couche de preuve

C'est le différenciateur. Enterprise couvre la commodité et **rien** de cette piste : ni
pseudonymisation réversible, ni chaîne vérifiable, ni artefacts réglementaires, ni revue
humaine (analyse §9.12.3). Ces specs se rédigent **avant** celles de la piste E qu'elles
enveloppent, même si certaines se livrent plus tard.

### P1 — Point d'application unique de la rédaction

**Problème.** À parité fonctionnelle, hacienda gagne un store vectoriel, un store de
documents et un store de blobs. Chacun est une occasion nouvelle d'écrire du texte non
rédigé. L'invariant I1 doit être structurel, pas contractuel.

**Portée.** Le décorateur générique de l'analyse §9.4, généralisé :

```text
Guard<S>  où S ∈ { VectorStore, DocumentStore, BlobStore }
  ├── écriture  → détecte, rédige/pseudonymise, PUIS délègue à S
  ├── lecture   → délègue à S, PUIS journalise toute révélation de span
  └── reste     → délégation transparente
```

Le type non décoré n'est **pas** exportable hors du crate : un embarqueur ne peut pas obtenir
un store nu. C'est la même discipline que la suppression du texte de span faite dans le core
(`facade.rs:561`) plutôt qu'au transport.

**Critères de sortie.** Un test tente d'instancier un store nu depuis l'extérieur du crate et
**ne compile pas**. Un corpus de contrôle inséré via chaque chemin d'écriture ne laisse aucune
occurrence en clair dans le backend.

**Dépendances.** S1 (le contexte tenant traverse le garde).

### P2 — Exposition et vérification de la chaîne d'audit

**Problème.** `AuditStore` a `entries()`, `verify()`, `seals()`, l'export CSV/JSON/JSONL — et
**rien n'est exposé en HTTP**. C'est du métier écrit et testé, invisible.

**Portée.** `GET /v1/audit/entries` (paginé, filtré, scopé tenant), `GET /v1/audit/verify`,
`GET /v1/audit/export`, `GET /v1/audit/seals`. Différenciation explicite vis-à-vis d'Enterprise
dans la documentation : leur `AuditEntry` est `{actor, action, resource_type}`, un journal
d'activité ; la nôtre porte `span_hash`, `category`, `confidence`, `source`,
`pipeline_version` et un chaînage blake3 vérifiable.

**Critères de sortie.** Un client peut prouver qu'une entrée n'a pas été altérée sans accès au
stockage ; l'altération d'une entrée fait échouer `verify` en nommant l'entrée fautive.

**Livrable le moins cher du programme.** Quelques centaines de lignes de handlers.

### P3 — Pseudonymisation réversible comme service

**Problème.** `Pseudonymiser` (AES-256-SIV, déterministe, rotation additive) est le
différenciateur technique le plus fort du dépôt et n'a aucune surface API. Enterprise n'a
strictement rien d'équivalent : son `token_replace` émet un jeton sans chemin de révélation.

**Portée.** `POST /v1/pseudonyms/reveal` (capacité `pii:reveal`, une entrée d'audit `Reveal`
par span), `POST /v1/keys/rotate`, `GET /v1/keys` (identifiants seulement, jamais de matériel),
espace de tokens **par tenant** (dépend de S1), et réponse aux droits d'accès et d'effacement
RGPD adossée au déterminisme des tokens.

**Critères de sortie.** Une valeur donne le même token à travers un corpus et deux processus ;
une clé retirée reste révélable ; deux tenants ne partagent aucun token ; aucune réponse ne
contient de matériel de clé.

> **⚠ Alerte (2026-08-14), toujours ouverte.** Le « deux tenants ne partagent aucun token »
> ci-dessus n'est **pas** vérifié aujourd'hui : `KeyResolver::active()`/`resolve()` et
> `Pseudonymiser::token()` ne prennent aucun paramètre de tenant, et `HaciendaFacade` ne
> construit qu'un seul `Pseudonymiser` pour toute la durée du process — vérifié directement
> contre `hacienda-core/src/redaction/pseudonym.rs` et `facade.rs`, pas seulement contre ce
> document. S1 a livré `TenantCtx`/`TenantId`/`Caller::tenant_ctx()`, atteignables partout,
> mais jamais branchés dans la couche de pseudonymisation spécifiquement — deux tenants
> partageant un déploiement obtiennent aujourd'hui le même jeton pour la même valeur, une
> fuite par corrélation inter-clients. Architecture de correction entièrement spécifiée,
> vérifiée faisable contre le code actuel (`NerDetector` déjà `Arc`-partageable, `Caller`
> atteint déjà tous les sites d'appel nécessaires) : voir
> `2026-08-14-P3a-tenant-scoped-pseudonym-keys.md`. Pas encore implémentée.

### P4 — File de revue humaine

**Portée.** `GET /v1/review/queue`, `POST /v1/review/{id}/assign`, `POST /v1/review/{id}/decision`,
`GET /v1/review/stats`. SLA et échéances. Le store event-sourcé existe déjà.

**Critères de sortie.** Une décision survit à un redémarrage ; le compteur rendu à l'appelant
compte des acceptations du store, jamais des tentatives (AI Act Art. 14).

### P5 — Artefacts de conformité

**Portée.** `GET /v1/compliance/report`, `/dpia`, `/model-card`, `/dora`, `/ai-act`,
`/checklist`, en JSON, Markdown et PDF. Les générateurs existent ; ils doivent être **nourris
par la configuration réelle du déploiement** — profil de rédaction, modèle et adaptateur
actifs (→ V4), politique de rétention — et non par un gabarit.

**Critères de sortie.** Une DPIA générée cite l'empreinte de configuration effective et
l'identifiant de la chaîne d'audit qui l'atteste ; changer le profil de rédaction change le
document.

**Note commerciale.** C'est le seul module dont l'acheteur est le DPO et non la DSI. Il n'a
aucun équivalent chez Enterprise (`gdpr` : zéro occurrence dans leur spec).

---

## 6. Piste E — Parité Enterprise

Chaque spec de cette piste **doit** déclarer par quel garde P1 elle passe. Une spec E qui
n'énonce pas son point d'application est incomplète.

### E0 — *Document loader* rédacteur (LangChain, LlamaIndex)

**Ajoutée par la décision D1.** Livrée en vague 1, bien avant le reste de la piste.

**Portée.** Un chargeur publié sur PyPI et npm qui appelle l'API hacienda, rend des documents
et des chunks **déjà rédigés**, et attache à chaque lot l'identifiant de chaîne d'audit qui
l'atteste. Le client garde son moteur vectoriel.

**Rôle dans le programme.** C'est la **validation préalable du garde P1 contre un store tiers**
— la partie la plus difficile de E4, éprouvée sur de vrais corpus avant d'engager E4.

**Dépendances.** P1, P2 seulement. Ni S2, ni S3, ni E4.

**Critères de sortie.** Un corpus de contrôle ingéré via le chargeur dans un pgvector client
n'est récupérable en clair par aucune requête de similarité ; l'entrée d'audit correspondante
est vérifiable via P2.

### E1 — Extraction et presets *(portée minimale — D5)*

**Parité visée.** `POST /v1/extract`, `GET /v1/presets`, `/{id}`, `/{id}/sample/{name}`,
et l'équivalent Pro `/v1/saved-presets` (CRUD).

**Portée retenue.** Aligner le contrat de `/v1/documents` sur `/v1/extract`. Les presets
nommés, versionnés et partagés au niveau projet sont **hors portée** par D5 : commodité pure,
qu'aucun appel d'offres n'examine.

**Garde P1.** Le contenu extrait est rédigé avant retour et avant toute mise en cache.

### E2 — Store de documents, versions et diff

**Parité visée.** `GET /v1/documents/{id}`, `/versions`, `/diff`, `/diff/{job_id}`.

**Portée.** Persistance de documents traités, historique de versions, diff asynchrone entre
deux versions.

**Point de conception dur.** Un diff entre deux versions rédigées est peu utile ; un diff
entre versions non rédigées viole I1. **Résolution : diffuser sur les tokens
pseudonymisés.** Le déterminisme de P3 fait qu'une même valeur porte le même token d'une
version à l'autre — le diff est donc exact et lisible sans jamais manipuler de clair. C'est un
avantage direct sur Enterprise, qui ne peut pas le faire.

### E3 — Uploads présignés

**Parité visée.** `POST /v1/uploads/presign`, `/confirm`.

**Point de conception dur.** Un upload présigné écrit **directement** dans le stockage objet,
donc contourne le garde P1 par construction. Résolution : l'objet déposé est marqué
`quarantine` et **illisible par toute API** tant que `confirm` n'a pas déclenché le passage
par la façade ; l'objet clair est détruit après traitement, seule la version rédigée persiste.

**Critères de sortie.** Un objet présigné jamais confirmé n'est lisible par aucun endpoint et
expire. Aucun chemin ne rend un objet en quarantaine.

### E4 — RAG : collections, ingestion, récupération

**Parité visée.** `GET,POST /v1/rag/collections`, `GET,DELETE /{name}`,
`POST,DELETE /{name}/documents`, `POST /{name}/documents/{id}/reindex`,
`POST /{name}/migrate-embeddings` (+ suivi), `POST /{name}/retrieve`, `GET /v1/rag/jobs/{id}`.

**Portée.** Reprise sous MIT de la surface de contrat et du backend mémoire de `xberg-rag`
rc.5 (analyse §9.10, paliers 1 et 2 : 1 819 lignes, 25 tests, zéro dépendance nouvelle),
avec attribution Kreuzberg, Inc. ; activation de `xberg/chunking` et `xberg/embeddings` en
features optionnelles ; backend pgvector ; endpoints.

**Garde P1 — c'est ici qu'il compte le plus.** Chaque chunk est rédigé **avant** vectorisation.
Un index qui contiendrait des vecteurs de texte clair rendrait la PII récupérable par
similarité, ce qui annulerait tout le programme.

**Critères de sortie.** Compilation propre en édition 2021 avec les tests amont qui passent
(critère falsifiable en une journée) ; un corpus de contrôle n'est jamais récupérable en clair
par `retrieve` ; le décorateur fonctionne indifféremment sur mémoire et pgvector.

**Alternative moins chère à évaluer d'abord.** Un *document loader* LangChain / LlamaIndex qui
rédige et audite tient une grande part de la promesse « RAG conforme » sans moteur vectoriel
(analyse §9.11.2). À trancher avant d'engager E4 en entier.

### E5 — Métrage d'usage *(portée minimale — D5)*

**Parité visée.** `GET /v1/usage`.

**Portée.** Compteurs par tenant — documents, pages, jetons, appels — agrégation, export vers
un système de facturation, quotas de S1.

**Contrainte.** Les événements de métrage sont dérivés du journal d'audit, jamais d'un
comptage parallèle : deux sources divergeraient et l'audit est la source qui fait foi.

---

## 7. Piste V — Verticales et LoRA métiers

Cette piste **prolonge** les PR #35, #41, #42, #43 ; elle ne les refait pas.

### V1 — Registre de verticales côté serveur

**Problème.** Les taxonomies (`m&a.yaml`, `financial_services.yaml`, `business_law.yaml`,
`shared.yaml`) vivent dans Studio, en TypeScript. Le serveur n'y a pas accès.

**Portée.** Remonter les taxonomies en ressource serveur versionnée, exposée
(`GET /v1/verticals`, `/{id}`), extensible par le client, et partagée avec Studio comme source
unique.

**Dépendance.** S1 — une verticale peut être propre à un tenant.

### V2 — Registre d'adaptateurs et routage par requête

**Problème.** `ModelConfig.lora_adapter_dir` est **global au processus**. Servir plusieurs
verticales suppose de choisir un adaptateur **par requête ou par tenant**. Le §12.4 de la spec
d'intégration a posé le problème ; personne ne l'a résolu.

**Portée.** Registre d'adaptateurs (identifiant, verticale, version, empreinte, provenance),
résolution par requête (`vertical` explicite, défaut de tenant, ou classification de document),
chargement, et politique de repli quand l'adaptateur demandé est indisponible.

**Contrainte issue de #42.** Le repli **ne doit jamais** être silencieux vers le modèle de
base : une API qui annonce une détection spécialisée en servant du générique est le mode
d'échec que la spec d'intégration §13 interdit. Repli = erreur explicite ou dégradation
annoncée dans la réponse **et** dans l'audit.

**Critères de sortie.** Deux requêtes concurrentes sur deux verticales rendent chacune ses
entités ; un adaptateur absent produit une erreur nommée, jamais un résultat générique
silencieux.

### V3 — Empreinte mémoire et stratégie de résidence

**Problème mesuré (#42 §2).** Le merge-at-load fait payer 614 Mo (F16) pour porter 2,65 Mo
d'information spécifique — 232× de surcoût. Sous le plafond de 4 Go du conteneur, **deux
verticales sont marginales et trois ne tiennent pas**. La parité Enterprise multi-verticales
est donc *bloquée par cette mesure*, pas par le routage.

**Portée.** Trancher entre : base partagée avec adaptateurs appliqués à l'inférence plutôt que
fusionnés ; pool de processus spécialisés ; ou — voie recommandée par #42 — **Tier 0 :
`detect_with_custom(text, categories, custom_labels)`**, la capacité GLiNER2 zéro-shot déjà
payée et inutilisée, qui produit une verticale **sans aucun poids**.

**Critères de sortie.** Trois verticales servies simultanément sous 4 Go, avec une mesure, pas
une estimation.

**Ordre imposé.** V3 **précède** tout entraînement d'adaptateur. Entraîner des poids qu'on ne
peut pas charger simultanément serait un investissement mort.

### V4 — Provenance verticale dans l'audit et la model card

**Portée.** Chaque entrée d'audit porte la verticale, l'identifiant et l'empreinte de
l'adaptateur actifs. La model card (P5) énumère les adaptateurs déployés, leurs jeux
d'entraînement et leurs métriques d'évaluation.

**Justification réglementaire.** AI Act Art. 11 et 12 : la documentation technique doit
décrire *le modèle qui a réellement produit la sortie*. Avec un routage par requête, cela ne
peut venir que de l'entrée d'audit. PR #43 a commencé ce travail.

---

## 8. Ordre de livraison

L'ordre des dépendances (S → E → P → V) n'est pas l'ordre de valeur. Séquence recommandée :

| Vague | Contenu | Justification |
| --- | --- | --- |
| **0 — semaines 1-3** | **P2**, **P5**, **P4** | Exposent du métier déjà écrit et testé. Quelques centaines de lignes de handlers pour la surface la plus différenciante du produit. Aucune dépendance. |
| **1 — semaines 2-8** | **S1**, puis **S4**, puis **P1** + **E0** | S1 est le seul chantier réellement bloquant : le rétro-ajouter après production impose une migration des chaînes d'audit. S4 rend le produit consommable. E0 met le garde P1 à l'épreuve d'un store tiers (D1). |
| **2 — semaines 6-14** | **S2**, **S3**, **P3** | La persistance partagée doit précéder tout nouveau store. P3 dépend de l'espace de clés par tenant de S1. |
| **3 — semaines 10-18** | **V3**, **V1**, **V2**, **V4** | V3 d'abord : sans réponse à l'empreinte mémoire, le routage n'a rien à router. Tier 0 (D2) rend V3 livrable sans aucun poids. |
| **4 — semaines 14-26** | **E2**, **E3**, **E4**, puis **E1** et **E5** au minimum | La parité en dernier, une fois le garde P1 éprouvé par E0. Portée réduite par D5 : quatre à six semaines récupérées. |

**Chemin critique :** S1 → P1 → E0 → E4. Tout le reste peut avancer en parallèle.

**Point de contrôle après la vague 0.** Les trois specs P exposées suffisent à qualifier un
prospect en secteur régulé. Si elles ne suscitent pas d'intérêt commercial, la parité
Enterprise des vagues 4 est à rediscuter avant d'engager quinze semaines.

---

## 9. Ce que ce découpage ne couvre pas

| Exclusion | Raison |
| --- | --- |
| ~~Serveur MCP~~ | **Spécifié à part** par `2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`, comme prévu — et à l'inverse de la piste envisagée ici : pas une activation de la feature `xberg/mcp`, qui contournerait le garde P1, mais un serveur propre à hacienda contre `HaciendaFacade`. Cette même spec ferme aussi l'écart CLI/API (`audit`, `review`, `compliance` sans front-end CLI) et documente pourquoi la parité SDK (§4 S4) n'a rien de plus à construire. |
| ~~Intégrations de frameworks RAG~~ | **Rapatriées dans le programme** par la décision D1, comme spec **E0** en vague 1. |
| Capacités internes du crate `xberg` (OCR, embeddings, reranking, transcription, résumé, traduction, légendage, mots-clés, détection de mise en page) au-delà de l'extraction de formats | **Programme frère** : `2026-08-13-hacienda-xberg-capability-parity-program.md`. Distinct de la piste E ci-dessus, qui vise la parité de *surface REST* avec Xberg Enterprise — celui-ci vise la parité de *capacité pipeline* avec le crate `xberg` lui-même, une couche en-dessous. Contient l'alerte P7 ci-dessus. |
| Entraînement d'adaptateurs | PR #35 le couvre, en Python, hors de ce workspace. |
| Studio | Consomme V1 et P3 ; son évolution propre est hors programme. |
| SSO/SAML, console d'administration, facturation | Nécessaires à une offre SaaS, hors du périmètre demandé. |

---

## 10. Décisions à trancher avant rédaction des specs filles

Cinq questions conditionnent le contenu des specs filles. Chacune reçoit ci-dessous une
recommandation ferme et la contrainte qui la porte.

### D1 — E4 complet, ou *document loader* d'abord ?

**Recommandation : les deux, dans cet ordre — loader en vague 1, E4 en vague 4 comme prévu.**
Ce n'est pas un compromis : ce sont deux besoins différents.

Le loader répond à « je garde mon LangChain et mon pgvector, je veux que ce qui y entre soit
rédigé et tracé ». E4 répond à « je veux que hacienda *soit* mon moteur ». Sous mandat de
parité, E4 reste requis ; la question n'est que son rang.

L'argument décisif est technique, pas commercial : **le loader valide le contrat de P1
— rédiger avant persistance — contre un store tiers, ce qui est la partie la plus difficile de
E4.** Le faire d'abord fait hériter E4 d'un garde déjà éprouvé, sur de vrais corpus clients,
avant d'y engager six à dix semaines. L'ordre inverse fait découvrir les défauts du garde dans
le composant le plus coûteux.

**Conséquence pour les specs filles.** Ajouter une spec **E0 — *document loader* rédacteur
(LangChain, LlamaIndex)**, en vague 1, dépendant de P1 et P2 seulement. E4 la référence comme
validation préalable de son garde.

### D2 — V3 : Tier 0 zéro-shot, ou poids d'emblée ?

**Recommandation : Tier 0, sans réserve. Et faire de l'entraînement une conséquence de la
mesure, jamais une hypothèse.**

Trois faits de #42 §2 l'imposent, et ils sont mesurés :

- **zéro adaptateur entraîné existe aujourd'hui.** Tier 0 est la seule voie qui puisse livrer
  une verticale maintenant ;
- une verticale zéro-shot ne porte **aucun poids**, donc le blocage mémoire de V3 — deux
  verticales marginales, trois qui ne tiennent pas sous 4 Go — **disparaît** pour les premières
  verticales, au lieu d'être contourné ;
- `detect_with_custom(text, categories, custom_labels)` est **déjà payé et inutilisé**.

Le raisonnement de fond : entraîner des poids avant d'avoir mesuré l'insuffisance du zéro-shot
revient à investir sans hypothèse falsifiable. Le harnais d'évaluation de PR #43 existe
précisément pour produire cette mesure.

**Barrière à inscrire dans la spec V3.** Un LoRA n'est entraîné que pour une verticale dont le
F1 mesuré en Tier 0 tombe **sous le seuil produit sur le jeu d'évaluation**, et le ticket
d'entraînement cite ce chiffre.

**Effet sur PR #35.** La *conception* du pipeline reste valable et doit être conservée — c'est
la capacité, et elle est réutilisable pour toute verticale. Ce qui est différé, ce sont les
**exécutions d'entraînement**. Un pipeline conçu ne coûte rien à garder ; un entraînement
spéculatif sur une verticale que le zéro-shot couvrait déjà est une perte sèche.

### D3 — E2 : diff sur tokens pseudonymisés — validé ?

**Recommandation : validé, comme parti pris de conception — avec deux conditions que la spec
E2 doit porter explicitement.**

C'est la seule résolution compatible avec I1, et c'est un avantage net sur Enterprise, qui ne
dispose d'aucun jeton déterministe et ne peut donc pas produire ce diff.

**Condition 1 — la rotation de clés.** Le déterminisme vaut *à clé constante*. Si une rotation
survient entre v1 et v2, la même valeur porte deux jetons différents et le diff signalerait un
changement fictif. L'identifiant de clé voyageant dans le jeton (P3), la résolution est
disponible : **le diff résout via l'ensemble des clés retirées**, ou refuse explicitement de
comparer deux versions dont les clés ne sont pas rapprochables. Refuser est acceptable ;
afficher un faux changement ne l'est pas.

**Condition 2 — la fidélité dépend du mode de rédaction.** En `Pseudonymize` et en `Hash`, le
déterminisme rend le diff exact. En `Mask`, l'identité est perdue et le diff dégrade à « quelque
chose a changé ici ». **Cette dégradation doit être annoncée dans la réponse**, pas subie
silencieusement — c'est la même règle que le repli d'adaptateur de V2.

### D4 — E3 : quarantaine des uploads présignés — acceptable en performance ?

**Recommandation : adopter. La préoccupation de performance est largement mal placée ; le vrai
coût est ailleurs.**

Un upload présigné existe pour éviter de faire transiter un gros fichier **à travers le serveur
d'API**. La quarantaine ne réintroduit pas ce transit : le fichier va toujours directement au
stockage objet. Ce qui est ajouté, c'est que la *lisibilité* attend le traitement — or le
traitement d'un document est déjà asynchrone, et un appelant ne peut de toute façon pas lire un
résultat avant qu'il existe. Il n'y a donc pas de latence nouvelle sur le chemin qui compte.

Le coût réel est le **cycle de vie du stockage** : pendant une fenêtre, l'original en clair et
la sortie rédigée coexistent. C'est une exposition, pas une lenteur.

**Exigence dure à inscrire dans E3, qui traite les deux à la fois.** Destruction de l'original
en clair après traitement, avec TTL sur les objets jamais confirmés. C'est de toute façon exigé
par la minimisation du RGPD Art. 5 : ce n'est pas un surcoût du choix de conception, c'est du
travail dû.

### D5 — Parité totale ou parité sélective ?

**Recommandation : parité sélective — E2, E3, E4 complets ; E1 et E5 au minimum utile.**

Cette question revient sur l'arbitrage rendu, et c'est délibéré : la §1 l'a notée comme
re-arbitrable après la vague 0, et la recommandation demandée porte dessus. La décision reste
au commanditaire.

Le critère qui rend l'arbitrage tranchable : **la parité est un outil de traitement d'objection
commerciale, pas un objectif produit.** Ce qui doit exister, c'est ce qu'un prospect comparera
ligne à ligne dans un appel d'offres.

| Spec | Verdict | Motif |
| --- | --- | --- |
| **E4** RAG | **Complet** | C'est là que la couche de preuve crée une valeur unique — des vecteurs rédigés. Comparé systématiquement. |
| **E2** versions/diff | **Complet** | Devient différenciant grâce au diff sur jetons (D3), pas malgré lui. |
| **E3** uploads | **Complet** | Plomberie, mais indispensable au-delà de quelques Mo, et peu coûteuse. |
| **E1** extraction/presets | **Minimal** | Aligner le contrat de `/v1/documents` sur `/v1/extract`. Les presets nommés et versionnés sont de la commodité pure : aucun appel d'offres ne s'y arrête. |
| **E5** métrage | **Minimal** | Compteurs dérivés de l'audit et quotas, rien de plus. Le besoin est commercial (facturation), pas concurrentiel. |

**Gain estimé : quatre à six semaines, sans perte de position concurrentielle** — le temps
récupéré va à la piste P, seul terrain où hacienda est aujourd'hui seule.

### Récapitulatif

| # | Décision | Recommandation | Effet sur les specs filles |
| --- | --- | --- | --- |
| D1 | E4 ou loader | **Les deux** — loader vague 1, E4 vague 4 | **Ajouter E0**, prérequis de validation de E4 |
| D2 | Tier 0 ou poids | **Tier 0**, entraînement piloté par la mesure | V3 porte la barrière ; #35 conserve sa conception, diffère ses runs |
| D3 | Diff sur jetons | **Validé** | E2 porte la résolution de rotation et la dégradation annoncée |
| D4 | Quarantaine | **Adoptée** | E3 porte la destruction de l'original et le TTL |
| D5 | Parité | **Sélective** | E1 et E5 réduits au minimum utile |

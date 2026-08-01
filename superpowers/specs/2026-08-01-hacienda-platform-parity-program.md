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

## 1. Objectif et réserve

**Objectif demandé :** que hacienda propose tout ce que fait Xberg Enterprise, **plus** la
couche de preuve, **plus** le chargement d'adaptateurs LoRA métiers.

**Réserve, énoncée une fois puis levée.** L'analyse §9.12.4 recommandait de *ne pas*
concurrencer l'amont sur la commodité (extraction, plomberie RAG) et de concentrer l'effort
sur la couche de preuve, seul terrain où hacienda est aujourd'hui seule. La décision prise
est la parité complète. Ce document l'exécute intégralement. Deux conséquences à assumer
explicitement plutôt qu'à découvrir :

- le programme devient large — quatre pistes, quinze specs — là où la couche de preuve
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
│  E1 extraction+presets · E2 documents/versions/diff · E3 uploads      │
│  E4 RAG · E5 métrage d'usage                                          │
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

### E1 — Extraction et presets

**Parité visée.** `POST /v1/extract`, `GET /v1/presets`, `/{id}`, `/{id}/sample/{name}`,
et l'équivalent Pro `/v1/saved-presets` (CRUD).

**Portée.** Aligner le contrat de `/v1/documents` sur `/v1/extract` ; presets nommés,
versionnés, partagés au niveau projet ; échantillons de sortie.

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

### E5 — Métrage d'usage

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
| **1 — semaines 2-8** | **S1**, puis **S4** | S1 est le seul chantier réellement bloquant : le rétro-ajouter après production impose une migration des chaînes d'audit. S4 rend le produit consommable. |
| **2 — semaines 6-14** | **S2**, **S3**, **P1**, **P3** | La persistance partagée et le point d'application doivent précéder tout nouveau store. P3 dépend de l'espace de clés par tenant de S1. |
| **3 — semaines 10-18** | **V3**, **V1**, **V2**, **V4** | V3 d'abord : sans réponse à l'empreinte mémoire, le routage n'a rien à router. |
| **4 — semaines 14-30** | **E1**, **E2**, **E3**, **E4**, **E5** | La parité en dernier, une fois que le garde P1 existe pour l'envelopper. E4 seulement après l'arbitrage « document loader d'abord » de sa fiche. |

**Chemin critique :** S1 → P1 → E4. Tout le reste peut avancer en parallèle.

**Point de contrôle après la vague 0.** Les trois specs P exposées suffisent à qualifier un
prospect en secteur régulé. Si elles ne suscitent pas d'intérêt commercial, la parité
Enterprise des vagues 4 est à rediscuter avant d'engager quinze semaines.

---

## 9. Ce que ce découpage ne couvre pas

| Exclusion | Raison |
| --- | --- |
| Serveur MCP | Chantier distinct : la moitié extraction est une feature `xberg/mcp` à activer (analyse §9.11.1), les outils PII/conformité se greffent dessus. À spécifier à part. |
| Intégrations de frameworks RAG | Voie parallèle et moins chère que E4 (analyse §9.11.2), à spécifier séparément. |
| Entraînement d'adaptateurs | PR #35 le couvre, en Python, hors de ce workspace. |
| Studio | Consomme V1 et P3 ; son évolution propre est hors programme. |
| SSO/SAML, console d'administration, facturation | Nécessaires à une offre SaaS, hors du périmètre demandé. |

---

## 10. Décisions à trancher avant rédaction des specs filles

1. **E4 complet ou *document loader* d'abord ?** L'alternative est nettement moins chère et
   couvre une grande part du besoin. Recommandation : loader d'abord, E4 sur demande client
   avérée.
2. **V3 : Tier 0 zéro-shot, ou poids d'emblée ?** #42 recommande Tier 0. Cela décale, voire
   annule, une partie de #35.
3. **E2 : diff sur tokens pseudonymisés — validé ?** C'est la seule résolution compatible avec
   I1, et c'est un avantage sur Enterprise. À confirmer comme parti pris.
4. **E3 : quarantaine des uploads présignés — acceptable en performance ?** Elle impose un
   aller-retour de traitement avant lisibilité.
5. **Parité totale ou parité sélective ?** E1, E2, E5 sont peu différenciants. Les livrer
   affaiblit la concentration sans créer d'avantage. À arbitrer après le point de contrôle de
   la vague 0.

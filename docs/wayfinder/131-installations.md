# Wayfinder #131 — Spécification des deux installations : Local Full et Team On-Premise

- **Issue traitée** : jamon8888/hacienda-engine#131 (label `wayfinder:research`)
- **Carte parente** : #118 — *Wayfinder — Secrecy Client Tauri : spécification de sécurité et feuille de route*
- **Issue traitée en parallèle** : #125 — *Choisir la topologie de confiance du mode local et du mode on-premise* (voir §10 pour la frontière)
- **Base inspectée** : `origin/main` @ `db8d895` (« feat(secrecy-client): LocalFull et recherche RAG hybride tokenisée (#132) »), **lecture seule**
- **Date** : 2026-11-16
- **Nature du livrable** : note de recherche/spec (aucune modification de fichier tracké, aucun commit, push, PR ou issue)
- **Rapport initial** : `docs/research/2026-11-14-wayfinder-131-xberg-lancedb-rehydration.md`

> Ce document **consolide** le rapport initial à l'état du code mergé (`db8d895`) : il fixe les
> deux installations (composants, réseau, données, clés), le rôle de Xberg/LanceDB/réhydratation
> par mode, les **écarts code actuel vs cible**, un **découpage en issues de dev**, et les
> **questions ouvertes**. Il ne remplace ni l'enquête LanceDB #130, ni la spec éditeur/viewer #129,
> ni l'audit RAG #123 — il s'y articule.

---

## 1. Décision structurante : deux produits, pas un binaire à serveur facultatif

La cible n'est pas « un client Tauri avec un serveur optionnel ». C'est **deux profils
contractuels** avec des frontières de confiance, des flux et des garanties différents. Ce
contrat existe déjà sous forme de types dans `crates/hacienda-client-core/src/lib.rs`
(`InstallationConfig::{LocalFull, TeamOnPremise}`) ; il doit devenir **la clé de voûte de
l'installation** (config, packaging, réseau, données, clés), pas un type décoratif.

Invariant commun aux deux profils (repris de #118) : séparation stricte
**texte en clair / texte pseudonymisé / coffre de réhydratation / index RAG / fournisseurs**.
Corollaires déjà encodés dans les types et leurs garde-fous :

- `LanceDbTopology::DirectClient` ⇒ `ConfigurationError::DirectClientToLanceDb` ;
- `ProviderRoute::DirectUnredacted` ⇒ `ConfigurationError::InsecureProviderRoute` ;
- LocalFull n'accepte qu'un endpoint **loopback http** (`is_loopback_http`) ; Team n'accepte que
  **https** (`validate_https`) ;
- le `Debug` de tous les types sensibles est masqué (`<redacted>`), et les variantes
  `ConfigurationError` ne portent **aucune** valeur (endpoint, tenant, chemin, clé).

---

## 2. Profil A — Local Full (autonome, mono-poste)

**Destination** : l'application autonome de référence — Tauri embarque Hacienda/Xberg, LanceDB
Embedded, un coffre local chiffré et un gateway local Claude/Codex. Utilisable **sans réseau
obligatoire**, hors egress fournisseur.

### 2.1 Composants

| Composant | Rôle | Implémentation / type | État `db8d895` |
|---|---|---|---|
| Client Tauri | UI éditeur/viewer, transport provider, IPC | `apps/hacienda-secrecy-client/src-tauri/` | Présent (LocalFull de base) |
| Hacienda Core + Xberg (in-process) | extraction, détection PII, redaction tokenisée, pseudonymisation | `HaciendaFacade::process`, `TrustedEditorRevisionIssuer` | Présent, feature `ner-candle` requise |
| Détecteur NER | GLiNER2 épinglé (poids hors bundle) | `PINNED_NER_MODEL_ID = fastino/gliner2-privacy-filter-PII-multi` @ `1cb4166…` | Installé séparément (`scripts/install_gliner2_pii.py`) |
| Embedding local | preset Xberg `multilingual` (E5, 768 d), chunk 1024/100 | `LOCAL_EMBEDDING_PRESET`, `LOCAL_EMBEDDING_COLLECTION = documents-multilingual-v1` | Activation explicite ; ingestion fail-closed sinon |
| LanceDB Embedded | index RAG token-only (dense + FTS/BM25 + hybride) | `LanceDbStore` (`crates/hacienda-rag/src/backends/lancedb.rs`) | Présent, chemin privé `<app_root>/lancedb` |
| Coffre local chiffré | originaux + map de réhydratation + clés | `EncryptedLocalVault` (marqueur) | **Non implémenté** (voir É4) |
| Gateway local provider | egress Claude/Codex redacté | `GatewayProviderTransport` (client) | Transport présent, **serveur absent** (voir É2) |
| Audit local | journal sans contenu (`span_hash` blake3) | `AuditStore`, `content_free_audit_input` | Présent |
| Viewer local | révélation explicite, fenêtre isolée, TTL 60 s | `LocalViewerState`, `ViewerCapability`, `NativeOriginalViewer` | Présent (token seul ; viewers multi-format non livrés) |

### 2.2 Réseau

- **Aucun serveur Hacienda/Xberg externe requis** : le profil Tauri `local_full_profile()`
  instancie les endpoints `hacienda`, `xberg`, `gateway` en `http://127.0.0.1:0` (boucle locale,
  in-process). La validation `validate_local_endpoint` accepte `http` loopback.
- **Egress fournisseur** : uniquement via le gateway local ; le transport
  `ProviderTransportConfig` n'accepte que **loopback http** ou **https** (jamais un `http://`
  distant). L'envoi est **désactivé** tant que `local-pii-status` ne rapporte pas
  `provider_send_enabled` (détecteur local complet **et** gateway configuré).
- **Synchronisation équipe** : désactivée par défaut ; explicitement opt-in (non implémentée).
- **Rété d'infrastructure IP** : le design ORT impose un choix CPU explicite sur macOS Intel
  (CoreML non viable) — voir É8.

### 2.3 Données

- **Originaux** : lus depuis le chemin utilisateur à l'ingestion (`read_selected_document`) et
  **référencés** par une liaison en mémoire de session (`StoredLocalDocument { original_path, binding }`).
- **Projections token-only** : `Backend::project(&redacted, "local", …)` produit une enveloppe
  éditeur Tiptap token-only (dossier logique `"local"`), persistée dans LanceDB via
  `persist_to_with_embedder` / `append_editor_revision_with_embedder`.
- **Pseudonymes** : tokens Hacienda réversibles `[CATEGORY:key_id:ciphertext]` (AES-256-SIV),
  stables **par profil local** (tenant `default_tenant()` en LocalFull).
- **Cache modèles/embeddings** : `<app_root>/models/xberg-embeddings` ; GLiNER sous données d'app
  `models/gliner2-pii` ; runtime ONNX empaqueté sous `resources/onnxruntime/**`.

### 2.4 Clés

| Secret | Emplacement | Détail |
|---|---|---|
| Clé de pseudonymisation (32 octets) | **Trousseau OS** | `OsCredentialKeyStore`, service `org.hacienda.secrecy-client`, user `local-full-pseudonym-key-v1` ; générée au premier lancement, en clair uniquement en mémoire (`Zeroizing`) |
| Référence de coffre | `EncryptedLocalVault.key_reference` | `os-keychain:local-full` — **marqueur**, pas d'implémentation cryptographique |
| Credential gateway/provider | Environnement | `HACIENDA_GATEWAY_TOKEN` ou `HACIENDA_PROVIDER_CREDENTIAL` |
| Configuration d'installation | Environnement | `HACIENDA_INSTALLATION_CONFIG` (JSON `InstallationConfig` validé) |
| Modèle NER (optionnel) | Environnement | `HACIENDA_NER_MODEL_DIR` (sinon données d'app) |

---

## 3. Profil B — Team On-Premise (multi-tenant, équipe)

**Destination** : l'installation d'équipe de référence — Tauri est un **client léger** ;
Hacienda Gateway/Engine on-premise exécute Xberg, résout le tenant, applique les ACL et écrit
l'audit ; LanceDB est le **data plane RAG token-only** derrière `RagStore` ; Postgres/object
store portent le control plane ; la réhydratation est **server-side, auditée et sous capacité**.

### 3.1 Composants

| Composant | Rôle | Type / ancre | État `db8d895` |
|---|---|---|---|
| Client Tauri léger | UI, viewer, session | `ClientSurface::LightTauri` | Types présents ; pas de build « light » |
| Hacienda Gateway (tenant-scoped) | authn/authz, redaction, egress, réhydratation | `GatewayConfig::AuthenticatedTenantScoped` | **Non implémenté** (É2, É5) |
| Hacienda Engine on-premise | extraction/redaction/audit serveur | `HaciendaFacade` + `hacienda serve` | API présente (`hacienda-api`) |
| LanceDB data plane | index token-only serveur | `LanceDbStore` derrière `RagStore` | Adaptateur présent ; déploiement serveur non câblé |
| Postgres control plane | tenants, users, API keys, versions, jobs, audit/review, ACL | migrations `hacienda-core`, `hacienda-rag` (`0100/0101/0102`) ; `PgVectorStore` | Présent en CI (`ci-postgres.yaml`), **absent du compose** |
| Object store privé | originaux + pièces jointes chiffrés | — | **Non implémenté** |
| Gateway LLM on-premise | contexte token-only uniquement | allowlist egress (`with_rag_egress_allowlist`) | Partiel (allowlist API) |

### 3.2 Réseau

- **HTTPS obligatoire** pour le gateway et l'engine (`validate_https`) ; réseau privé interne,
  mTLS possible.
- **Aucun accès direct client → LanceDB** : `LanceDbTopology::ServerTokenOnly` ; l'accès store
  passe par le gateway qui résout le tenant. Un client qui tente un accès direct est rejeté par
  la validation de profil (`DirectClientToLanceDb`).
- **Middleware de confiance** : le gateway est la seule route fournisseur ; `engine` et `gateway`
  sont des endpoints distincts dans le type (`GatewayConfig::AuthenticatedTenantScoped`).
- **Egress LLM** : allowlist explicite côté API ; le gate de redaction obligatoire est dans
  `rag_stream.rs` (`GuardedLlm`) — la rédaction de `system_prompt` reste un écart ouvert (É7).

### 3.3 Données

- **Tenant** : `TenantScope` obligatoire au niveau profil, **répété** dans le gateway et dans
  `LanceDbTopology::ServerTokenOnly` ; les trois doivent coïncider (sinon
  `InvalidTrustBoundary`). Le tenant n'est **jamais** pris dans le payload client.
- **Originaux** : object store privé chiffré, séparé des index vectoriels/FTS.
- **Index** : table LanceDB token-only ; `dossier_id`/`version_ref` présents dans le schéma
  actuel, mais **pas de `tenant_id` ni de `acl_reference`** (É5).
- **Audit** : journal Hacienda indépendant des versions LanceDB ; entrées `Reveal` digest-only.
- **Isolation de collection** : `hacienda-api` préfixe le nom par le tenant
  (`{tenant}:{name}`, cf. `scope_collection_name`) ; le tenant `default` reste non préfixé
  (compat).

### 3.4 Clés

| Secret | Type / source | Détail |
|---|---|---|
| Authentification client | `SecretReference` (`oidc:client-session`) | Résolue en variable d'env `HACIENDA_CREDENTIAL_<NORMALISÉ>` |
| Clés de pseudonymisation | `KeyResolver` **serveur, tenant-scoped** | `EnvKeyResolver` aujourd'hui ; KMS cible (#16) |
| Accès LanceDB | `SecretReference` (`control-plane:lancedb`) | Token opaquet, jamais un chemin/payload |
| Chiffrement originaux | object store | non défini |
| Secret de session viewer | capacité courte durée | non défini (É6) |

---

## 4. Matrice d'installation consolidée

| Fonction | Local Full | Team On-Premise |
|---|---|---|
| Client | `LocalTauri` (surface complète) | `LightTauri` (session/viewer) |
| Frontière de confiance | `LocalProcess` | `AuthenticatedTenantGateway` |
| Extraction Xberg | in-process Tauri | Hacienda Engine serveur |
| Redaction | locale, fail-closed | serveur, tenant-scoped |
| LanceDB | `LocalEmbedded` (chemin privé) | `ServerTokenOnly` (derrière gateway) |
| Control plane | fichiers/audit local | Postgres multi-tenant + audit/review |
| Originaux | coffre local chiffré (cible) | object store privé chiffré |
| Pseudonymes | stables par profil local | stables par tenant |
| Map Xberg | optionnelle, document-local | à éviter ; préférer Hacienda |
| Réhydratation | viewer Tauri local + capacité coffre | capacité serveur auditée |
| Claude/Codex | gateway local | gateway on-premise |
| Réseau obligatoire | non (hors egress fournisseur) | oui (interne) |
| Synchronisation | opt-in, chiffrée, conflict-aware | serveur autoritaire |
| Config d'installation | `HACIENDA_INSTALLATION_CONFIG` (env JSON) | idem (env JSON) |

---

## 5. Rôle de Xberg / LanceDB / réhydratation par mode

### 5.1 Xberg

- **Local Full** : extraction + redaction **dans le processus Tauri** ; embeddings Xberg locaux
  (`preset multilingual`, E5/768) calculés **sur le texte tokenisé uniquement** ; GLiNER2/Candle
  **avant** LanceDB (enrichit provenance/tokens, n'est pas l'embedder).
- **Team On-Premise** : mêmes opérations **côté serveur**, tenant-scoped, avant tout stockage.
- Xberg fournit aussi `redact_capturing_rehydration_map` (`TokenReplace` + `encrypt_map`/
  `decrypt_map`, feature `redaction-rehydrate`) — **non exposé** par les bindings Python/TS.
  Cette API est l'option **document-local** (voir §5.3), pas le défaut.

### 5.2 LanceDB

- **Data plane RAG token-only uniquement**, jamais le coffre, l'audit, l'ACL métier ou le
  contrôle d'egress. Verdict #130 : bon moteur, **pas** un backend complet de confidentialité.
- **Local Full** : Embedded, chemin privé, table « documents + chunks », FTS/BM25 + dense +
  fusion RRF (`RRF_K = 60.0`).
- **Team On-Premise** : remote/shared possible **derrière** `RagStore`, avec Postgres/Hacienda
  comme control plane ; l'autorisation catalogue/de table LanceDB **ne remplace pas** l'ACL
  tenant→dossier→version→chunk.
- **Modes** : `vector`, `full_text`, `hybrid` supportés ; `sparse`/`late_interaction` à renvoyer
  `UnsupportedMode` tant qu'un adaptateur + tests ne les prouvent pas.
- `RagStore` est **single-tenant par conception** : la multi-tenancy est portée par l'appelant
  (instance scopée par tenant / décorateur RLS), jamais par la signature.

### 5.3 Réhydratation : deux contrats à ne pas mélanger

| Contrat | Portée | Stabilité | Usage cible |
|---|---|---|---|
| **A — Hacienda stable** | tenant | déterministe, réversible, cross-document | **défaut** pour le RAG multi-document |
| **B — Xberg `RehydrationMap`** | document | local, non corrélé | option document-local explicite, versionnée (`rehydration_scheme=xberg_map_v1`) |

- **Local Full** : réhydratation **viewer Tauri local** via `RehydrationCapability::LocalEncryptedVault` ;
  `VaultRehydrationAdapter::reveal` rend un `EphemeralPlaintext` (Zeroizing, sans `Serialize`/
  `Debug`/`Display`) + `AuditReference`. Le `tenant_scope` est **interdit** dans ce profil.
- **Team On-Premise** : réhydratation **serveur** via `RehydrationCapability::AuditedServer`
  (`audit` non vide exigé) ; le client demande une capacité courte durée, le serveur vérifie
  l'ACL et renvoie un flux viewer limité — **jamais** vers Claude/Codex.
- **Interdits** (les deux modes) : réhydratation dans la requête LLM ; outil MCP `pii_reveal`
  disponible au modèle en production ; valeur claire dans transcript/citations/logs/snapshots ;
  réhydratation automatique d'une réponse entière sans sélection humaine ; map Xberg servant à
  révéler un token Hacienda (ou l'inverse).

---

## 6. Contrat de connectique (ingestion → recherche → réhydratation)

```text
bytes + mime + document_id
        │  (LocalFull: in-process Tauri ; Team: Hacienda Engine serveur)
        ▼
HaciendaFacade / Xberg
  1. extraction contrôlée (ExtractInput::from_bytes)
  2. NER + regex + findings Tiptap manuels
  3. redaction de TOUTES les surfaces textuelles
  4. audit Hacienda (sans contenu)
        │
        ├── originaux ───────────► coffre chiffré / object store
        ├── audit + version ─────► control plane (local / Postgres)
        └── projection token-only ► chunking → embedding → LanceDB
```

- **Ingestion** : `Backend::project` construit l'enveloppe éditeur + `DocumentRecord`/`ChunkRecord`
  token-only ; `persist_to(_with_embedder)` écrit la collection (`documents-multilingual-v1`).
  Le chunker travaille **après** redaction, jamais sur l'`ExtractedDocument` original.
- **Recherche (LocalFull)** : la question est redactée localement (`redact_local_text` → exige
  `pii_enabled` et **un** pipeline), seul le texte tokenisé alimente l'embedder et le retrieval
  `RetrieveMode::Hybrid` (top_k 8, `candidate_multiplier 4`, `group_by_document`). Les titres/
  extraits relus depuis LanceDB sont **re-redactés avant IPC**.
- **Recherche (Team)** : `/v1/rag/collections/{name}/…` + `/answer` ; `scope_collection_name`
  préfixe par tenant ; `include_content=true` forcé par `answer`, puis `GuardedLlm` rédige
  prompt et chunks avant `answer_stream`.
- **Réhydratation** : requête `{ document_ids, tenant_scope, capability }` validée par
  `InstallationConfig::validate_rehydration` **avant** toute révélation ; la réponse ne représente
  **jamais** de clair (`RehydrationResponse` = tokenisé + `OpaqueTokenReference`).

---

## 7. Écarts code actuel vs cible

Classés par impact décroissant. Ancrage par fichier/symbole.

- **É1 — Le profil d'installation n'entre pas dans la config serveur/CLI.**
  `hacienda-core/src/config.rs::HaciendaConfig` n'a que `extraction`, `pii`, `compliance`,
  `review`, `glossary`, `auth` (`deny_unknown_fields`). Aucune section `installation`, `vault`,
  `lancedb`, `gateway` ou `tenant`. Le profil n'existe aujourd'hui que côté client Tauri via
  `HACIENDA_INSTALLATION_CONFIG` (JSON). **Cible** : une configuration d'installation de premier
  rang, partagée client/serveur, résolue et validée par une seule source de vérité.

- **É2 — Le gateway redactant n'existe pas comme composant serveur.**
  `hacienda-client-core/src/provider.rs` fournit le **client** `GatewayProviderTransport`
  (loopback/https, SSE validé) et l'allowlist d'egress API, mais aucun serveur ne sert
  `/v1/messages` (Anthropic) ni `/v1/chat/completions` (OpenAI) en **redactant**. En LocalFull,
  le « gateway » est donc soit le fournisseur lui-même (non redactant), soit fail-closed.
  **Cible** : un gateway local/on-premise qui applique la redaction structurelle avant egress
  (décision #121).

- **É3 — Schéma LanceDB générique, pas token-only par construction.**
  `crates/hacienda-rag/src/backends/lancedb.rs` déclare `document_schema`/`chunk_schema` avec des
  colonnes **neutres** : `title`, `full_text`, `content`, `keywords`, `entities`, `metadata`,
  `chunk_metadata`… La spec #129 promet des colonnes `*_token_only` (`title_token_only`,
  `search_text`, `authors_token_only`, `acl_reference`, `redaction_policy_version`,
  `editor_schema_version`, `index_state`…). Aujourd'hui la propriété « token-only » est une
  **convention de l'appelant** (`Backend::project`), pas une garantie du store : `upsert_document`
  accepte n'importe quelle chaîne. **Cible** : schéma explicite token-only + garde-fou de
  non-egress testable au niveau du store.

- **É4 — Coffre local durable absent ; liaison originaux en mémoire de session.**
  `LocalFullDocumentService.documents: HashMap` perd `StoredLocalDocument { binding, original_path }`
  au redémarrage (documenté dans les addenda #129/#131). `EncryptedLocalVault` est un **marqueur**
  (`path = "<native-vault>"`), sans implémentation de coffre chiffré : les originaux restent des
  fichiers utilisateur, non chiffrés par le client. **Cible** : coffre chiffré durable +
  restauration des références (document/version/viewer) après redémarrage, clé côté trousseau OS.

- **É5 — Isolation multi-tenant et ACL document/chunk incomplètes côté data plane.**
  `RagStore` est single-tenant ; `LanceDbStore` porte `dossier_id`/`version_ref` mais **pas de
  `tenant_id`** ni d'`acl_reference`. L'isolation repose sur le préfixe de **nom de collection**
  dans `hacienda-api::scope_collection_name` (et le tenant `default` non préfixé). Les filtres
  (`filter.rs`, allowlist `doc.*`/`chunk.*`) sont des contraintes de **recherche**, pas une ACL.
  **Cible** : tenant effectif + ACL document/chunk au data plane, préfiltre d'autorisation
  **avant** scoring + contrôle de second niveau après retrieval.

- **É6 — Réhydratation : autorisation trop large, pas de TTL/justification, MCP non gardé.**
  `pii:reveal` autorise **tous** les jetons du tenant (pas par document/champ) ; ni la requête
  REST, ni `RehydrationRequest`, ni l'entrée d'audit ne portent de **motif** ou de **TTL** ; le
  `principal` est `None` dans `VaultRehydrationAdapter::reveal` et dans le MCP `pii_reveal`
  (`Caller::Trusted`, aucun cap vérifié). `DisplayCapability` est défini mais **non câblé**.
  **Cible** : contrat A/B unifié, capacité par document, expiration courte, usage unique,
  justification, audit nommant le principal, `pii_reveal` fermé côté agent.

- **É7 — `system_prompt` non redacté vers le fournisseur.**
  `GuardedLlm` rédige `prompt` et `chunk.content` mais pas `LlmAnswerConfig.system_prompt`, qui
  part verbatim (`stream.rs`). Constat toujours ouvert (addendum #123 du 2026-11-15). **Cible** :
  rédaction du system prompt ou suppression de l'override non redacté.

- **É8 — Packaging et runtime modèles/embeddings non fournis par plateforme.**
  `docker/Dockerfile` ne livre ni ONNX Runtime ni modèle ; le staging ORT vit dans le script
  Tauri (`prepare_onnxruntime.mjs/.py`, archive épinglée **1.23.2**, `resources/onnxruntime/**`).
  macOS Intel n'a **pas** d'ORT utilisable en Homebrew (forcer `Cpu`, CoreML trop lent). Le paquet
  Tauri distribué n'est pas encore validé ; l'embedding est **opt-in** (téléchargement à
  l'activation) et l'ingestion échoue fermé avant. **Cible** : runtime par plateforme + validation
  macOS Intel, Apple Silicon et Linux, avec chemin de chargement documenté.

- **É9 — Déploiement on-premise non reproductible depuis le dépôt.**
  `docker-compose.yml` ne déclare que `hacienda` (distroless, bind `0.0.0.0:8787`) + prometheus/
  grafana/alertmanager : **aucun Postgres, LanceDB, object store, ni split gateway/engine**. La
  CI Postgres existe (`ci-postgres.yaml`) mais l'installation d'équipe n'est pas packagée.
  `config/production.toml` n'a qu'`[auth]` avec un **token statique en mémoire** (pas de tenant,
  pas d'OIDC). **Cible** : un compose/helm d'équipe (Postgres + LanceDB + gateway + object store)
  avec auth tenant-scoped et secrets montés.

- **É10 — Divergences documentaires à corriger (non bloquantes).**
  La recherche #131 cite `../../../hacienda/hacienda-core/src/xberg_surface.rs` ; la surface vit
  réellement sous `hacienda-core/src/xberg_surface.rs`. Les documents de recherche décrivent parfois
  le schéma LanceDB token-only comme acquis alors qu'il est conventionnel (cf. É3).

> Rappel des corrections **déjà mergées** sur `main` (`db8d895`) et à ne pas re-signaler comme
> ouvertes : le transport du tenant depuis l'auth et la validation des noms de collection
> (`7ba6851`), et la redaction des sorties de retrieval avant sérialisation (`9970b93`) — cf.
> addendum #123. Le constat `system_prompt` (É7) reste, lui, ouvert.

---

## 8. Découpage en issues de dev

Chaque issue cite son **ancre code** (règle de gouvernance DEV_PLAN §4) et se rattache à #118.
`L` = Local Full, `T` = Team On-Premise, `L/T` = commun.

| # | Titre proposé | Portée | Ancre | Dépend de |
|---|---|---|---|---|
| **I1** | *Config d'installation de premier rang (`[installation]`)* | Section de config partagée client/serveur, résolution + validation unique, exposition API/CLI, refus fail-closed | `hacienda-core/src/config.rs`, `crates/hacienda-client-core/src/lib.rs` | #125 |
| **I2** | *Gateway redactant Anthropic/OpenAI (local + on-prem)* | Serveur `/v1/messages` + `/v1/chat/completions`, SSE/headers compatibles, redaction structurelle fail-closed, allowlist egress | `crates/hacienda-client-core/src/provider.rs`, `hacienda-api` | I1, #121 |
| **I3** | *Durcissement `LanceDbStore` token-only* | Schéma explicite `*_token_only`, garde-fou non-egress, `tenant_id`/`acl_reference`, `dossier_id`/`version_id`, tests de non-fuite des index/FTS/embeddings | `crates/hacienda-rag/src/backends/lancedb.rs` | #130 |
| **I4** | *Coffre local chiffré durable* | Remplacement du `HashMap` de session, persistance chiffrée des originaux + map de réhydratation, restauration des références après redémarrage, clé trousseau OS | `apps/hacienda-secrecy-client/src-tauri/src/main.rs` | I1, #129 |
| **I5** | *ACL document/chunk + tenant effectif au data plane* | `tenant_id`/ACL par ligne, préfiltre d'autorisation avant scoring, contrôle post-retrieval, tests négatifs (tenant voisin, version supprimée, chunk retiré) | `crates/hacienda-rag/src/{store,filter}.rs`, `hacienda-api/src/handlers/rag.rs` | I1, #123 |
| **I6** | *Unification du contrat de réhydratation* | Fusion contrat A/B, capacité par document, TTL/usage unique/justification, `principal` dans l'audit, fermeture MCP `pii_reveal`, câblage `DisplayCapability` | `crates/hacienda-backend/src/lib.rs`, `hacienda-core/src/facade.rs`, `hacienda-mcp/src/lib.rs` | #122 |
| **I7** | *Rédaction du `system_prompt` + audit « contenu envoyé au LLM »* | Corriger la fuite É7, ajouter l'audit P6 différé | `crates/hacienda-rag/src/stream.rs`, `hacienda-api/src/handlers/rag_stream.rs` | #123 |
| **I8** | *Control plane on-premise packagé* | Compose/helm Team (Postgres + LanceDB + gateway + object store), auth tenant-scoped (OIDC/mtls), secrets montés, `AuthenticatedTenantGateway` serveur | `docker-compose.yml`, `docker/`, `config/`, `hacienda-api` | I1, I2, I5 |
| **I9** | *Packaging runtime modèles/embeddings par plateforme* | ORT par OS/arch + modèle épinglé, chemin de chargement, validation macOS Intel / Apple Silicon / Linux, échec fermé | `apps/hacienda-secrecy-client/scripts/prepare_onnxruntime.*`, `scripts/ci/actions/setup-onnx-runtime/`, `docker/` | #131 (validation) |
| **I10** | *Viewers multi-format intégrés* | PDF/DOCX/XLSX/PPTX/EML/images, fallback Markdown tokenisé, ouverture originale auditée | `apps/hacienda-secrecy-client/` | #129 |
| **I11** | *Migration Local Full ↔ Team On-Premise* | Export/import de versions token-only, re-keying, re-embedding, réconciliation, absence de PII brute dans les artefacts | `crates/hacienda-client-core`, `crates/hacienda-backend` | I3, I4, I8 |

**Critère de sortie transverse** (commun à I1–I11) : un test de non-egress qui prouve l'absence
de valeur brute dans LanceDB, ses index FTS, ses embeddings, les snapshots, traces et erreurs.

---

## 9. Relation avec #125 (topologie de confiance) — pour ne pas dupliquer

- **#131 (ce document)** : inventaire **concret** des deux installations — composants, réseau,
  données, clés, et connectique Xberg/LanceDB/réhydratation.
- **#125** : **où placer la frontière de confiance** et comment l'invariant de confidentialité
  tient dans les deux modes, y compris les chemins de panne (serveur indisponible, clé
  indisponible, politique absente, mise à jour non vérifiée, poste compromis).
- **Dépendance** : #125 est **blocked-by** de #123/#124/#126 (trancher la topologie après ces
  constats). #131 fournit le matériel d'installation que #125 doit protéger ; il ne re-décide pas
  la topologie. Toute décision de frontière de #125 (ex. gateway local vs interception) doit se
  refléter dans `InstallationConfig` via I1/I2.

---

## 10. Questions ouvertes

1. **I1 — Config unifiée** : `HaciendaConfig` accueille-t-il une section `[installation]` (et
   comment *TeamOnPremise* est-il représenté côté serveur, où le tenant est résolu à l'exécution) ?
   Ou le profil reste-t-il une entrée client séparée, avec un contrat de correspondance explicite ?

2. **I2 — Forme du gateway** : reverse-proxy local indépendant, sidecar Tauri, ou middleware dans
   `hacienda serve` ? Quel niveau d'interception (compatible Anthropic/OpenAI vs TLS) est retenu
   pour Claude/ChatGPT Desktop (décision conjointe #121/#124/#133) ?

3. **I3 — Schéma LanceDB** : conserver `LanceDbStore` single-tenant derrière une instance scopée,
   ou introduire une colonne `tenant_id` + ACL au store ? Preuve de parité avec `PgVectorStore`
   (upsert idempotent, filtres, suppression, crash/reprise) ?

4. **I3/I5 — Sparse/late-interaction** : rester `UnsupportedMode` tant qu'un adaptateur n'existe
   pas, ou spécifier un adaptateur SPLADE/ColBERT séparé ? Impact sur `Capabilities` annoncées.

5. **I4 — Stockage des originaux** : coffre fichier chiffré local, table/blob LanceDB **non
   searchable**, ou object store dès le local ? #129 laisse le choix ouvert entre table/blob
   chiffré et stockage objet.

6. **I6 — Contrat de réhydratation** : un seul contrat paramétré (tenant vs document) ou deux
   contrats A/B conservés mais pontés ? Où vit la map Xberg document-local (coffre, table
   séparée) et comment empêcher son usage cross-document ?

7. **I6 — Capacité viewer** : TTL, usage unique, double approbation haute sensibilité (#120) —
   quels seuils, et comment les vérifier sans oracle d'erreur ?

8. **I6/I18 — Clés** : trousseau OS en local, `EnvKeyResolver` ou KMS (#16) en équipe ? Rotation
   additive suffit-elle, ou faut-il une révocation d'urgence (retirer une clé) ?

9. **I8 — Postgres** : `PgVectorStore` reste-t-il le backend d'équipe (repli mature) ou LanceDB
   remote/Enterprise le remplace-t-il après validation commerciale on-premise (support, régions,
   suppression, licence) ?

10. **I9 — Runtime ORT** : distribution par plateforme, licence et chemin de chargement ; macOS
    Intel doit-il rester CPU-only, et comment valider le **paquet Tauri distribué** (et pas
    seulement un smoke isolé) sur les trois cibles ?

11. **I9/#131 — Validation features** : quel niveau de preuve pour les embeddings Xberg et
    GLiNER2 (le seuil interne Xberg à 0,5 tronque le rappel) sur macOS Intel, Apple Silicon et
    Linux, avant de considérer l'installation « validée » ?

12. **I10 — Formats/viewers** : premier lot de formats, bibliothèques de rendu autorisées, et
    politique d'ouverture externe par OS.

13. **I11 — Migration et rétention** : que migre-t-on exactement entre profils, et quelle
    politique de conservation/purge couvre snapshots, backups, object store et audit (#129) ?

---

## 11. Sources inspectées (lecture seule, `origin/main` @ `db8d895`)

### Types et contrats

- `crates/hacienda-client-core/src/lib.rs` — `InstallationConfig`, `LocalFullConfig`,
  `TeamOnPremiseConfig`, `LanceDbTopology`, `GatewayConfig`, `RehydrationCapability/Request`,
  `ConfigurationError` (validation + `Debug` masqué).
- `crates/hacienda-client-core/src/provider.rs` — `ProviderTransport`, `GatewayProviderTransport`,
  transport Anthropic/OpenAI, validation SSE, `RehydrationAdapter`, `DisplayCapability`,
  `SecrecyClient`.
- `crates/hacienda-backend/src/lib.rs` — `Backend`, `CollectionScope`, `project`, `persist_to(_with_embedder)`,
  `append_editor_revision(_with_embedder)`, `rehydrate`, `VaultRehydrationAdapter::reveal`,
  `AuthenticatedTenantGateway`, `connect_configured_lancedb`/`connect_lancedb`.
- `crates/hacienda-rag/src/store.rs` — `RagStore` (single-tenant) ; `query.rs` — `RetrieveMode` ;
  `filter.rs` — allowlist `doc.*`/`chunk.*`.
- `crates/hacienda-rag/src/backends/lancedb.rs` — `LanceDbStore`, schémas `metadata`/`document`/
  `chunk`, `RRF_K = 60.0`.

### Serveur / API / config

- `hacienda-core/src/config.rs` — `HaciendaConfig` (`deny_unknown_fields`), sections réelles.
- `hacienda-api/src/handlers/rag.rs` — `scope_collection_name`, `strip_tenant_prefix`,
  `/v1/rag/*` ; `routes.rs` — routes RAG/`answer`.
- `docs/configuration.md`, `config/production.toml` — surface de config et exemple de prod.
- `hacienda-core/migrations/*`, `crates/hacienda-rag/migrations/0100/0101/0102` — control plane.

### Client Tauri / packaging / CI

- `apps/hacienda-secrecy-client/src-tauri/src/main.rs` — `local_full_profile`,
  `LocalFullDocumentService` (ingest/load/save/search), `HaciendaLocalRedactor`, `LocalViewerState`,
  `NativeOriginalViewer`, `ProviderCommandState`, env (`HACIENDA_INSTALLATION_CONFIG`,
  `HACIENDA_GATEWAY_TOKEN`, `HACIENDA_PROVIDER_CREDENTIAL`, `HACIENDA_NER_MODEL_DIR`).
- `apps/hacienda-secrecy-client/README.md`, `src-tauri/tauri.conf.json`, `capabilities/local-viewer.json`.
- `docker/Dockerfile`, `docker-compose.yml`, `scripts/install.sh`,
  `apps/hacienda-secrecy-client/scripts/prepare_onnxruntime.{mjs,py}`,
  `scripts/ci/actions/setup-onnx-runtime/{linux,macos}.sh`.
- `.github/workflows/ci-postgres.yaml`, `ci-docker.yaml`, `ci-rust.yaml`.

### Recherche / specs liées

- `docs/research/2026-11-14-wayfinder-131-xberg-lancedb-rehydration.md`
- `docs/research/2026-11-14-wayfinder-130-lancedb-full-rag.md`
- `docs/research/wayfinder-123-rag-security.md`
- `docs/superpowers/specs/2026-11-14-wayfinder-129-editor-viewer-lancedb-contract.md`
- `docs/superpowers/specs/2026-11-14-wayfinder-123-rag-hybrid-security-audit.md`

---

*Références : issue #131 (question et exigences), carte #118 (séparation stricte
clair/pseudonymisé/coffre/RAG/fournisseurs), issues sœurs #122/#123/#124/#126/#129/#130,
issue parallèle #125, décisions « Decisions so far » de #118.*

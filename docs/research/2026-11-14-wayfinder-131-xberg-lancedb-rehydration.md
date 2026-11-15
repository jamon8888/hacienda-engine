# Connectique Hacienda/Xberg ↔ LanceDB et réhydratation — Wayfinder #131

**Date :** 2026-11-14<br>
**Statut :** architecture validée pour deux profils d’installation<br>
**Périmètre :** client Tauri, Hacienda Engine, Xberg, LanceDB, coffre de réhydratation,
Claude/Codex et déploiement on-premise multi-tenant.

## Décision structurante : deux produits d’installation

La cible n’est pas un seul binaire avec un serveur facultatif. Elle comporte deux profils
contractuels, avec des garanties et des flux différents.

### Profil A — Local Full

```text
Tauri desktop
  ├── Hacienda Core + Xberg local
  │     ├── extraction des formats
  │     ├── détection PII / findings manuels
  │     ├── redaction tokenisée
  │     └── embeddings Xberg locaux
  ├── LanceDB Embedded local
  │     └── index RAG token-only
  ├── coffre local chiffré
  │     ├── originaux
  │     ├── clés/token map
  │     └── audit et versions
  └── gateway local Claude/Codex
        └── aucune PII vers le fournisseur
```

Caractéristiques :

- fonctionnement sans Hacienda Server obligatoire ;
- les octets originaux peuvent rester sur le poste ;
- Xberg et la redaction sont exécutés dans le processus Rust local ou dans un worker local
  contrôlé par Tauri ;
- LanceDB est ouvert sur un chemin privé de l’application, jamais sur le dossier de travail
  général de l’utilisateur ;
- la clé de pseudonymisation et la clé du coffre viennent du trousseau OS ou d’un secret
  local protégé, jamais d’une colonne LanceDB ;
- synchronisation vers une équipe désactivée par défaut et explicitement opt-in.

Le mode local doit rester utilisable avec l’embedding Xberg compilé localement. Le dépôt
Hacienda possède déjà un chemin `xberg::embed_texts_async` et un warm-up des presets, mais
les embeddings réels sont feature-gated (`rag-embeddings`) et les binaires ONNX doivent être
validés par plateforme, notamment sur Intel macOS.

### Profil B — Team On-Premise

```text
Postes Tauri légers
  └── gateway local de session / UI / viewer
        │ HTTPS/mTLS ou réseau privé
        ▼
Hacienda Gateway + Engine on-premise
  ├── authn/authz + tenant context
  ├── HaciendaFacade + Xberg
  │     ├── extraction contrôlée
  │     ├── redaction et audit
  │     └── embeddings locaux du serveur
  ├── LanceDbStore derrière RagStore
  │     └── LanceDB shared/remote, token-only
  ├── Postgres control plane
  │     ├── tenants, utilisateurs, API keys
  │     ├── versions, jobs, audit/review
  │     └── ACL et politiques
  ├── object store privé chiffré
  │     └── originaux et pièces jointes
  └── gateway LLM allowlisté
        └── contexte token-only uniquement
```

Caractéristiques :

- le serveur on-premise est la frontière de confiance de l’organisation ;
- chaque requête arrive avec un tenant résolu par Hacienda, jamais choisi dans le payload ;
- les clients ne se connectent pas directement à LanceDB ;
- LanceDB sert de data plane de recherche, tandis que Postgres et Hacienda restent le control
  plane multi-tenant, audit, jobs, versions et politiques ;
- les originaux sont dans un object store privé ou un coffre de fichiers chiffré, séparé des
  index vectoriels/FTS ;
- la réhydratation passe par une capacité serveur autorisée, avec audit et éventuellement
  double approbation pour les dossiers haute sensibilité ;
- le gateway Claude/Codex est le seul chemin fournisseur.

LanceDB remote/Enterprise peut être évalué comme data plane, mais il ne doit pas être
présenté comme l’implémentation complète de la sécurité multi-tenant Hacienda. La documentation
Rust expose des ACL de déploiement/catalogue, pas le modèle métier complet tenant → dossier →
version → chunk → capacité de révélation.

## Connectique Hacienda/Xberg → LanceDB

### Pipeline d’ingestion commun

```text
bytes + mime + document_id
        │
        ▼
HaciendaFacade / Xberg
  1. extraction contrôlée
  2. NER, regex et findings Tiptap manuels
  3. redaction complète de toutes les surfaces textuelles
  4. audit Hacienda
        │
        ├── original bytes → coffre chiffré / object store
        ├── audit + version → control plane
        └── contenu token-only → chunking → embedding → LanceDB
```

Dans le dépôt, le chemin API existant est déjà proche de cette séparation :

- `POST /v1/documents` n’accepte que des bytes encodés et construit `ExtractInput::from_bytes` ;
- Hacienda expose les opérations Xberg sûres de formats, MIME et cache sans monter les serveurs
  Xberg natifs ;
- `POST /v1/pii/redact`, `POST /v1/pii/token` et `POST /v1/pii/reveal` passent par
  `HaciendaFacade` ;
- `/v1/rag/*` reçoit un `Arc<dyn RagStore>` et applique aujourd’hui un scoping de collection
  par tenant au niveau API ;
- le chemin `migrate-embeddings` réutilise `xberg::embed_texts_async` sur le contenu des chunks.

Le futur `LanceDbStore` doit donc être branché dans `ApiState::with_rag_store`, sans ajouter
un accès Xberg ou LanceDB au client Claude/Codex et sans contourner la facade.

### Schéma LanceDB token-only

Clé logique :

```text
tenant_id / dossier_id / document_id / version_id / chunk_id
```

Colonnes autorisées :

```text
content_token_only
search_text_token_only
embedding
document_id / version_id / chunk_id
classification_non_secret
acl_reference_opaque
source_hash
embedding_source / embedding_version
audit_event_id
```

Colonnes interdites dans la table searchable :

```text
original_text
original_bytes
token -> plaintext
clé de pseudonymisation
secret viewer capability
```

Les surfaces documentaires suivantes sont également sensibles et doivent être redacted avant
toute projection LanceDB :

```text
title_token_only
filename_token_only
authors_token_only
subject_token_only
keywords_token_only
uri_label_token_only
attachment_names_token_only
pdf_office_metadata_token_only
structured_fields_token_only
```

Un titre ou un nom de fichier ne doit jamais être copié tel quel dans LanceDB, même si le corps
du document a été redacted. Le `viewer_ref` et le nom physique du fichier restent opaques ;
l’interface peut afficher une projection réhydratée uniquement après contrôle de capacité.

LanceDB supporte les colonnes blob et les versions de table, mais ces primitives ne changent
pas la règle de sécurité. Si un blob chiffré est conservé avec LanceDB, il doit être séparé
de la table recherchable, ne pas être FTS/vectorisé, et être récupérable seulement via une
capacité Hacienda vérifiée.

## Réhydratation : deux contrats, à ne pas mélanger

### Contrat Hacienda : pseudonymes stables inter-documents

Le contrat actuel Hacienda est le choix par défaut pour le RAG multi-document :

- pseudonymes déterministes et réversibles par tenant ;
- token portant catégorie et identifiant de clé ;
- AES-256-SIV côté Hacienda ;
- rotation additive des clés ;
- résolution de clé tenant-scoped ;
- révélation via `HaciendaFacade::reveal_token_with_auth` ;
- contrôle `pii:reveal` et entrée d’audit `Reveal` ;
- normalisation explicite des valeurs avant chiffrement.

Ce contrat permet de rechercher un token cohérent dans plusieurs documents et de conserver la
co-référence nécessaire au RAG. Le token peut donc apparaître dans Tiptap, LanceDB, les
citations et les réponses Claude/Codex, mais jamais la valeur brute.

### Contrat Xberg : map document-local opt-in

Xberg documente une API Rust `redact_capturing_rehydration_map` avec `TokenReplace`, puis
`encrypt_map`/`decrypt_map` sous la feature `redaction-rehydrate`. La map est en clair en
mémoire et Xberg renvoie des octets chiffrés, mais la persistance, la clé, l’autorisation et
le cycle de vie sont laissés à l’application. Cette API n’est pas exposée aujourd’hui par les
bindings Python/TypeScript.

Ce mode est acceptable pour un document local isolé si l’application :

1. chiffre immédiatement la map ;
2. place le blob dans le coffre local, jamais dans LanceDB searchable ;
3. enregistre `rehydration_scheme=xberg_map_v1` et la version de politique ;
4. limite la révélation au viewer local ;
5. audite chaque révélation et ne renvoie jamais le texte réhydraté au LLM.

Il ne doit pas remplacer silencieusement le pseudonyme Hacienda : les tokens Xberg
`[PERSON_1]` sont document-local et ne garantissent pas la corrélation sécurisée entre deux
documents ou deux tenants.

## Flux de réhydratation sûr

```text
Réponse Claude/Codex token-only
        │
        ▼
Gateway Hacienda
  ├── valide le format et la version token
  ├── vérifie tenant + dossier + version + ACL
  ├── demande une capacité viewer explicite
  ├── révèle uniquement les tokens autorisés
  ├── écrit l’audit avant/après opération
  └── transmet le texte clair uniquement au viewer Tauri
```

Interdictions :

- aucune réhydratation dans la requête LLM ;
- aucun outil MCP `pii_reveal` disponible au modèle dans le mode production ;
- aucune valeur claire dans le transcript, les citations RAG, les logs ou les snapshots ;
- aucune réhydratation automatique d’une réponse entière sans sélection humaine ;
- aucune map Xberg document-local utilisée pour révéler un token Hacienda, ou inversement.

Pour le profil Local Full, le gateway peut être une commande Rust Tauri interne, mais la même
API de politique doit être conservée : `RevealRequest { tenant, dossier, version, token,
reason, viewer_capability }` → plaintext dans un canal mémoire scoped au viewer, jamais dans
un événement global Tauri ou un log.

Pour le profil Team On-Premise, la réhydratation doit être serveur-side : le client demande une
capacité courte durée, le serveur vérifie l’ACL et renvoie un flux viewer limité ou un contenu
à usage unique. Le serveur ne doit pas renvoyer la valeur au contexte Claude/Codex.

## Matrice de décision

| Fonction | Local Full | Team On-Premise |
|---|---|---|
| Extraction Xberg | Tauri/Hacienda Core local | Hacienda Engine serveur |
| Redaction | locale, fail-closed | serveur, tenant-scoped |
| LanceDB | Embedded chemin privé local | LanceDbStore serveur, remote/shared possible |
| Control plane | fichiers/audit local chiffré | Postgres multi-tenant + audit/review |
| Originaux | coffre local OS-keychain | object store privé chiffré |
| Pseudonymes | Hacienda stable par profil local | Hacienda stable par tenant |
| Map Xberg | optionnelle, document-local | à éviter ; préférer le contrat Hacienda |
| Réhydratation | viewer Tauri local | viewer Tauri avec capacité serveur |
| Claude/Codex | gateway local | gateway on-premise |
| Réseau obligatoire | non | réseau privé interne, LLM selon allowlist |
| Synchronisation | opt-in, chiffrée, conflict-aware | serveur autoritaire, clients synchronisés |

## Compatibilité avec `RagStore`

Le trait actuel est single-tenant par conception. L’adaptateur LanceDB doit recevoir une
instance déjà scoped par trust domain ou un contexte injecté par Hacienda, et ne doit jamais
prendre un tenant libre depuis `RetrieveQuery`.

Correspondance minimale :

| `RagStore` | LanceDB/Xberg |
|---|---|
| `ensure_collection` | table LanceDB + schéma Arrow + index vector/FTS |
| `upsert_document` | projection token-only + `merge_insert` par clé composite |
| `retrieve Vector` | `nearest_to` |
| `retrieve FullText` | index FTS/BM25 sur `search_text_token_only` |
| `retrieve Hybrid` | deux recherches + fusion/reranking contrôlé |
| `filter` | `only_if` après compilation d’un filtre allowlisté |
| `get_document_chunks` | lecture version token-only, jamais original |
| `delete_by_filter` | suppression scoped + audit Hacienda |
| `set_embedding_provenance` | métadonnées de table/collection controlées par Hacienda |
| `Sparse/LateInteraction` | `UnsupportedMode` initial, sauf adaptateur prouvé |

Le chunker doit travailler après redaction, jamais sur `ExtractedDocument` original. Les
embeddings Xberg doivent être calculés sur `content_token_only`, et le modèle d’embedding,
sa version et sa dimension doivent être enregistrés sans secret.

## Fonctionnalités LanceDB à activer pour les dossiers juridiques

GLiNER 2.5/Fastino est utilisé avant LanceDB pour détecter les personnes, organisations,
lieux, références et autres catégories sensibles. Il ne remplace pas l’embedding ni le
retrieval. Ses résultats doivent enrichir les métadonnées non secrètes, la provenance et les
tokens, sans remettre les mentions brutes dans l’index.

### À activer dès le premier prototype

1. **Recherche vectorielle dense avec index ANN** : utile pour les questions paraphrasées,
   les arguments juridiques similaires et la recherche transversale dans un dossier. Utiliser
   un embedding local ou allowlisté calculé après redaction ; GLiNER n’est pas l’embedder.
2. **FTS/BM25** sur `search_text_token_only` : indispensable pour les articles de loi, noms
   de juridictions, numéros de décision, références de contrats, dates et formulations
   exactes. Le FTS nécessite un index et renvoie les résultats par score BM25.
3. **Hybrid search + RRF** : chemin par défaut pour une question juridique, car le lexical
   retrouve les références précises et le vectoriel retrouve les formulations équivalentes.
   La documentation Rust indique que le reranking LanceDB est actuellement exposé pour la
   recherche hybride.
4. **Redaction de toutes les surfaces documentaires** : le titre, nom de fichier, chemin
   logique, auteur, sujet, mots-clés, URL/label, nom de pièce jointe, propriétés PDF/Office,
   contenu de tableau et champs structurés doivent être traités comme du contenu sensible.
   GLiNER et les règles Hacienda doivent s’appliquer avant de produire les projections
   `title_token_only`, `metadata_token_only` et `search_text_token_only`. Aucun titre brut ne
   doit apparaître dans LanceDB, les citations, les logs ou le prompt fournisseur.
5. **Filtres avant retrieval** : `tenant_id`, `dossier_id`, `version_id`, `document_type`,
   `jurisdiction`, `language`, `effective_from/effective_to`, `sensitivity_level`,
   `review_state` et `acl_reference`. Le filtre doit être compilé par Hacienda depuis une
   allowlist de champs, jamais construit directement depuis une chaîne utilisateur.
6. **Index scalaires** : BTree pour dates, versions, catégories et statuts ; LabelList pour
   les tags de dossier et catégories GLiNER. Les filtres restent une décision Hacienda, même
   si LanceDB les exécute.
7. **Métadonnées de citation** : `document_id`, `version_id`, page, section, paragraphe,
   offset tokenisé et `viewer_ref` opaque. Elles permettent à Claude/Codex de citer le passage
   tokenisé et à Tauri d’ouvrir le bon original sans transmettre le chemin ou la PII.
8. **`merge_insert` + versions de table** : utile pour les éditions Tiptap, les re-chunks et
   les reprises idempotentes. La version LanceDB ne remplace pas la version juridique,
   l’audit ou la politique de conservation dans Postgres/Hacienda.
9. **`optimize`/compaction et métriques** après ingestion ou mise à jour par lots. Les index
   ne doivent pas être supposés synchronisés automatiquement avec chaque ajout ; le job
   d’indexation doit exposer un état `pending/ready/failed` et une métrique de fraîcheur.

### À activer en seconde phase

- **`refine_factor`, `nprobes` et recherche exacte/ANN** pour régler le compromis rappel-latence
  par taille de corpus ; benchmarker sur les dossiers réels avant de choisir les valeurs.
- **Deux champs lexicaux** : un champ français normalisé pour le langage naturel et un champ
  exact/ngram pour les citations (`L. 123-4`, `RG n°`, références internes). Les résultats
  peuvent être fusionnés dans l’adaptateur plutôt que de dégrader le champ principal avec
  une tokenisation unique.
- **Reranker local spécialisé juridique** après le candidat hybride, si la mesure nDCG montre
  un gain. Il doit recevoir uniquement le texte tokenisé et les métadonnées autorisées.
- **`explain_plan` et statistiques de table** pour vérifier les filtres, les index utilisés,
  la fraîcheur et les coûts de requête en environnement équipe.
- **Version checkout/restore** pour les opérations de récupération après erreur, jamais comme
  mécanisme unique de suppression légale : la purge doit aussi couvrir snapshots, backups,
  object store et audit selon la politique retenue.

### À ne pas activer par défaut

- **Sparse/SPLADE et late-interaction/ColBERT** : pas de capacité Rust LanceDB équivalente
  confirmée dans l’étude ; ne pas les annoncer via `Capabilities` sans adaptateur et tests.
- **Blobs LanceDB pour les originaux** : les blobs existent, mais l’original et la map de
  réhydratation doivent rester dans le coffre séparé. Au maximum, stocker une référence ou un
  blob déjà chiffré dans une table non searchable, sans FTS ni embedding.
- **RTE/real-time ingestion** : inutile pour le premier cas juridique ; privilégier une file
  idempotente, un commit de version puis une indexation contrôlée et observable.
- **Embedding de contenu brut** : interdit. Toute re-embedding doit repartir du texte
  tokenisé et enregistrer la version du modèle, le digest et la politique de redaction.

### Métadonnées GLiNER recommandées

```text
ner_model_id             = "fastino/GLiNER2-..."
ner_model_digest         = "opaque-digest"
ner_policy_version       = "pii-policy-vN"
entity_types             = ["person", "organization", "case_reference"]
entity_token_ids         = ["opaque-token-id-1", "opaque-token-id-2"]
redaction_review_state   = "automatic|human_reviewed|approved"
has_sensitive_content    = true
```

`entity_types` et `entity_token_ids` peuvent servir à filtrer ou à expliquer le résultat sans
révéler les mentions. La valeur détectée par GLiNER, le span original et la map de révélation
restent dans le coffre/audit, pas dans les métadonnées LanceDB.

### Profil recommandé pour un dossier juridique

```text
query
  ├── résolution tenant/dossier/version/ACL Hacienda
  ├── FTS exact + BM25 sur tokens et références
  ├── vector ANN sur embeddings token-only
  ├── fusion RRF
  ├── reranking local optionnel
  ├── group_by_document + citations page/section
  └── réponse LLM token-only avec viewer_ref opaque
```

Le résultat optimal pour le secret professionnel est donc **hybride + filtres stricts +
citation structurée**, et non un simple vector store.

## Décision finale

1. **Local Full :** Tauri embarque Hacienda/Xberg, LanceDB Embedded et un coffre de
   réhydratation local. C’est l’installation autonome de référence.
2. **Team On-Premise :** Tauri est un client, Hacienda Engine/Gateway est le control plane
   multi-tenant, LanceDB est le data plane RAG token-only et Postgres/object store complètent
   le backend. C’est l’installation d’équipe de référence.
3. Le contrat Hacienda de pseudonymisation stable reste le contrat de réhydratation par défaut.
   Le mécanisme Xberg `RehydrationMap` reste une option document-local explicitement marquée.
4. L’accès direct client → LanceDB est interdit dans les deux profils de production.
5. La priorité suivante est un prototype `LanceDbStore` sur les deux profils, avec tests de
   non-egress, isolation tenant, audit de révélation, crash/reprise et compatibilité des
   embeddings Xberg.

## Sources

- [Xberg — redaction et réhydratation](https://github.com/xberg-io/xberg/blob/main/docs-site/src/content/docs/guides/redaction.mdx)
- [LanceDB Rust — connexion, tables et recherche](https://docs.rs/lancedb/latest/lancedb/)
- [Contrat local `RagStore`](../../crates/hacienda-rag/src/store.rs)
- [Contrat de requête RAG](../../crates/hacienda-rag/src/query.rs)
- [Surface Hacienda Xberg](../../../hacienda/hacienda-core/src/xberg_surface.rs)
- [API Hacienda PII](../../../hacienda/hacienda-api/src/handlers/pii.rs)
- [Hacienda Xberg surface parity](../../../hacienda/docs/superpowers/specs/2026-11-14-xberg-surface-parity-design.md)
- [Revue Xberg/Hacienda de pseudonymisation](../../../hacienda/docs/research/2026-11-13-official-xberg-pseudonymization-review.md)

## Addendum d’implémentation — 2026-11-15

La première tranche LocalFull est livrée dans `6e6f9eb` : ingestion Xberg/Hacienda,
redaction avant création de projection, contenu Tiptap token-only, stockage LanceDB,
révisions append-only et ouverture native auditée de l’original. La liaison vers le chemin
original et certaines références restent toutefois en mémoire de session ; le rechargement
après redémarrage, le coffre documentaire durable et l’intégration TeamOnPremise de bout en
bout ne sont pas établis par ce commit.

## Validation du runtime d’embedding Xberg — 2026-11-15

Le smoke test isolé du client a chargé le preset `multilingual` avec la bibliothèque officielle
ONNX Runtime 1.23.2 pour macOS x86_64 (version utilisée car la distribution officielle ne fournit
plus de version macOS x86_64 plus récente). Les fichiers du preset ont été téléchargés depuis la
révision Xberg épinglée ; `model.onnx` (1,11 Go) correspond à son SHA-256 de manifeste. En forçant
`XBERG_ORT_EP=cpu`, l’inférence a réussi sur un texte tokenisé, avec un vecteur fini de 768 éléments.

Le choix automatique d’Xberg sous macOS Intel a tenté de compiler le graphe pour CoreML et n’a pas
terminé après plus de 18 minutes. La configuration du client choisit donc explicitement le CPU sur
macOS x86_64 et conserve l’accélération automatique sur les autres cibles. Ce constat motive le
choix de fournisseur pour cette cible ; il ne constitue pas un benchmark comparatif.

Limites : le smoke est isolé et ne couvre pas encore le paquet Tauri distribué, la découverte et le
chargement du runtime ONNX depuis ses ressources, ni le parcours complet ingestion/recherche hybride
dans le binaire client. Ces points restent des critères de livraison LocalFull. Le modèle GLiNER2
dispose d’une validation séparée documentée dans l’issue #131 ; ces validations ne prouvent pas les
viewer multi-format, la durabilité du coffre ou Team On-Premise de bout en bout.

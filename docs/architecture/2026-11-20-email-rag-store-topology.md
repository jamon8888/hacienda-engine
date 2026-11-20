# Topologie du store RAG : documents et emails partagent le même LanceDB

**Date :** 2026-11-20
**Contexte :** intégration du plugin `hacienda-email` (PR #166, mergée) et durcissement
multi-tenant du data plane RAG (PR #163). Ce document fixe une décision d'architecture
qui sinon reste implicite dans le code et donne lieu à des contre-propositions
récurrentes (« il faut un LanceDB séparé pour les emails »).

---

## Décision

**Un seul `RagStore` par `ApiState`, partagé entre documents et emails.** La séparation
est **logique** (tenant + dossier + type de collection), pas **physique** (un seul
fichier LanceDB / une seule base Postgres).

```
ApiState.rag_store: Option<Arc<dyn RagStore>>
        │
        ├── /v1/documents/*  → collection "<tenant>:<name>"     (documents)
        ├── /v1/email/*      → collection "email-corpus"        (emails redactés)
        └── /v1/rag/*        → retrieve() sur les deux, avec les mêmes filtres
```

Le plugin email reçoit le même `Arc<dyn RagStore>` que les documents via
`EmailProcessor::with_rag_store()` (`hacienda-api/src/handlers/email.rs`). Les emails
sont indexés après redaction obligatoire (design D2 du plugin) dans la collection
`email-corpus` (config `RagConfig.index_name`), avec `tenant_id` et `dossier_id`
estampés sur chaque ligne, puis **interrogeables par les mêmes endpoints
`/v1/rag/*/retrieve`** que les documents.

---

## Pourquoi un seul store

1. **Séparation logique, pas physique.** `RagStore` est injecté ; l'isolation est une
   propriété des données (`tenant_id` / `acl` par ligne, nom de collection préfixé par
   le tenant pour les documents), pas du moteur. C'est le modèle de hacienda-core :
   `AuditStore`/`ReviewStore`/`JobStore` suivent déjà le même patron.
2. **Un seul point de garde de confidentialité.** Tout contenu — document **ou** email —
   passe par le même `GuardedLlm` / `redact_retrieval_output` avant d'être renvoyé par
   l'API. Deux stores imposeraient de dupliquer ce guard, et rouvriraient le gap E4
   (« ingestion-time redaction guard ») que le plugin email a précisément fermé.
3. **Pipeline partagé.** `chunk_full_text`, le filtre `Filter` IR, le retrieval hybride
   (dense + FTS + RRF), la provenance d'embedding et l'optimi(s)ation du store sont
   réutilisés tels quels.
4. **Une seule infra à exploiter.** Un seul fichier LanceDB à sauvegarder, chiffrer,
   optimiser — un seul schéma à faire évoluer (migrations `0102` dossier/version,
   `0103` tenant/ACL).

---

## Le risque et sa mitigation

Le risque n'est pas « deux bases », c'est **« un tenant qui voit les emails d'un
autre »**. Il est traité à trois niveaux (PR #163) :

- **Stockage** : `tenant_id` / `acl` persistés sur chaque chunk (migration SQL `0103`
  pour pgvector, `add_columns`/`AllNulls` pour LanceDB). `NULL` = tenant par défaut.
- **Avant scoring** : les trois backends (memory, LanceDB, pgvector) appliquent
  `query.tenant_id` et `query.acl_reference` comme frontières dures avant de scorer.
- **Au handler** : `tenant_id` est estampé **côté serveur** depuis le `TenantCtx`
  authentifié — jamais cru depuis le corps de requête — et le check post-retrieval
  conserve un chunk si son tenant **ou** son ACL autorise l'appelant.

---

## Quand séparer physiquement

La séparation physique n'est justifiée que par une contrainte de **déploiement**, pas
de logique : par exemple le profil « Local Full » où les emails restent sur le poste de
l'avocat alors que les documents sont sur le serveur « Team On-Premise ».

Dans ce cas, l'architecture propre est :

- deux `ApiState` / deux `RagStore` construits sur des racines différentes
  (`LanceDbStore::connect(path_local)` vs `connect(path_server)`),
- deux groupes de routes (`/v1/email/*` → store local, `/v1/documents/*` → store serveur),
- **aucun changement de code métier** : le contrat `RagStore` est identique, seule
  l'injection au démarrage diffère.

Le code actuel le permet déjà : `with_rag_store()` est une injection. Il ne s'agit donc
pas d'une limitation, mais d'un choix par défaut — un seul store avec isolation
logique — qui peut être révisé sans refonte si le besoin de déploiement se confirme.

---

## Références

- `crates/hacienda-email/DESIGN.md` — décisions D1 (redaction/pseudonymisation par
  tenant), D2 (redaction avant indexation), D5 (pas de processor email partagé sur HTTP)
- `docs/architecture/README.md` §3 — registre des corrections et décisions
- `crates/hacienda-rag/migrations/0103_add_tenant_acl.sql` — schéma multi-tenant

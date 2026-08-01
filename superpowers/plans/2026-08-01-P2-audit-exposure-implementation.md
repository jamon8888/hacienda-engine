# P2 — Exposition et vérification de la chaîne d'audit — Plan d'implémentation

> **Pour les agents :** SOUS-COMPÉTENCE REQUISE — utiliser `superpowers:subagent-driven-development`
> (recommandé) ou `superpowers:executing-plans` pour exécuter ce plan tâche par tâche. Les étapes
> utilisent la syntaxe case à cocher (`- [ ]`).

**Objectif :** rendre la chaîne d'audit consultable, vérifiable et exportable en HTTP, sans
qu'aucune réponse ne contienne de valeur du corpus.

**Architecture :** cinq routes ajoutées à `ROUTE_TABLE`, des handlers dans un nouveau
`hacienda-api/src/handlers/audit.rs`, et **deux extensions du trait `AuditStore`** rendues
nécessaires par la vérité terrain ci-dessous : l'histoire complète et l'export ne sont pas
joignables depuis le trait aujourd'hui.

**Pile :** Rust 2021, `axum` 0.8, `serde_json`, types existants de `hacienda_core::audit`.
Aucune dépendance nouvelle.

**Spec :** `superpowers/specs/2026-08-01-P2-audit-exposure-and-verification.md`.

**Programme :** `2026-08-01-hacienda-platform-parity-program.md` §5.2, vague 0.

---

## Vérité terrain — vérifié contre le code, 2026-08-01

**Vérifié en lisant les sources :**

| Fait | Emplacement |
| --- | --- |
| `AuditStore` expose `append`, `entries`, `tip`, `seals`, `verify`, `rotate`, `close` | `hacienda-core/src/audit/store.rs:42-` |
| **`entries()` ne rend que le segment ouvert**, pas l'histoire. Le doc du trait le dit : les segments scellés sont accessibles « via `seals`, ou en relisant les fichiers JSONL d'un backend fichier » | `store.rs:50-56` |
| `HaciendaFacade::audit_entries_with_auth` hérite de cette limite et la documente | `facade.rs:294`, `facade.rs:281-286` |
| **`export` prend `&AuditChain`, pas `&dyn AuditStore`.** Il n'existe aucun chemin d'export depuis un store | `audit/export.rs:18` |
| `ExportFormat` = `JsonLines`, `Json`, `Csv` | `audit/export.rs:7-11` |
| `AuditEntry` porte `id`, `timestamp`, `category`, `action`, `span_hash`, `span_length`, `confidence`, `source`, `pipeline_version`, `config_hash`, `principal`, plus le hachage de chaîne | `audit/entry.rs:68-88` |
| `SegmentSeal` porte `segment_id`, `node_id`, `config_hash`, `prev_seal_hash`, `sealed_tip`, `entry_count`, `opened_at`, `sealed_at` | `audit/segment.rs:293-307` |
| `verify_seal_chain` et `compute_seal_hash` sont exportés | `audit/mod.rs:34` |
| `audit_tip` n'est **pas** gardée par capacité, délibérément et avec justification écrite | `facade.rs:315`, doc en `facade.rs:306-314` |
| `Capability::AuditRead` et `AuditExport` existent déjà et sont distinctes | `auth/mod.rs:24-27` |
| `ROUTE_TABLE` lie chemin, accès et handler ; `build_auth_state` en dérive l'autorisation | `hacienda-api/src/routes.rs:54-127` |
| `ApiError` a `not_found`, `forbidden`, `unauthenticated`, `internal`, et une enveloppe `{code, ...}` testée | `hacienda-api/src/error.rs:64-125` |
| Le patron de handler est `State` + `Parts` → `extract_auth_context` → `caller_from_arc` → appel façade → `map_err(ApiError::from)` | `handlers/pii.rs:26-45` |

**Deux manques structurels, qui deviennent les tâches 1 et 2 :**

1. **Pas d'accès à l'histoire complète.** `entries()` rend le segment ouvert. Une API qui
   annoncerait « les entrées d'audit » en n'en rendant qu'une fraction, sans le dire, serait
   pire que pas d'API : un auditeur conclurait à l'absence d'événements qui existent.
2. **Pas d'export depuis un store.** `export(&AuditChain, _)` ne s'applique pas à un
   `Arc<dyn AuditStore>`.

**Hypothèses non vérifiées, à confirmer en tâche 1 :**

- que `FileAuditStore` puisse relire ses segments scellés sans réouvrir les fichiers un par un ;
- que la volumétrie d'une chaîne de production tienne le streaming sans pagination côté store.

---

## Tâche 1 — Lecture de l'histoire complète

- [ ] **Étape 1.1** — Lire `audit/store_file.rs` et établir si les segments scellés sont
      relisibles depuis le store. Écrire la réponse dans ce plan avant d'écrire du code.
- [ ] **Étape 1.2** — Ajouter au trait `AuditStore` :

      ```rust
      /// Entries across sealed segments and the open one, oldest first.
      async fn history(&self, after: Option<&str>, limit: usize)
          -> Result<Vec<AuditEntry>, AuditError>;
      ```

      `after` est l'identifiant d'entrée servant de curseur — jamais un offset (décision
      D-P2-4 de la spec).
- [ ] **Étape 1.3** — Implémenter pour `InMemoryAuditStore`, `FileAuditStore`,
      `IndexedDbAuditStore`. Le backend IndexedDB peut renvoyer `AuditError` « non supporté »
      s'il ne conserve pas les segments scellés : mieux vaut refuser que rendre partiel.
- [ ] **Étape 1.4** — Test : écrire assez d'entrées pour forcer deux rotations, puis vérifier
      que `history` les rend **toutes**, dans l'ordre, à travers les segments scellés.
- [ ] **Étape 1.5** — Test : appeler `history` pendant qu'un `append` concurrent tourne ;
      aucune entrée ne doit être sautée ni dupliquée entre deux pages.

## Tâche 2 — Export depuis un store

- [ ] **Étape 2.1** — Ajouter `async fn export(&self, format: ExportFormat) -> Result<Vec<u8>, AuditError>`
      au trait, ou une fonction libre prenant `&dyn AuditStore`. **Décider laquelle et
      écrire pourquoi** : une méthode de trait oblige chaque backend à l'implémenter ; une
      fonction libre s'appuie sur `history` et n'a qu'une implémentation.
- [ ] **Étape 2.2** — L'export **enveloppe entrées et sceaux ensemble** (décision D-P2-5) : un
      export d'entrées seules ne se vérifie pas hors ligne.
- [ ] **Étape 2.3** — Test : exporter, vérifier la chaîne **hors du serveur** à partir du seul
      export, sceaux compris.

## Tâche 3 — Les cinq routes

- [ ] **Étape 3.1** — Créer `hacienda-api/src/handlers/audit.rs`, en suivant le patron de
      `handlers/pii.rs`.
- [ ] **Étape 3.2** — Ajouter à `ROUTE_TABLE` :

      | Chemin | Accès |
      | --- | --- |
      | `/v1/audit/entries` | `Capability(AuditRead)` |
      | `/v1/audit/verify` | `Capability(AuditRead)` |
      | `/v1/audit/seals` | `Capability(AuditRead)` |
      | `/v1/audit/export` | `Capability(AuditExport)` |
      | `/v1/audit/tip` | `Capability(DocumentsProcess)` |

- [ ] **Étape 3.3** — DTO dans `dto.rs` : `AuditEntryDto`, `SegmentSealDto`, `VerifyResponse`,
      `AuditPage { entries, next_cursor }`.
- [ ] **Étape 3.4** — `verify` rend 200 avec le résultat, y compris en cas de rupture : une
      chaîne rompue est une **réponse**, pas une erreur serveur. Le corps nomme l'entrée ou le
      sceau fautif (décision D-P2-2).
- [ ] **Étape 3.5** — Vérifier que `every_guarded_route_reflected_in_auth_state` passe toujours
      — il balaie la table, donc les nouvelles routes y entrent automatiquement.

## Tâche 4 — Le test qui compte

- [ ] **Étape 4.1** — `no_endpoint_returns_corpus_plaintext` : traiter un corpus témoin aux
      valeurs distinctives, puis interroger **chacune** des cinq routes et asserter qu'aucune
      réponse ne contient une de ces valeurs. Balayer la table plutôt que lister les routes à
      la main, pour qu'une route ajoutée plus tard soit couverte d'office.
- [ ] **Étape 4.2** — `verify_names_the_broken_entry` : altérer une entrée sur disque, appeler
      `/v1/audit/verify`, asserter un 200 nommant l'entrée — pas un 500 opaque.
- [ ] **Étape 4.3** — `tip_is_reachable_with_documents_process_alone` : jeton
      `hcd_documents:process_*`, `/v1/audit/tip` ne rend ni 401 ni 403.
- [ ] **Étape 4.4** — `export_requires_audit_export_not_audit_read` : un jeton portant
      `audit:read` seul reçoit 403 sur `/v1/audit/export`.

## Tâche 5 — Documentation

- [ ] **Étape 5.1** — Décrire les cinq routes dans le document OpenAPI. **Si S4 n'est pas encore
      livré**, ajouter au moins les chemins ; le schéma complet viendra avec S4.
- [ ] **Étape 5.2** — Documenter dans le README de l'API **ce que la chaîne prouve** et en quoi
      elle diffère d'un journal d'activité. C'est l'argument commercial (spec §2), et il doit
      être lisible par un acheteur, pas seulement par un développeur.
- [ ] **Étape 5.3** — Entrée `CHANGELOG.md` sous `[Unreleased] / Added`.

---

## Critères de sortie

- [ ] Un client prouve qu'une entrée n'a pas été altérée **sans accès au stockage**.
- [ ] `history` rend l'histoire complète à travers les segments scellés — vérifié après deux
      rotations.
- [ ] Un export se vérifie hors ligne, sceaux compris.
- [ ] Aucune des cinq routes ne rend une valeur du corpus témoin.
- [ ] `cargo clippy --all-targets --all-features -D warnings` et `cargo fmt --check` propres.

## Hors périmètre

Cloisonnement par tenant (→ S1, qui ajoutera `TenantCtx` en paramètre de `history` et
`export`), rétention, purge, alerting sur rupture de chaîne.

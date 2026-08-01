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

**Ligne de base (2026-08-01, `cargo test -p hacienda-core -p hacienda-api`) :**
**284 passés, 4 échecs préexistants, 2 ignorés.**

Les quatre échecs sont une limitation d'environnement, **pas un défaut du code et pas notre
fait** :

```text
audit::store_file::tests::should_poison_store_after_a_failed_append_write
audit::store_file::tests::should_not_include_phantom_entries_in_chain_after_a_failed_append_and_reopen
review::store_file::tests::should_poison_store_and_not_expose_phantom_state_after_a_failed_decide_write
review::store_file::tests::should_not_show_phantom_decision_after_a_failed_write_and_restart
```

Ils simulent un échec d'écriture par `fs::set_permissions(0o444)` (`store_file.rs:1485`), et
cet environnement tourne en **uid 0** : root outrepasse les contrôles de permission, l'écriture
réussit, le test échoue. La CI tourne en non-root et ils y passent.

**Consigne aux agents : ne pas chercher à les réparer, ne pas les compter comme régression.**
Le critère est « 284 passés inchangés, aucun nouvel échec ».

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

## Résultat de l'investigation 1.1 — conception révisée

L'étape 1.1 a été exécutée le 2026-08-01. Quatre constats ont été **re-vérifiés
indépendamment** et invalident trois points du plan initial. Chacun est corrigé ci-dessous.

| Constat | Preuve | Effet |
| --- | --- | --- |
| **`read_jsonl` est une lecture *mutante*** : `discard_unterminated_tail` fait `set_len` + `sync_all` | `store_file.rs:820-823` | Un `history()` qui lirait le fichier du segment **ouvert** pendant un `append` concurrent peut observer un enregistrement à moitié écrit et **le tronquer**. Le segment ouvert doit être servi depuis `state.open` en mémoire, jamais depuis le disque — c'est déjà ce que fait `verify()` (`store_file.rs:520-541`) |
| **Le module `export` n'existe pas sur wasm32** | `mod.rs:16-17`, `mod.rs:32-33` | Une méthode de trait prenant `ExportFormat` **ne compile pas** là où vit `IndexedDbAuditStore`. Tranche l'étape 2.1 : fonction libre, cfg-gatée |
| **Chaque segment redémarre à `GENESIS_HASH`** et `AuditChain::append` rejette toute entrée n'étendant pas la tête courante | `segment.rs:167` (`AuditChain::new`), `chain.rs:56-64` | On **ne peut pas** reconstruire une `AuditChain` unique à partir d'une histoire multi-segments. Un export passant par `AuditChain` échouerait à la première rotation |
| **`export_csv` omet `principal`**, alors que `chain_hash` le couvre | `export.rs:46-50`, `entry.rs:79-87` | Un export CSV **ne se vérifie pas hors ligne**. L'enveloppe vérifiable doit être JSON/JSONL |

Deux constats supplémentaires, non bloquants mais structurants :

- **Les trois backends conservent les segments scellés** — `FileAuditStore` sur disque
  (`store_file.rs:90-91`), `InMemoryAuditStore` et `IndexedDbAuditStore` en mémoire avec leurs
  entrées (`store.rs:153`, `store_idb.rs:54-59`). L'échappatoire « le backend IndexedDB peut
  refuser » du plan initial est **inutile** : supprimée.
- **Il n'existe pas d'ordre total inter-nœuds.** Un `FileAuditStore` ne voit que son propre
  `node_dir` (`store_file.rs:258`, et la note `116-122`). `history()` rend donc *l'histoire de
  ce nœud*, pas celle du déploiement. **Obligation de documentation**, sinon l'API dit « les
  entrées d'audit » en pensant « celles de ce nœud » — exactement le mode d'échec que ce plan
  reproche à `entries()`.

---

## Tâche 1 — Lecture de l'histoire complète

- [x] **Étape 1.1** — Investigation faite. Voir la section ci-dessus.
- [x] **Étape 1.2** — Ajouter les types de curseur dans `hacienda-core/src/audit/` :

      ```rust
      /// Position immuable dans la chaîne : l'entrée à `index` dans le segment `segment_id`.
      /// `index` est la position que `AuditChain::verify` utilise comme numéro de séquence
      /// (`chain.rs:81-83`) — une valeur que la chaîne engage déjà, pas un décalage fortuit.
      pub struct AuditCursor { pub segment_id: String, pub index: u64 }
      // + FromStr / Display sur une forme opaque "{segment_id}:{index}"

      pub struct AuditPage { pub entries: Vec<AuditEntry>, pub next: Option<AuditCursor> }
      ```

- [x] **Étape 1.3** — Ajouter au trait `AuditStore` :

      ```rust
      /// Entrées des segments scellés puis du segment ouvert, plus anciennes d'abord,
      /// **pour ce nœud**. `after` est un curseur opaque rendu par cette méthode —
      /// jamais un décalage, jamais un identifiant d'entrée.
      async fn history(&self, after: Option<&AuditCursor>, limit: usize)
          -> Result<AuditPage, AuditError>;
      ```

      **Pourquoi pas `Option<&str>` sur un identifiant d'entrée** : `AuditEntry` ne porte ni
      `segment_id` ni numéro de séquence (`entry.rs:68-90`), il n'existe aucun index sur
      disque, et l'identifiant est un uuid v4 donc non triable. Le retrouver imposerait un
      balayage complet de l'histoire **à chaque page**. `(segment_id, index)` se résout en une
      ouverture de fichier via `jsonl_path` (`store_file.rs:673-675`) contre le `Vec<SegmentSeal>`
      déjà ordonné en mémoire (`store_file.rs:92`).

      Le trait doit rester **objet-safe** — propriété épinglée par
      `should_construct_arc_dyn_audit_store` (`store.rs:497-502`).

- [x] **Étape 1.4** — Implémenter pour les trois backends. Pour `FileAuditStore` :
      **segments scellés depuis le disque, segment ouvert depuis `state.open`** — jamais
      `read_jsonl` sur le fichier vivant (constat 1).
- [x] **Étape 1.5** — Ne **pas** prendre `io_order` en lecture : cela bloquerait tous les
      `append` pendant une lecture disque O(n), et un verrou ne peut de toute façon pas couvrir
      deux requêtes HTTP. La correction de la pagination vient de l'immuabilité du curseur, pas
      d'un verrou.
- [x] **Étape 1.6** — Test : forcer deux rotations, vérifier que `history` rend **toutes** les
      entrées, dans l'ordre, à travers les segments scellés.
- [x] **Étape 1.7** — Test : paginer pendant qu'un `append` concurrent tourne ; aucune entrée
      sautée ni dupliquée. Les entrées ajoutées après la frappe du curseur apparaissent
      simplement sur une page ultérieure — croissance, pas dérive.
- [x] **Étape 1.8** — Test : après `close()`, `history` rend le segment final depuis le disque.
      `entries()` rend vide dans cet état (`store_file.rs:488-492`) ; `history` **ne doit pas**
      hériter de ce comportement.
- [x] **Étape 1.9** — Un curseur inconnu rend une erreur explicite, jamais un redémarrage
      silencieux depuis le début — qui dupliquerait.

## Tâche 2 — Export depuis un store

- [ ] **Étape 2.1** — **Fonction libre, cfg-gatée**, décidée par le constat 2 :

      ```rust
      #[cfg(not(target_arch = "wasm32"))]
      pub async fn export_store(store: &dyn AuditStore, format: ExportFormat)
          -> Result<Vec<u8>, AuditError>;
      ```

      Pas une méthode de trait : `ExportFormat` n'existe pas sur wasm32, où vit
      `IndexedDbAuditStore`.
- [ ] **Étape 2.2** — **Ne pas router par `AuditChain`** (constat 3). Construire l'enveloppe
      directement depuis `history()` et `seals()`.
- [ ] **Étape 2.3** — L'enveloppe porte **entrées et sceaux ensemble** (décision D-P2-5),
      désormais obligatoire et non plus souhaitable : sans les sceaux, une histoire
      multi-segments n'a aucune continuité vérifiable.
### Décision D-P2-6 — séparer l'enveloppe de preuve de l'extraction tabulaire

**Le cadrage « faut-il ajouter la colonne `principal` au CSV ? » était faux.** La vérification
d'une entrée exige `compute_chain_hash(prev_chain_hash, seq, {id, category, action, span_hash,
config_hash, principal})` (`entry.rs:176-190`). Il manque donc au CSV **deux choses, pas une** :

1. **`principal`**, une entrée du hachage, absente des colonnes (`export.rs:46-50`) ;
2. **les frontières de segments.** Le CSV est une liste plate. Chaque segment redémarre à
   `GENESIS_HASH` avec `seq` remis à 0 (`segment.rs:167`), et rien dans le fichier ne dit où.
   Un vérificateur ne peut donc ni connaître `seq`, ni savoir où `prev` se réinitialise.

**Conséquence : ajouter `principal` ne rendrait pas le CSV vérifiable.** Une liste plate d'entrées
est structurellement incapable de porter une histoire multi-segments vérifiable, quelles que
soient ses colonnes.

Le défaut réel est la **confusion de deux artefacts** qui n'ont ni le même usage ni le même
lecteur :

| | Enveloppe de preuve | Extraction tabulaire |
| --- | --- | --- |
| Lecteur | Régulateur, auditeur externe | Analyste, tableur, SIEM |
| Exigence | Vérifiable hors ligne | Lisible, greppable, importable |
| Format | JSON / JSONL, **groupé par segment**, sceaux inclus | CSV plat |
| Porte `seq` et `prev` recouvrables | oui, par le groupement | non, et ce n'est pas son rôle |

**Décision, en trois points :**

- [ ] **Étape 2.4a** — **L'enveloppe de preuve est JSON/JSONL, groupée par segment, sceaux
      inclus.** Le groupement n'est pas cosmétique : c'est lui qui rend `seq` et la
      réinitialisation de `prev` recouvrables, donc la vérification possible. Code neuf, aucune
      rupture.
- [ ] **Étape 2.4b** — **Ajouter `principal` au CSV** — non pour le rendre vérifiable, il ne
      peut pas l'être, mais parce que **retirer silencieusement l'attribution d'un extrait
      d'audit est un défaut en soi** : « qui a révélé cette valeur » est précisément la question
      à laquelle l'extrait sert à répondre, et le champ est couvert par `chain_hash`
      (`entry.rs:79-87`) donc réputé fiable.

      *Sur la rupture :* le CSV est sorti en 0.1.0 le 2026-07-28, il y a quatre jours ; il n'a
      **aucun appelant dans le dépôt** (seulement le ré-export `mod.rs:35`) ; et en 0.x SemVer
      autorise la rupture sur montée mineure. Corriger maintenant coûte ~zéro et le coût croît
      avec chaque consommateur. Entrée `CHANGELOG.md` sous `### Changed`, avec la raison.
- [ ] **Étape 2.4c** — **L'API doit rendre la distinction impossible à manquer.** Un appelant
      qui demande `?format=csv` reçoit une réponse indiquant explicitement qu'il ne s'agit pas
      d'une enveloppe vérifiable. Laisser un utilisateur remettre un CSV à un régulateur en le
      croyant probant est le mode d'échec que toute cette spec existe pour fermer.

**Ce qui est délibérément écarté :** ajouter `segment_id` et `seq` en colonnes pour rendre le CSV
vérifiable. Cela transformerait une table en demi-enveloppe, mal taillée pour les deux usages —
illisible en tableur, et toujours sans les sceaux nécessaires à la chaîne inter-segments.
- [ ] **Étape 2.5** — Test : exporter après deux rotations, puis vérifier la chaîne **hors du
      serveur** à partir du seul export, sceaux compris.

## Contrat hérité de la tâche 1 — à respecter en tâche 3

Trois décisions prises à l'implémentation que le plan ne couvrait pas. Elles engagent le
contrat HTTP.

1. **`AuditPage::next` est `Some` dès que la page est non vide ; `None` seulement quand la page
   est vide.** Le contrat client est donc « paginer jusqu'à recevoir une page vide », pas
   « paginer jusqu'à ce que `next_cursor` soit nul ». Raison : une chaîne d'audit ne fait que
   croître, et un appelant arrivé au bout doit garder un curseur reprenable pour suivre les
   entrées suivantes. L'alternative le laisserait sans point de reprise, donc contraint à
   relire depuis le début en dédupliquant — précisément ce que le curseur existe pour éviter.
   Coût : un aller-retour supplémentaire pour vider l'histoire.
2. **`limit == 0` rend une page vide avec `next: None`**, sans erreur côté core. Avec la
   sémantique ci-dessus, un client ne peut pas la distinguer de « à jour ». **Le handler HTTP
   doit donc rejeter ou borner `limit == 0`**, et ne pas laisser passer.
3. **Les fichiers scellés sont lus en ligne, pas sur `spawn_blocking`** — même exposition que
   `verify()` aujourd'hui. Un handler appelant `history()` bloque donc un worker du runtime sur
   de l'E/S disque. Ce n'est pas une régression, mais la tâche 3 en hérite : décider
   explicitement plutôt que de le découvrir sous charge.

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
- [ ] **Étape 5.4** — Documenter que `history` rend **l'histoire de ce nœud**, pas celle du
      déploiement : il n'existe pas d'ordre total inter-nœuds (`store_file.rs:116-122`). Dire
      « les entrées d'audit » en pensant « celles de ce nœud » est le mode d'échec que la tâche 1
      existe pour fermer ; le reproduire à l'échelle du déploiement serait la même faute.
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

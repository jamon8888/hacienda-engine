# P4 — File de revue humaine — Plan d'implémentation

> **Pour les agents :** SOUS-COMPÉTENCE REQUISE — utiliser `superpowers:subagent-driven-development`
> (recommandé) ou `superpowers:executing-plans` pour exécuter ce plan tâche par tâche. Les étapes
> utilisent la syntaxe case à cocher (`- [ ]`).

**Objectif :** rendre la file de revue exploitable en HTTP, sans créer une seconde porte vers le
texte en clair à côté de `pii:reveal`.

**Architecture :** cinq routes, des handlers dans un nouveau `hacienda-api/src/handlers/review.rs`,
**deux DTO distincts** — un pour les listes, sans mention ; un pour l'élément unitaire, avec
mention et écriture d'audit. Plus un `overdue` ajouté à `QueueStats`.

**Pile :** Rust 2021, `axum` 0.8, types existants de `hacienda_core::review`. Aucune dépendance
nouvelle.

**Spec :** `superpowers/specs/2026-08-01-P4-human-review-api.md`.

**Programme :** `2026-08-01-hacienda-platform-parity-program.md` §5.4, vague 0.

---

## Vérité terrain — vérifié contre le code, 2026-08-01

**Vérifié en lisant les sources :**

| Fait | Emplacement |
| --- | --- |
| `ReviewQueue` expose `new`, `with_store`, `needs_review`, `submit`, `decide`, `assign`, `list`, `get`, `stats`, `close`, `priority_from_confidence` | `hacienda-core/src/review/queue.rs:43-181` |
| `ReviewQueueItem` porte `id`, `text_snippet`, `category`, `start`, `end`, `confidence`, `source`, `status`, `priority`, `assigned_reviewer`, `created_at`, `deadline`, `decision`, `decided_by`, `decided_at`, `comment` | `review/types.rs:7-24` |
| `ReviewStatus` = `Pending`, `InReview`, `Approved`, `Rejected`, `Modified` | `review/types.rs:28-34` |
| `ReviewDecision` = `Approve` (défaut), `Reject`, `Modify` | `review/types.rs:74-79` |
| `Priority` = `Low`, `Normal`, `High` (défaut), `Critical` | `review/types.rs:52-58` |
| `QueueStats` = `total`, `pending`, `in_review`, `approved`, `rejected`, `modified` — **pas de `overdue`** | `review/types.rs:93-100` |
| `ReviewConfig` = `confidence_threshold` (0.5) et `deadline_hours` (`Some(24)`) — **il n'y a pas de champ `auto_assign`**, contrairement à ce que suggère l'exemple du README racine | `review/types.rs:104-116` |
| `FileReviewStore` est event-sourcé : journalise `Submitted`/`Assigned`/`Decided` et rejoue à l'ouverture | `CHANGELOG.md`, `review/store_file.rs` |
| `HaciendaFacade::review_queue_with_auth` exige `Capability::ReviewDecide` et rend `Option<&ReviewQueue>` | `facade.rs:272-278` |
| `submit_for_review` ne met en file que les détections sous le seuil, et note que le snippet « est la mention du modèle, vide pour les spans regex — déterministes et n'ayant besoin d'aucun contexte humain » | `facade.rs:769-792` |
| Le compteur rendu compte les acceptations du store, pas les tentatives, avec justification écrite | `facade.rs:762-768` |
| `Capability::ReviewDecide` existe | `auth/mod.rs:28-29` |
| `RedactionAction::Reveal` et `record_reveal` existent, écrivant une entrée par span avec `span_hash` | `facade.rs:654-695` |

**Écarts entre la spec et le code, qui deviennent des tâches :**

1. **`QueueStats` n'a pas de `overdue`** — la spec §4 l'exige. Changement de type dans le core.
2. **`ReviewQueue::list` n'est pas paginée** et rend des `ReviewQueueItem` complets, snippet
   compris. La séparation des deux DTO est donc entièrement à faire côté transport.
3. **`ReviewConfig` n'a pas d'`auto_assign`.** La spec ne s'en sert pas ; ne pas l'ajouter.
   Noté ici parce que le README racine le mentionne et induirait en erreur.

**Hypothèse non vérifiée, à confirmer en tâche 1 :** que `deadline` soit peuplé à la soumission
quand `deadline_hours` est `Some`. Si ce n'est pas le cas, `overdue` n'a rien à calculer.

---

## Tâche 1 — Échéances et `overdue`

- [ ] **Étape 1.1** — Lire `review/queue.rs:76-110` et établir si `submit` peuple `deadline`
      depuis `config.deadline_hours`. Écrire la réponse ici avant de coder.
- [ ] **Étape 1.2** — Si ce n'est pas le cas, le faire, avec un test qu'un élément soumis sous
      `deadline_hours = Some(24)` porte une échéance à +24 h.
- [ ] **Étape 1.3** — Ajouter `overdue: usize` à `QueueStats` et le calculer dans `stats()`.
- [ ] **Étape 1.4** — **Ne pas ajouter d'escalade automatique.** La spec l'interdit : une
      escalade qui déciderait à la place d'un humain contredirait l'AI Act Art. 14 qu'elle
      prétend servir. Le dépassement se signale, il ne se résout pas.
- [ ] **Étape 1.5** — Test : un élément dont l'échéance est passée compte dans `overdue` et
      **ne change pas de statut**.

## Tâche 2 — Les deux DTO — le cœur de la spec

C'est la tâche qui protège la porte. `text_snippet` **est** la valeur personnelle dont on doute.

- [ ] **Étape 2.1** — Dans `dto.rs`, définir deux types distincts :

      ```text
      ReviewItemSummaryDto   — tout sauf text_snippet. Rendu par les listes.
      ReviewItemDetailDto    — avec text_snippet. Rendu par GET /v1/review/{id} seulement.
      ```

- [ ] **Étape 2.2** — `ReviewItemSummaryDto` **ne doit pas** avoir de champ `text_snippet`,
      fût-il `Option` ou `#[serde(skip)]`. Un champ absent du type ne peut pas être rempli par
      erreur ; un champ optionnel le peut. Même raisonnement que la suppression faite dans le
      core plutôt qu'au transport (`facade.rs:561`).
- [ ] **Étape 2.3** — `From<ReviewQueueItem>` pour le résumé **abandonne** le snippet ; pour le
      détail, le conserve.
- [ ] **Étape 2.4** — Test : sérialiser un résumé et asserter que le JSON ne contient pas la
      chaîne du snippet — pas seulement que le champ est absent.

## Tâche 3 — Audit sur lecture unitaire

- [ ] **Étape 3.1** — `GET /v1/review/{id}` écrit une entrée d'audit `Reveal` portant le
      `span_hash` blake3 de la mention, en réutilisant la mécanique de `record_reveal`
      (`facade.rs:654-695`) plutôt qu'en la réimplémentant.
- [ ] **Étape 3.2** — Ajouter la méthode sur la façade — `review_item_with_auth(caller, id)` —
      et non dans le handler : la garantie vit dans le core, pour la même raison que partout
      ailleurs dans ce dépôt.
- [ ] **Étape 3.3** — Test : lire un élément, puis asserter qu'une entrée `Reveal` existe dans
      la chaîne, avec le `span_hash` correspondant à celui écrit lors de la rédaction du même
      span. C'est la jointure qui donne sa valeur à l'audit.
- [ ] **Étape 3.4** — Documenter que `review:decide` est une capacité **au moins aussi sensible**
      que `pii:reveal`, dans `auth/mod.rs` et dans la doc de l'API.

## Tâche 4 — Les cinq routes

- [ ] **Étape 4.1** — Créer `hacienda-api/src/handlers/review.rs` sur le patron de
      `handlers/pii.rs`.
- [ ] **Étape 4.2** — Ajouter à `ROUTE_TABLE`, toutes sous `Capability(ReviewDecide)` :

      | Chemin | Méthode | Rend |
      | --- | --- | --- |
      | `/v1/review/queue` | GET | Résumés paginés, filtrés par `status`, `overdue` |
      | `/v1/review/{id}` | GET | Détail — **écrit un audit** |
      | `/v1/review/{id}/assign` | POST | Attribution |
      | `/v1/review/{id}/decision` | POST | `Approve` \| `Reject` \| `Modify` + commentaire |
      | `/v1/review/stats` | GET | `QueueStats` avec `overdue` |

- [ ] **Étape 4.3** — Un identifiant inconnu rend 404 via `ApiError::not_found`, jamais 500.
- [ ] **Étape 4.4** — Une facade sans file configurée rend 404 sur ces routes, pas 500 : c'est
      une absence de ressource, pas une panne.
- [ ] **Étape 4.5** — Vérifier que `every_guarded_route_reflected_in_auth_state` passe — il
      balaie la table, et `/v1/review/{id}` est paramétrée : le test substitue déjà les
      segments `{...}`, ce qui est précisément le skip qui cachait un bug sur `/v1/jobs/{id}`.

## Tâche 5 — Immuabilité des décisions

- [ ] **Étape 5.1** — Vérifier que `decide` sur un élément déjà décidé est refusé ou ajoute un
      événement, mais **ne réécrit pas** l'événement précédent. Lire `review/store_file.rs`
      avant de conclure.
- [ ] **Étape 5.2** — Test : décider, redémarrer le store, décider à nouveau, et asserter que
      l'historique porte les deux événements avec leurs acteurs et horodatages distincts.
- [ ] **Étape 5.3** — Test : aucun élément ne change d'état sans un acteur nommé.

## Tâche 6 — Documentation

- [ ] **Étape 6.1** — Décrire les cinq routes dans le document OpenAPI (chemins au minimum si S4
      n'est pas livré).
- [ ] **Étape 6.2** — Documenter le rattachement à l'AI Act Art. 14 : la supervision humaine
      n'est effective que si un humain peut réellement décider, ce que cette API rend possible.
- [ ] **Étape 6.3** — Entrée `CHANGELOG.md` sous `[Unreleased] / Added`.

---

## Critères de sortie

- [ ] Aucune liste ne rend de mention ; le JSON sérialisé le prouve.
- [ ] Chaque lecture unitaire laisse une entrée d'audit joignable à la rédaction par `span_hash`.
- [ ] Une décision survit à un redémarrage avec son acteur et son horodatage.
- [ ] Aucun élément ne change d'état sans acteur humain nommé.
- [ ] `overdue` est signalé et jamais résolu automatiquement.
- [ ] `cargo clippy --all-targets --all-features -D warnings` et `cargo fmt --check` propres.

## Hors périmètre

Cloisonnement par tenant (→ S1), interface de relecture (Studio), notifications, attribution
automatique — `ReviewConfig` n'a pas de champ pour cela et la spec n'en demande pas.

# P4 — File de revue humaine

**Date :** 2026-08-01
**Statut :** Proposé
**Piste :** P (couche de preuve) · **Vague :** 0
**Programme :** `2026-08-01-hacienda-platform-parity-program.md` §5.4
**Dépend de :** rien (S1 la scope, ne la bloque pas)

---

## 1. Problème

`ReviewQueue` et `ReviewStore` sont complets : `submit`, `assign`, `decide`, `list(Option<ReviewStatus>)`,
`get`, `stats`, `close`. `FileReviewStore` est durable et **event-sourcé** — il journalise
`Submitted` / `Assigned` / `Decided` puis rejoue à l'ouverture, plutôt que de réécrire un fichier
d'état par mutation, « un crash en pleine réécriture perdant tout le fichier, un crash en pleine
addition perdant au pire l'enregistrement en cours ».

`HaciendaFacade::submit_for_review` route déjà les détections sous le seuil vers la file. Et
**aucune route HTTP n'existe** : les décisions ne peuvent être ni consultées ni prises hors du
processus.

Sans surface, la supervision humaine de l'AI Act Art. 14 est une propriété du code que personne
ne peut exercer.

## 2. Surface

| Route | Capacité | Objet |
| --- | --- | --- |
| `GET /v1/review/queue` | `review:decide` | Liste filtrée par `ReviewStatus`, paginée |
| `GET /v1/review/{id}` | `review:decide` | Un élément |
| `POST /v1/review/{id}/assign` | `review:decide` | Attribue à un relecteur |
| `POST /v1/review/{id}/decision` | `review:decide` | `Approve` \| `Reject` \| `Modify` + commentaire |
| `GET /v1/review/stats` | `review:decide` | `QueueStats` |

`ReviewQueueItem` porte déjà tout ce qu'il faut : `text_snippet`, `category`, `start`, `end`,
`confidence`, `source`, `status`, `priority`, `assigned_reviewer`, `created_at`, `deadline`,
`decision`, `decided_by`, `decided_at`, `comment`.

## 3. Décisions

**D-P4-1 — `text_snippet` est du contenu PII et doit être traité comme tel.** C'est le point
sensible de cette spec. Le snippet est *la mention détectée* — c'est-à-dire exactement la valeur
personnelle dont on doute. Le rendre sur `review:decide` seul reviendrait à créer une seconde
porte vers le clair, à côté de `pii:reveal`.

Résolution : **`review:decide` autorise à voir le snippet**, parce qu'un relecteur qui ne voit
pas la mention ne peut pas décider — la revue serait un théâtre. Mais :

- la lecture d'un élément écrit une entrée d'audit `Reveal`, avec le `span_hash` de la mention,
  comme `scan_text_with_auth` sous `SpanText::Include` ;
- `review:decide` est donc une capacité **au moins aussi sensible** que `pii:reveal` et doit
  être documentée comme telle ;
- `GET /v1/review/queue` (la liste) rend les métadonnées **sans** les snippets. Seul
  `GET /v1/review/{id}` les rend, un élément à la fois, et chaque accès est tracé.

Une liste qui déverserait cent mentions en une requête rendrait la traçabilité par span vide de
sens.

**D-P4-2 — les spans regex n'entrent pas en revue.** `submit_for_review` le fait déjà : le
snippet est la mention du modèle, vide pour les spans regex, « déterministes et n'ayant besoin
d'aucun contexte humain ». Une file encombrée d'items qu'aucun humain ne peut améliorer serait
abandonnée, et avec elle la supervision réelle.

**D-P4-3 — une décision est finale et ne se réécrit pas.** `Modify` ajoute un événement ; elle
ne mute pas l'événement précédent. Le store est event-sourcé : la spec en dépend, elle ne le
contourne pas. Un historique de décisions réécrit ne prouve plus rien.

**D-P4-4 — le compteur rendu compte des acceptations du store, jamais des tentatives.** C'est
déjà la règle de `submit_for_review` : « un nombre qui compterait des tentatives dirait à un
opérateur que des éléments sont en file alors qu'il n'y a rien ».

## 4. Échéances et SLA

`ReviewConfig` porte `deadline_hours` (il n'existe pas de champ `auto_assign` : voir le plan
d'implémentation §note). La spec y ajoute :

- `GET /v1/review/queue?overdue=true` — éléments au-delà de leur échéance ;
- `QueueStats` étendu d'un `overdue` ;
- **aucune escalade automatique.** Une escalade qui prendrait une décision à la place d'un humain
  contredirait l'article qu'elle prétend servir. Le dépassement est signalé, jamais résolu.

## 5. Tests

| Test | Assertion |
| --- | --- |
| `queue_list_omits_snippets` | D-P4-1. Le test qui protège la porte. |
| `getting_one_item_writes_a_reveal_audit_entry` | Avec le `span_hash` de la mention. |
| `decision_survives_restart` | Rejeu event-sourcé. |
| `modify_appends_it_does_not_rewrite` | D-P4-3. |
| `regex_spans_never_enter_the_queue` | D-P4-2. |
| `stats_count_store_acceptances_not_attempts` | D-P4-4. |
| `overdue_is_reported_never_auto_decided` | Aucune décision sans acteur humain nommé. |
| `queue_is_scoped_to_the_calling_tenant` | Avec S1. |

## 6. Critères de sortie

- Une décision survit à un redémarrage et conserve son acteur et son horodatage.
- Aucune liste ne rend de mention ; chaque mention lue laisse une entrée d'audit.
- Aucun élément ne change d'état sans un acteur humain nommé.
- Le compteur rendu à l'appelant correspond au contenu réel du store.

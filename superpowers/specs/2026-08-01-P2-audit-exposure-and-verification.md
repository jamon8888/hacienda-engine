# P2 — Exposition et vérification de la chaîne d'audit

**Date :** 2026-08-01
**Statut :** Proposé
**Piste :** P (couche de preuve) · **Vague :** 0
**Programme :** `2026-08-01-hacienda-platform-parity-program.md` §5.2
**Dépend de :** rien (S1 l'améliore, ne le bloque pas)

---

## 1. Problème

`hacienda_core::audit` est complet et testé : `AuditStore` expose `append`, `entries`, `tip`,
`seals`, `verify`, `rotate`, `close` ; `FileAuditStore` est durable, avec reprise d'un segment
laissé ouvert par un processus mort ; `export` rend JSON, JSON-Lines et CSV ; la chaîne est
segmentée, chaînée en blake3, et vérifiée par trois contrôles indépendants.

**Rien de tout cela n'est joignable en HTTP.** La table de routes compte sept entrées, dont
aucune ne touche l'audit. Le métier le plus différenciant du produit est écrit, testé, et
invisible.

C'est le meilleur rapport valeur/effort du dépôt : quelques centaines de lignes de handlers.

## 2. Ce que cela vaut face à Enterprise

Xberg Enterprise expose `GET /v1/audit`. Son schéma est :

```text
AuditEntry { id, actor, action, resource_type, metadata, created_at }
```

— un **journal d'activité** (`"job.submit"`, `"api_key.revoke"`). Aucune chaîne de hachage,
aucun endpoint de vérification (analyse §9.12.3).

Le nôtre porte, **par span rédigé** : `category`, `action`, `span_hash`, `span_length`,
`confidence`, `source`, `pipeline_version`, `config_hash`, `principal`, plus le chaînage. Il
répond à « cette valeur a-t-elle été rédigée, par quel détecteur, sous quelle configuration, et
qui l'a relue depuis » — question à laquelle un journal d'activité ne répond pas.

**La spec doit rendre cette différence lisible dans la documentation de l'API**, pas seulement
dans le code : c'est l'argument de vente.

## 3. Surface

| Route | Capacité | Rend |
| --- | --- | --- |
| `GET /v1/audit/entries` | `audit:read` | Entrées paginées, filtrées, scopées tenant |
| `GET /v1/audit/verify` | `audit:read` | Résultat des trois contrôles, ou la première rupture nommée |
| `GET /v1/audit/seals` | `audit:read` | Sceaux de segments, avec `entry_count` et `seal_hash` |
| `GET /v1/audit/export` | `audit:export` | `json` \| `jsonl` \| `csv` (`ExportFormat`) |
| `GET /v1/audit/tip` | `documents:process` | Tête de chaîne courante |

**Décision D-P2-1 — `tip` n'est pas gardée par `audit:read`.** La tête est un hachage opaque
qui ne révèle rien des entrées derrière lui. La garder empêcherait un appelant porteur de
`documents:process` d'obtenir la preuve de chaîne de **son propre** résultat. C'est déjà la
règle appliquée par `HaciendaFacade::audit_tip`, et elle est documentée comme telle.

**Décision D-P2-2 — `verify` est un endpoint, pas un champ.** Un booléen `verified: true` dans
une réponse n'est qu'une affirmation du serveur. Un endpoint dédié qui, en cas d'échec, **nomme
l'entrée ou le sceau fautif** est vérifiable par un auditeur. C'est ce que `AuditError` fournit
déjà.

**Décision D-P2-3 — `entries` est paginé et n'accepte pas de tri arbitraire.** Ordre de chaîne
uniquement. Un tri par colonne inviterait à traiter le journal comme une table ; il est une
séquence, et son ordre porte du sens.

## 4. Filtres

`from`, `to` (horodatage), `category`, `action`, `principal`, `limit`, `cursor`.

**Décision D-P2-4 — pagination par curseur, jamais par offset.** La chaîne ne fait que croître ;
un offset dérive dès qu'une entrée est ajoutée pendant la pagination, et un auditeur qui saute
une entrée sans le savoir est exactement le défaut que ce module existe pour éviter.

## 5. Export

`ExportFormat::{Json, JsonLines, Csv}` existe. L'export est **streamé**, pas construit en
mémoire : une chaîne de conformité couvre des mois.

**Décision D-P2-5 — l'export porte les sceaux avec les entrées.** Un export d'entrées sans les
sceaux ne se vérifie pas hors ligne, ce qui est le principal cas d'usage (remise à un
régulateur, à un auditeur externe). Le format enveloppe les deux.

## 6. Zéro PII dans le journal — à re-vérifier ici

Le journal ne contient jamais de valeur : seulement `span_hash` (blake3), `span_length` et la
catégorie. C'est une propriété du modèle existant, et l'exposition HTTP est précisément le
moment où une régression deviendrait publique.

**Test obligatoire :** un corpus témoin traité, puis chaque endpoint de cette spec interrogé,
et aucune réponse ne contient une valeur du corpus.

## 7. Tests

| Test | Assertion |
| --- | --- |
| `no_endpoint_returns_corpus_plaintext` | §6. Le test qui compte. |
| `verify_names_the_broken_entry` | Une entrée altérée sur disque → 200 avec l'identifiant fautif, pas un 500 opaque. |
| `export_round_trips_offline` | Export récupéré, vérifié hors du serveur, sceaux compris. |
| `cursor_pagination_skips_nothing_under_concurrent_append` | Écriture pendant la pagination. |
| `tip_is_reachable_with_documents_process_alone` | D-P2-1. |
| `export_requires_audit_export_not_audit_read` | Les deux capacités sont distinctes. |
| `entries_are_scoped_to_the_calling_tenant` | Avec S1. |

## 8. Critères de sortie

- Un client prouve qu'une entrée n'a pas été altérée **sans accès au stockage**.
- Une altération fait échouer `verify` en nommant l'entrée ou le sceau.
- Un export se vérifie hors ligne.
- Aucune réponse ne contient de valeur du corpus témoin.

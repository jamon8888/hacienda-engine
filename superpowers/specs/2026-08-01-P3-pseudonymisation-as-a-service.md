# P3 — Pseudonymisation réversible comme service

**Date :** 2026-08-01
**Statut :** Proposé
**Piste :** P (couche de preuve) · **Vague :** 2
**Programme :** `2026-08-01-hacienda-platform-parity-program.md` §5.3
**Dépend de :** S1 (espace de clés par tenant) · **Utilisée par :** E2 (diff sur jetons)

---

## 1. Problème

`Pseudonymiser` est le différenciateur technique le plus fort du dépôt : AES-256-SIV (RFC 5297)
sur la valeur normalisée NFKC, catégorie PII en donnée associée authentifiée, jetons
déterministes — la même valeur donne le même jeton d'un processus à l'autre — et rotation
additive, l'identifiant de clé voyageant dans le jeton (`[EMAIL:k1:MZXW6YTB...]`).

Il n'a **aucune surface API**. Le seul chemin est la configuration d'un mode de rédaction.

Xberg Enterprise n'a rien d'équivalent : `RedactionStrategy` y vaut `mask | hash |
token_replace | drop`, et `token_replace` émet un `replacement_token` **sans aucun chemin de
révélation** — zéro occurrence de `pseudonym` dans les 317 Ko de leur spec (analyse §9.12.3).

## 2. Ce que le déterminisme achète

Trois capacités qu'aucun concurrent examiné ne propose :

1. **Co-référence inter-documents.** Un lecteur suit un même sujet à travers un corpus sans
   jamais voir sa valeur. C'est ce qui fait fonctionner les bundles à graphe de Studio.
2. **Diff exact sur contenu rédigé** (→ E2). Une même valeur porte le même jeton d'une version
   à l'autre : le diff est juste sans manipuler de clair.
3. **Droits RGPD sur corpus rédigé.** Un droit d'accès ou d'effacement se résout en calculant
   le jeton de la valeur demandée et en cherchant *ce jeton*, sans déchiffrer le corpus.

## 3. Surface

| Route | Capacité | Objet |
| --- | --- | --- |
| `POST /v1/pseudonyms/reveal` | `documents:process` **+** `pii:reveal` | Jetons → valeurs |
| `POST /v1/pseudonyms/token` | `documents:process` | Valeurs → jetons (sans révélation) |
| `GET /v1/keys` | `audit:read` | Identifiants et états, **jamais** de matériel |
| `POST /v1/keys/rotate` | administrateur | Promeut une nouvelle clé active |

**Décision D-P3-1 — `token` existe et n'exige pas `pii:reveal`.** C'est ce qui rend le droit
d'accès praticable : un opérateur calcule le jeton d'une valeur qu'il connaît déjà, puis cherche
ce jeton. Il n'apprend rien qu'il ne sache. La direction inverse — jeton vers valeur — est la
seule qui divulgue, et elle porte `pii:reveal`.

**Décision D-P3-2 — `reveal` écrit une entrée d'audit par jeton, pas par appel.** Même règle que
`record_reveal` aujourd'hui : chaque entrée porte le `span_hash` du span révélé, celui-là même
que la rédaction a inscrit. C'est ce qui permet à un auditeur de joindre « cette valeur a été
rédigée ici » à « et ce principal l'a lue là ». Une entrée par appel, hachant la concaténation,
ne répondrait ni à l'une ni à l'autre.

**Décision D-P3-3 — aucune réponse ne contient de matériel de clé.** `GET /v1/keys` rend des
identifiants, des états (`active` / `retired`), des dates. `RedactionConfig::key_id` est déjà un
*identifiant* et non une clé, précisément pour que le matériel n'atteigne jamais `config show`,
les journaux, ni un bundle de support. L'API tient la même ligne.

## 4. Rotation

Additive. Une nouvelle clé devient active ; les précédentes passent `retired` et restent
révélables tant qu'elles sont déclarées. L'identifiant voyageant dans le jeton, aucun corpus
n'a besoin d'être réécrit.

**Décision D-P3-4 — une clé retirée ne peut pas être supprimée par l'API.** La supprimer rendrait
irréversible, silencieusement, tout corpus émis sous elle — et la découverte se ferait lors d'un
droit d'accès. La suppression est une opération d'exploitation délibérée, hors API, documentée
comme destructive.

**Décision D-P3-5 — la rotation ne re-dérive rien.** Elle n'invalide aucun jeton existant. Un
client voulant réellement re-pseudonymiser un corpus fait une ré-ingestion, qui est une
opération de données, pas de clés.

## 5. Cloisonnement par tenant

Dépend de S1 §4. Chaque tenant a son matériel, résolu indépendamment. **Deux tenants portant la
même valeur obtiennent deux jetons différents.** Sans cela, un client peut tester l'appartenance
d'une valeur au corpus d'un autre — c'est le défaut que S1 existe pour fermer, et P3 en est le
principal bénéficiaire.

## 6. Erreurs

| Cas | Réponse |
| --- | --- |
| Jeton malformé | 400, sans écho du jeton |
| Clé inconnue dans un jeton | 422, nommant l'identifiant de clé — pas le jeton |
| Clé connue mais non chargée | 503 : l'exploitation peut la charger ; ce n'est pas une erreur du client |
| `pii:reveal` absent | 403, **avant** toute tentative de déchiffrement |

**Décision D-P3-6 — un jeton d'un autre tenant est « clé inconnue », pas « interdit ».** Même
logique que le 404 de S1 : répondre 403 confirmerait que le jeton est valide ailleurs.

## 7. Tests

| Test | Assertion |
| --- | --- |
| `same_value_same_token_across_processes` | La propriété fondatrice. |
| `two_tenants_same_value_different_tokens` | Avec S1. |
| `retired_key_still_reveals` | Rotation additive. |
| `reveal_writes_one_audit_entry_per_token` | Et le `span_hash` correspond à celui de la rédaction. |
| `no_response_contains_key_material` | Balayage de tous les endpoints de la spec. |
| `reveal_without_capability_is_403_before_decryption` | Ordre des contrôles. |
| `cross_tenant_token_is_unknown_key_not_forbidden` | D-P3-6. |
| `malformed_token_error_does_not_echo_the_token` | Pas de fuite par message d'erreur. |

## 8. Critères de sortie

- Une valeur donne le même jeton à travers un corpus, deux processus et deux redémarrages.
- Une clé retirée reste révélable ; deux tenants ne partagent aucun jeton.
- Aucune réponse, aucun journal, aucun message d'erreur ne contient de matériel de clé.
- Un droit d'accès se résout sans déchiffrer le corpus, en passant par `token`.

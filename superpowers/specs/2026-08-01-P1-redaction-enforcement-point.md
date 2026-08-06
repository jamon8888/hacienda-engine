# P1 — Point d'application unique de la rédaction

**Date :** 2026-08-01
**Statut :** Proposé
**Piste :** P (couche de preuve) · **Vague :** 1 · **Chemin critique :** oui
**Programme :** `2026-08-01-hacienda-platform-parity-program.md` §5.1
**Dépend de :** S1 · **Bloque :** E0, E2, E3, E4
**Éprouvée par :** E0 avant que E4 ne s'y appuie (décision D1)

---

## 1. Problème

À parité fonctionnelle, hacienda gagne trois stockages qu'elle n'a pas aujourd'hui : un store
vectoriel (E4), un store de documents versionnés (E2), un store d'objets (E3). Chacun est une
occasion nouvelle d'écrire du texte non rédigé.

L'invariant I1 du programme — *aucun texte non rédigé ne franchit une frontière de
persistance* — ne peut pas reposer sur la discipline des appelants. Le dépôt a déjà tranché ce
type de question une fois, et bien : la suppression du texte de span est faite dans le core
(`facade.rs:561`) et non au transport, avec la justification écrite que « chaque futur appelant
— FFI, CLI, un second transport HTTP — n'a pas à la réimplémenter, l'un d'eux oubliera ».

Cette spec applique le même raisonnement aux stockages. La garantie doit être **structurelle** :
il ne doit pas exister de chemin compilable qui écrive dans un store sans passer par la
rédaction.

## 2. Objectifs / Non-objectifs

**Objectifs**

- Un garde générique enveloppant tout store de contenu.
- Impossibilité, hors du crate, d'obtenir un store non gardé.
- Journalisation d'audit de toute révélation en lecture.

**Non-objectifs**

| Différé | Raison |
| --- | --- |
| Les stores eux-mêmes | → E2, E3, E4 |
| La politique de rédaction | Existe : `RedactionConfig`, profils PCI/HIPAA/GDPR |
| Le chiffrement au repos | Propriété du backend, pas du garde |

## 3. Forme

```text
Guard<S>  où S ∈ { VectorStore, DocumentStore, BlobStore }

  écriture  → détecte → rédige/pseudonymise → PUIS délègue à S
  lecture   → délègue à S → PUIS journalise toute révélation de span
  reste     → délégation transparente
```

Le garde est **générique sur le backend**, pas spécifique à un store. Un client peut donc
choisir son backend — mémoire, SQLite, pgvector, le sien — sans pouvoir contourner la rédaction
ni l'audit. C'est ce qui rend le produit défendable commercialement autant que techniquement.

## 4. Décisions

**D-P1-1 — le type non gardé n'est pas exportable.** Les implémentations concrètes de store
sont `pub(crate)`. Seul le garde est `pub`. Un embarqueur ne peut pas obtenir un store nu, même
en le voulant.

*Conséquence assumée :* un client qui voudrait légitimement un store non rédigé (corpus déjà
anonymisé en amont) ne peut pas. C'est délibéré — c'est un produit de conformité, et l'échappatoire
serait utilisée par défaut. Le cas se traite par un profil de rédaction vide, explicitement
configuré et audité, jamais par un contournement du type.

**D-P1-2 — rédiger avant, pas après.** L'écriture rédige puis délègue. L'ordre inverse — écrire
puis nettoyer — laisse une fenêtre où le clair est dans le backend, et cette fenêtre survit à un
crash entre les deux.

**D-P1-3 — un échec de rédaction échoue l'écriture.** Jamais de dégradation vers « écrit non
rédigé avec un avertissement ». C'est la règle que la façade applique déjà : « les échecs de
détection ne sont jamais rétrogradés en résultats partiels ».

**D-P1-4 — la lecture journalise, elle ne bloque pas.** Une lecture rend ce que le store
contient, qui est déjà rédigé. Le seul événement auditable en lecture est la révélation de
span, sous `Capability::PiiReveal`, qui écrit une entrée `Reveal` — même mécanique que
`scan_text_with_auth`, avec le même `span_hash` blake3 pour joindre rédaction et révélation.

**D-P1-5 — le garde porte le `TenantCtx`.** Il le transmet au store *et* au résolveur de clés :
la rédaction d'un tenant utilise l'espace de tokens de ce tenant (S1).

## 5. Le cas particulier de l'écriture indirecte

E3 (uploads présignés) écrit **directement** dans le stockage objet, sans traverser le serveur
d'API — donc sans traverser le garde. C'est structurel, pas un oubli.

Résolution, spécifiée en E3 et rappelée ici parce qu'elle est une exception à I1 : l'objet
déposé est marqué `quarantine`, **illisible par tout endpoint**, jusqu'à ce que `confirm`
déclenche son passage par la façade. L'original en clair est détruit après traitement.

**Le garde doit donc distinguer deux états de stockage** : `quarantine` (écrit, non lisible) et
`clean` (rédigé, lisible). Aucun chemin de lecture ne rend un objet en quarantaine.

## 6. Tests

| Test | Assertion |
| --- | --- |
| `bare_store_is_not_constructible_outside_the_crate` | **Test de compilation** (`trybuild`). C'est le test central : il échoue en compilant, pas en s'exécutant. |
| `control_corpus_never_appears_in_backend` | Pour chaque chemin d'écriture, un corpus témoin inséré ne laisse aucune occurrence en clair dans le stockage sous-jacent. |
| `redaction_failure_fails_the_write` | Détecteur en échec injecté → l'écriture retourne `Err` et le backend est inchangé. |
| `reveal_writes_one_audit_entry_per_span` | Et le `span_hash` correspond à celui écrit lors de la rédaction. |
| `reveal_without_capability_is_403` | Avant tout accès au store. |
| `quarantined_object_is_unreadable_by_every_endpoint` | Balayage de la table de routes. |
| `guard_uses_the_calling_tenant_keyspace` | Deux tenants → deux tokens pour la même valeur. |

**Le test `control_corpus_never_appears_in_backend` doit inspecter le backend directement**,
pas passer par l'API du store : c'est la différence entre vérifier la rédaction et vérifier
qu'on ne la voit pas.

## 7. Critères de sortie

- Le test de compilation prouve qu'aucun store nu ne s'obtient hors du crate.
- Le corpus témoin ne laisse aucune trace en clair, sur chaque backend implémenté.
- E0 a exercé le garde contre un store tiers réel (décision D1 du programme) **avant** que E4
  ne s'y appuie.

# S4 — Contrat OpenAPI et clients générés

**Date :** 2026-08-01
**Statut :** Proposé
**Piste :** S (socle) · **Vague :** 1
**Programme :** `2026-08-01-hacienda-platform-parity-program.md` §4
**Dépend de :** rien · **Consommée par :** E0 et tout client

---

## 1. Problème

Deux écarts, dont l'un est un risque de crédibilité immédiat.

**Le document OpenAPI est squelettique.** `hacienda-api/src/handlers/openapi.rs` construit bien
un document 3.1 dérivé de `ROUTE_TABLE`, à la requête plutôt qu'au démarrage « pour qu'aucune
copie périmée ne soit possible », et un test (`openapi_path_set_equals_route_table`) interdit la
dérive de l'ensemble des chemins. C'est une bonne couture. Mais chaque chemin ne porte qu'un
`description: "Access: {:?}"` — ni opérations, ni schémas de corps, ni réponses typées, ni
erreurs. Aucun générateur de client ne peut en tirer quoi que ce soit.

**Le README annonce 14 bindings qui n'existent pas.** `packages/` est absent et `alef.toml`
référence quatre fichiers sources inexistants (`hacienda/src/cli.rs`, `hacienda/src/api.rs`,
`hacienda-core/src/mcp/server.rs`, `hacienda-core/src/cli_overrides.rs`). Un prospect qui tente
`pip install hacienda` après lecture du README repart avec une impression durable.

## 2. La bonne cible : des clients HTTP, pas des bindings natifs

L'amont distingue deux modèles (analyse §9.12.1) : `xberg-io/xberg` `packages/` livre des
**bindings natifs** générés par alef depuis le Rust ; `xberg-io/sdks` livre des **clients HTTP**
générés depuis OpenAPI 3.1 par `openapi-python-client`, `openapi-typescript` et `oapi-codegen`,
versionnés indépendamment.

Le produit hacienda est **un serveur d'API** : `hacienda-cli serve`, une table de routes,
`/openapi.json`. Ses clients parleront HTTP. Générer des clients depuis OpenAPI est nettement
moins coûteux que réparer alef pour 14 cibles natives, et sert le produit tel qu'il est.

**Décision D-S4-1 — les bindings natifs ne sont pas abandonnés, ils sont découplés.** Ils gardent
leur sens pour Studio (WASM) et pour un embarqueur voulant du zéro-egress en process. Ils
deviennent une offre distincte, hors du chemin critique.

## 3. Étoffer le document

Le document reste **dérivé de `ROUTE_TABLE`** — c'est l'invariant I4 du programme. `RouteSpec`
s'étend :

```rust
pub struct RouteSpec {
    pub path: &'static str,
    pub access: Access,
    pub(crate) make_router: fn() -> MethodRouter<ApiState>,
    /// Nouveau : opérations, schémas de requête et de réponse, erreurs.
    pub(crate) describe: fn() -> OperationSpec,
}
```

**Décision D-S4-2 — la description vit dans la table, pas à côté.** Un document décrit dans un
fichier séparé dérive de la table qu'il prétend décrire ; c'est précisément ce que la table
existe pour empêcher. Ajouter une route sans la décrire doit être impossible.

**Décision D-S4-3 — le test anti-dérive est renforcé.** `openapi_path_set_equals_route_table`
vérifie aujourd'hui l'ensemble des chemins. Il doit vérifier en plus que **chaque route porte au
moins une opération, un schéma de réponse et son enveloppe d'erreur**. Une route sans schéma
échoue la CI.

**Décision D-S4-4 — les schémas d'erreur sont normatifs.** L'enveloppe existe déjà
(`{"code": "payload_too_large", ...}`, testée). Elle est décrite dans le document, et chaque
route déclare les codes qu'elle peut rendre — 401, 403, 404, 413, 422, 429, 503. Un client
généré qui ne connaît pas les erreurs oblige chaque utilisateur à les redécouvrir.

## 4. Clients générés

| Langage | Générateur | Paquet |
| --- | --- | --- |
| Python | `openapi-python-client` (httpx) | `hacienda-sdk` (PyPI) |
| TypeScript | `openapi-typescript` (openapi-fetch) | `@hacienda/sdk` (npm) |
| Go | `oapi-codegen` | `github.com/jamon8888/hacienda-engine/packages/go` |

**Décision D-S4-5 — versionnés indépendamment du cœur.** Un client doit pouvoir corriger un bug
sans release du moteur. C'est le modèle de `xberg-io/sdks` (0.3.1 face à un cœur 1.0.2), et il
est correct pour un client HTTP.

**Décision D-S4-6 — la génération est en CI, et sa dérive échoue la CI.** Les clients sont
regénérés et comparés ; un document modifié sans regénération casse la construction. Sans cela
les clients divergent silencieusement, ce qui est pire que ne pas en avoir.

## 5. Le README

**Décision D-S4-7 — alignement avant toute démarche commerciale.** Deux options, et il faut en
choisir une explicitement :

- **soit** réparer `alef.toml` (corriger les quatre sources fantômes) et générer réellement les
  bindings natifs annoncés ;
- **soit** passer les 13 lignes non générées en 🚧 et ne laisser ✅ que WASM, seul binding
  réellement construit (`crates/hacienda-wasm`, testé sur wasm32).

La seconde est recommandée : elle est immédiate et honnête, et n'engage pas quatorze cibles avant
d'avoir un client. Les nouveaux clients HTTP s'ajoutent au tableau à mesure qu'ils existent.

## 6. Tests

| Test | Assertion |
| --- | --- |
| `every_route_has_an_operation_and_a_response_schema` | D-S4-3. Échoue la CI sur une route non décrite. |
| `every_route_declares_its_error_codes` | D-S4-4. |
| `openapi_document_validates_against_3_1_schema` | Validation formelle. |
| `generated_clients_are_up_to_date` | D-S4-6. Régénération et comparaison. |
| `each_generated_client_exercises_every_endpoint` | Tests d'intégration contre un serveur réel. |
| `readme_binding_table_matches_reality` | Test lisant le README et vérifiant que chaque ✅ correspond à un artefact publié. |

Le dernier test est inhabituel et il est délibéré : c'est la régression qui a produit l'écart
actuel, et rien d'autre ne l'aurait attrapée.

## 7. Critères de sortie

- Le document OpenAPI décrit chaque route avec opérations, schémas et erreurs, et valide contre
  le schéma 3.1.
- Trois clients générés exercent chaque endpoint en intégration.
- Une route ajoutée sans description échoue la CI.
- Le tableau des bindings du README correspond à ce qui est réellement publié.

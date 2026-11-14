# Enquête LanceDB comme backend RAG complet — Wayfinder #130

**Date :** 2026-11-14<br>
**Statut :** recherche terminée, décision d’architecture proposée<br>
**Périmètre :** client Tauri local/offline, Hacienda Engine, service on-premise pour équipe,
documents tokenisés et dossiers soumis au secret professionnel.

## Verdict

LanceDB est un très bon moteur de données et de recherche RAG pour Hacienda, mais ce n’est
pas à lui seul un backend complet de confidentialité ou un produit RAG complet.

La recommandation est donc **complémentaire et progressive** :

1. utiliser LanceDB Embedded comme data plane local/offline pour les versions token-only,
   avec le coffre chiffré séparé pour les originaux et les correspondances de réhydratation ;
2. conserver le contrat [`RagStore`](../../crates/hacienda-rag/src/store.rs) et implémenter
   un adaptateur `LanceDbStore`, sans supprimer `PgVectorStore` avant les tests de parité ;
3. pour le mode équipe, utiliser LanceDB remote/Enterprise uniquement après validation de
   la disponibilité on-premise, du contrat commercial, de l’authentification et du modèle
   de stockage ; le gateway Hacienda reste le plan de contrôle obligatoire ;
4. ne jamais exposer LanceDB directement à Claude, Codex ou MCP. Le gateway décide du tenant,
   de l’ACL, de la redaction, de l’egress et de la réhydratation.

Cette conclusion répond à la question « LanceDB remplace-t-il le backend complet ? » :
**non pour la sécurité et le domaine Hacienda ; oui comme candidat sérieux pour le data
plane RAG local, et éventuellement comme data plane d’équipe.**

## Ce que LanceDB couvre réellement

La documentation officielle présente LanceDB comme une base vectorielle persistante, sans
serveur à gérer en mode embedded, avec stockage local ou objet et des clients Rust, Python
et JavaScript/TypeScript. Le crate Rust expose aussi des modules pour les rerankers, les blobs,
le remote et l’autorisation de déploiement.

Sources : [documentation Rust LanceDB](https://docs.rs/lancedb/latest/lancedb/index.html),
[API Python LanceDB](https://lancedb.github.io/lancedb/python/),
[connexion JavaScript](https://lancedb.github.io/lancedb/js/functions/connect/).

### Matrice de capacité

| Besoin Hacienda | LanceDB | Conclusion pour Hacienda |
|---|---|---|
| Recherche vectorielle dense | Oui | Compatible avec `RetrieveMode::Vector`. |
| Recherche lexicale/BM25 | Oui, index FTS Lance/LanceDB | Compatible avec `FullText`, sur texte token-only uniquement. |
| Recherche hybride | Oui, fusion vectorielle + FTS et reranking RRF | Compatible avec `Hybrid`; mesurer le comportement sur le corpus réel. |
| Reranking | Oui, API de reranker et fusion hybride | Garder la politique de score/candidate pool dans l’adaptateur et les tests. |
| Filtres de métadonnées | Oui, filtres SQL/colonnes | Ne remplace pas l’ACL document/chunk ni l’isolation de tenant. |
| Embeddings | Fonctions/providers disponibles selon le client | Le pipeline Hacienda doit décider quand et sur quel texte embedder. |
| Chunking, extraction Office/PDF, classification PII | Non, pas un remplacement de Hacienda | À garder dans l’ingestion Hacienda. |
| Sparse/SPLADE | Non confirmé comme capacité native de première classe dans la documentation officielle étudiée | Retourner `UnsupportedMode` au départ ou créer un adaptateur spécialisé. |
| Late interaction/ColBERT | Non confirmé comme mode natif complet équivalent au contrat Hacienda | Ne pas annoncer `LateInteraction` sans preuve et benchmark. |
| Versions de tables | Oui, versions/optimisation et opérations de table | Ne remplace pas les versions de dossier et l’audit append-only Hacienda. |
| Upsert idempotent | Oui, `merge_insert`/update selon le client | Utiliser une clé composite et tester les écritures concurrentes. |
| Blobs/binaires | Oui, colonnes blob dédiées | Pour un original sensible, préférer le coffre séparé ; jamais d’indexation du blob en clair. |
| Local/offline | Oui, particulièrement adapté à l’embedded | Bon choix pour le client individuel et les files de synchronisation. |
| Remote/team | Oui via remote/Enterprise et `db://` selon l’offre | Valider on-premise, licences, réseau, auth et reprise avant engagement. |
| Autorisation | ACL de déploiement/table documentée côté catalog/remote | Insuffisant pour l’ACL par tenant, dossier, version et chunk. |

Les documents officiels décrivent la recherche FTS avec index et BM25, les filtres, la
recherche hybride et le reranking. Ils ne permettent pas de conclure que les modes
SPLADE/sparse et ColBERT/late-interaction du contrat Hacienda sont prêts à l’emploi dans
LanceDB ; ces modes doivent rester optionnels tant qu’un adaptateur et des tests ne les
valident pas.

Sources : [Full-text search](https://github.com/lancedb/docs/blob/main/docs/search/full-text-search.mdx),
[rerankers Rust](https://docs.rs/lancedb/latest/lancedb/rerankers/index.html),
[blob columns Rust](https://docs.rs/lancedb/latest/lancedb/blob/index.html).

## Comparaison avec le backend actuel

Le dépôt possède déjà une abstraction backend neutre. [`RagStore`](../../crates/hacienda-rag/src/store.rs)
est volontairement single-tenant et couvre les collections, upserts, suppressions, filtres,
retrieval, statistiques et migration d’embeddings. [`RetrieveQuery`](../../crates/hacienda-rag/src/query.rs)
prévoit déjà les modes vectoriel, full-text, hybride, sparse et late-interaction.

Le backend actuel `PgVectorStore` fournit dense, full-text et hybride, avec des champs JSONB
pass-through pour les futurs modes avancés. Il reste un bon backend d’équipe lorsque
Postgres est déjà la plateforme opérationnelle et que les ACL, migrations, transactions et
observabilité SQL sont prioritaires.

LanceDB ne doit donc pas être introduit en remplaçant le contrat : il doit être un deuxième
backend derrière ce contrat. La comparaison à faire en prototype est la suivante :

| Critère | LanceDB Embedded | PgVector actuel | LanceDB remote/Enterprise |
|---|---|---|---|
| Local/offline | Très favorable | Peu adapté | Non pertinent |
| Dense + FTS + hybride | Favorable | Déjà disponible | Favorable, à valider |
| Sparse/late interaction | À prouver | Non exposé actuellement | À prouver |
| Transactions métier Hacienda | À encapsuler | Mature via Postgres | À valider selon offre |
| ACL fine par dossier/chunk | À construire dans Hacienda | À construire dans Hacienda | ACL catalogue ne suffit pas seule |
| Déploiement équipe on-premise | Stockage partagé à concevoir | Déjà naturel | Dépend de l’offre Enterprise |
| Réversibilité | Bonne si `RagStore` est conservé | Déjà la référence | Bonne si schéma token-only stable |

## Architecture recommandée

```text
Tauri local / Hacienda Gateway
  ├── politique tenant + ACL + audit + egress
  ├── redaction/re-hydratation et coffre de clés
  ├── extraction/chunking et embeddings contrôlés
  ├── RagStore adapter
  │     ├── LanceDbStore (local/offline ou data plane d’équipe)
  │     └── PgVectorStore (serveur actuel / repli)
  └── Claude/Codex uniquement via contenu token-only validé

LanceDB token-only
  ├── content/search_text/embeddings tokenisés
  ├── tenant_id + dossier/version/chunk + classification non secrète
  └── index vectoriel + FTS + fusion/reranking

Coffre séparé chiffré
  ├── original bytes
  ├── token -> valeur brute
  └── clés/références/capacités de viewer
```

LanceDB peut stocker des blobs, mais un blob chiffré ne doit pas être considéré comme une
autorisation de réhydratation. La clé, la capacité viewer et la décision de divulgation
restent dans Hacienda. La table de recherche ne doit contenir aucune PII brute, même dans
une colonne non utilisée par la requête : cela simplifie la preuve d’absence d’egress,
l’indexation, les sauvegardes et les logs.

Pour le service d’équipe, la variante recommandée est :

- gateway Hacienda devant LanceDB ;
- un namespace ou une table par trust domain si cela aide l’isolation opérationnelle ;
- `tenant_id`, `dossier_id`, `version_id` et ACL effective dans les lignes token-only ;
- préfiltre d’autorisation avant le scoring, et contrôle de second niveau après retrieval ;
- journal d’audit Hacienda indépendant des versions LanceDB ;
- synchronisation idempotente et reconciliation après perte réseau.

L’autorisation documentée par LanceDB concerne le déploiement/catalogue et les privilèges de
table. Elle ne doit pas être interprétée comme une ACL métier automatique par ligne pour les
tenants et dossiers Hacienda.

Source : [module d’autorisation LanceDB](https://docs.rs/lancedb/latest/lancedb/authz/index.html).

## Contrat token-only et RAG complet

Le RAG complet de Hacienda comprend davantage que la base de retrieval :

1. import et extraction contrôlée des formats ;
2. détection et décision de redaction, y compris les ajouts manuels Tiptap ;
3. création de version tokenisée et de chunks ;
4. embeddings calculés sur la représentation tokenisée ;
5. index vectoriel/FTS et filtres autorisés ;
6. retrieval, fusion, reranking et citations token-only ;
7. synthèse par Claude/Codex via le gateway ;
8. réhydratation uniquement dans l’interface autorisée, jamais dans le prompt fournisseur ;
9. audit, conservation, purge et preuve de non-egress.

LanceDB couvre surtout les étapes 4 et 5, une partie de 6, et des primitives de stockage
pour 3/9. Il ne couvre pas à lui seul les étapes 1, 2, 7, 8 et la politique de 9.

Cela s’intègre au contrat éditeur/viewer déjà écrit dans
[`spec #129`](../superpowers/specs/2026-11-14-wayfinder-129-editor-viewer-lancedb-contract.md) :
Tiptap persiste le JSON canonique token-only, LanceDB indexe les projections token-only et
le viewer humain garde un chemin séparé vers l’original.

## Risques et décisions bloquantes

### 1. Isolation multi-tenant

Le trait actuel est single-tenant par conception. Un `LanceDbStore` ne doit pas prendre un
`tenant_id` arbitraire depuis la requête : le gateway doit injecter une instance ou un
contexte scoped, puis appliquer une ACL indépendante. Le simple filtre SQL ne suffit pas si
un chemin d’API oublie le filtre.

### 2. Recherche sur les tokens

Les tokens doivent être stables et suffisamment sémantiques pour préserver l’utilité du RAG,
mais ne doivent pas permettre d’inférer la valeur brute. Les benchmarks doivent mesurer le
compromis entre `[PERSON:k1]`, classes de token, pseudonymes cohérents et masques génériques.

### 3. Embedding et fournisseurs

Les fonctions d’embedding documentées par LanceDB peuvent intégrer des providers, mais les
clés et la destination doivent être gouvernées par Hacienda. Le choix par défaut est un
embedding local ou un endpoint explicitement allowlisté ; aucune fonction d’embedding ne doit
recevoir l’original non tokenisé.

### 4. Versions et réhydratation

Les versions LanceDB sont une primitive de stockage, pas la preuve de l’historique légal.
L’audit Hacienda doit enregistrer l’auteur, la décision de redaction, la politique, le hash,
la capacité de réhydratation et la raison de divulgation.

### 5. Remote/on-premise

La documentation publique distingue l’embedded de l’Enterprise/Cloud remote. La possibilité
exacte d’un déploiement sécurisé Hacienda on-premise, son modèle de support, les régions, la
gestion des clés et les garanties de suppression doivent être confirmées contractuellement
avant de choisir LanceDB comme backend d’équipe unique.

Source : [Enterprise quickstart](https://docs.lancedb.com/enterprise/quickstart).

## Plan de validation avant implémentation

1. Écrire un jeu de tests de parité `RagStore` sur le backend mémoire, PgVector et le futur
   adaptateur LanceDB : upsert idempotent, filtres, suppression, version, `top_k`, scores,
   groupement par document et refus des modes non supportés.
2. Construire un corpus synthétique de dossiers avec PII, faux tokens, homonymes, documents
   multilingues et pièces jointes.
3. Mesurer recall@k, nDCG, latence, taille, coût d’index, reconstruction après crash et
   comportement de `merge_insert` concurrent.
4. Tester explicitement qu’aucune valeur brute ne se retrouve dans LanceDB, ses index FTS,
   ses embeddings, les snapshots, les traces ou les erreurs.
5. Tester les scénarios d’ACL négatifs : tenant voisin, ancienne version, chunk supprimé,
   viewer sans capacité, et synchronisation hors ligne rejouée.
6. Décider après le prototype si LanceDB devient le backend local officiel et si le backend
   d’équipe reste PgVector ou passe à LanceDB remote/Enterprise.

## Décision proposée au Wayfinder

**LanceDB est accepté comme candidat data plane RAG token-only, recommandé pour le mode local
et conservé derrière `RagStore`. Il n’est pas accepté comme remplacement autonome du gateway,
du coffre, de l’audit, de la politique multi-tenant ou de PgVector sans prototype de parité et
validation on-premise.**

Cette décision permet d’avancer sur l’éditeur Tiptap et le viewer sans coupler prématurément
la sécurité métier à un moteur de base de données.

## Addendum d’implémentation — 2026-11-15

La recherche lexicale native BM25 LanceDB et la fusion hybride des candidats sont maintenant
intégrées (`487691a`), derrière `RagStore`. Le flux LocalFull écrit aussi les projections
token-only et les nouvelles versions dans LanceDB (`6e6f9eb`). Cela ne transforme pas
LanceDB en backend produit complet : l’autorisation, le coffre de réhydratation, l’audit,
les identités durables après redémarrage et le service multi-tenant restent des couches
distinctes à livrer et vérifier.

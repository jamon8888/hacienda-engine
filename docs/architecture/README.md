# Architecture et programme produit — point d'entrée

**Dernière mise à jour :** 2026-08-01

Ce dossier contient l'analyse d'architecture de `hacienda-engine`, la lecture concurrentielle
de l'amont `xberg`, et le programme de specs qui en découle. Ce fichier est l'index : il donne
l'ordre de lecture, le registre des décisions, et — point important — **la trace des
corrections**, parce que plusieurs conclusions de la première rédaction ont été invalidées par
vérification et qu'une lecture partielle mènerait à des conclusions fausses.

---

## 1. Par où commencer

| Vous êtes | Lisez, dans cet ordre | Temps |
| --- | --- | --- |
| **Direction / produit** | §1 et §6 de l'analyse, puis §9.12.3-4 (position concurrentielle), puis §10 du programme (les cinq décisions) | ~20 min |
| **Architecte** | L'analyse en entier, puis le programme §2 à §8 | ~1 h |
| **Développeur sur la vague 0** | Programme §2 bis, puis les specs P2, P4, P5 | ~30 min |
| **Développeur sur le chemin critique** | Programme §2 bis, puis S1, P1, E0 | ~45 min |
| **Vous reprenez ce travail plus tard** | §3 de ce fichier (corrections), avant tout le reste | 5 min |

---

## 2. Carte des documents

### Analyse

| Document | Contenu |
| --- | --- |
| [`2026-07-31-analyse-architecture-et-pistes-produit.md`](2026-07-31-analyse-architecture-et-pistes-produit.md) | Évaluation du dépôt (API, SDK, RAG, multi-tenance, persistance, observabilité), rôle réel de `xberg`, ce que l'amont livre, position concurrentielle face à Xberg Enterprise, pistes business |

### Programme et specs

Tous dans `superpowers/specs/`.

| Document | Rôle |
| --- | --- |
| [`2026-08-01-hacienda-platform-parity-program.md`](../../superpowers/specs/2026-08-01-hacienda-platform-parity-program.md) | **Le découpage.** 19 specs, 4 pistes, invariants de programme, ordre de livraison, recommandations sur les 5 décisions ouvertes |
| `2026-08-01-S1-tenancy-and-projects.md` | Tenants, projets, espace de clés par tenant — **chemin critique** |
| `2026-08-01-S4-api-contract-and-clients.md` | Document OpenAPI étoffé, clients HTTP générés |
| `2026-08-01-P1-redaction-enforcement-point.md` | Garde non contournable sur tout store — **chemin critique** |
| `2026-08-01-P2-audit-exposure-and-verification.md` | Chaîne d'audit exposée et vérifiable |
| `2026-08-01-P3-pseudonymisation-as-a-service.md` | Jetons, révélation, rotation de clés |
| `2026-08-01-P4-human-review-api.md` | File de revue humaine |
| `2026-08-01-P5-compliance-artefacts-api.md` | DPIA, model card, DORA, AI Act |
| `2026-08-01-E0-redacting-document-loader.md` | Chargeur LangChain/LlamaIndex — **chemin critique**, valide P1 |

**Non rédigées** : S2, S3 (attendent le choix de backend), V1–V4 (attendent la fusion de
PR #42/#43 dont elles prolongent les mesures), E1–E5 (attendent P1 livré et éprouvé par E0).
Elles restent définies par le programme. Voir son §2 bis.

### Specs antérieures dont ce travail dépend

| Document | Ce qu'il apporte |
| --- | --- |
| `2026-07-28-hacienda-cli-api-integration-design.md` | Le principe du **proxy rédacteur** : hacienda n'est pas un sur-ensemble de xberg. Fondement de l'invariant I1 |
| `2026-07-31-vertical-model-specialisation-design.md` (PR #42) | Mesures LoRA : 0,43 % du modèle, 232× de surcoût en merge-at-load, plafond 4 Go |
| `2026-07-29-business-law-gliner2-lora-design.md` (PR #35) | Pipeline d'entraînement d'adaptateurs |
| `2026-07-27-vertical-ner-architecture-design.md` | Taxonomies verticales |

---

## 3. Corrections — à lire avant toute reprise

L'analyse a été écrite en plusieurs passes, chacune corrigeant la précédente après vérification
directe contre l'amont. **Une lecture de la première version mène à trois conclusions fausses.**
Le document publié intègre les corrections ; cette section existe pour que personne ne les
re-dérive à l'envers.

| Affirmation initiale | Réalité vérifiée | Comment |
| --- | --- | --- |
| « `xberg-rag` et `xberg-doc-store` sont disponibles, il suffit de les déclarer au même tag » | **Absents** du tag `v1.0.2` épinglé comme de `main` (1.0.6). Ils ont cessé d'être publiés entre la rc.5 et la GA | `git ls-tree` sur `xberg-io/xberg` aux deux références |
| « `TenantCtx` existe en amont, il suffit de l'adopter » | Il était dans `xberg-doc-store`, **parti avec le reste**. À écrire ici (spec S1) | idem |
| « La couche vectorielle est inaccessible, donc à réécrire » | **Faux : la licence n'est pas le problème.** MIT, irrévocable pour toute version publiée. ~3 900 lignes reprenables, sans couplage à la version de xberg | Lecture des imports et du manifeste rc.5 |
| « Le serveur MCP est absent, à écrire » | La moitié extraction est une **feature `xberg/mcp`** sur une dépendance déjà déclarée | `crates/xberg/src/mcp/`, `Cargo.toml` ligne 515 |
| « Le RAG suppose de construire un moteur vectoriel » | L'amont publie **huit intégrations de frameworks** — voie bien moins chère (spec E0) | `integrations/README.md` au tag épinglé |

**Provenance des vérifications.** `xberg-io/xberg` n'est pas atteignable par l'outillage GitHub
de cette session (portée limitée à `jamon8888`), mais l'est **en git simple** via le proxy. Les
tags `v1.0.2` et `main` ont été récupérés et inspectés directement. Le fork `jamon8888/xberg`
est figé à **1.0.0-rc.5** : il ne représente pas l'amont et ne doit jamais entrer dans le graphe
de dépendances — sa seule valeur est d'être la dernière copie MIT publiée de `xberg-rag`.

**Non vérifié.** Le dépôt privé « Xberg Enterprise » n'a pas pu être inspecté ; un sondage des
noms plausibles sous `xberg-io` n'a rien donné. En revanche **ses specs OpenAPI sont publiques**
dans `xberg-io/sdks`, ce qui a permis d'en lire toute la surface fonctionnelle. Restent inconnues
sa licence, son mode de distribution et sa gouvernance.

**Exception temporaire à « ne doit jamais entrer dans le graphe de dépendances » (2026-08-08).**
Le sous-module git `test_documents` du commit `xberg-io/xberg` épinglé par `tag = "v1.0.2"`
(`9dcc864d`) pointe vers un commit qui n'existe plus sur `xberg-io/test_documents` — vérifié par
un `git fetch <sha>` direct, qui renvoie « not our ref », et vrai de **chaque** tag `xberg-io/xberg`
de v1.0.2 à v1.0.14 : ce n'est pas réparable en changeant de version. `cargo` récupère
inconditionnellement les sous-modules d'une dépendance git lors de la résolution, donc ceci casse
`cargo build` pour tout le workspace, sur `main` comme sur toute branche. Le workaround adopté :
`jamon8888/xberg@fix/test-documents-submodule-pin` — arbre identique au commit `9dcc864d`, avec
uniquement ce gitlink repointé vers le HEAD valide actuel de `test_documents`. `Cargo.toml` et
`hacienda-core/Cargo.toml` pointent temporairement là plutôt que sur `xberg-io/xberg` directement ;
revenir au tag amont dès que `xberg-io` corrige le sous-module. Voir le commentaire au-dessus de
la dépendance `xberg` dans chacun des deux fichiers pour le détail complet.

---

## 4. Registre des décisions

### Décisions de programme (§10 du programme)

| # | Question | Décision | Statut |
| --- | --- | --- | --- |
| **D1** | Moteur RAG complet ou chargeur de framework ? | **Les deux** — chargeur en vague 1 (spec E0), E4 en vague 4. Le chargeur éprouve le garde P1 contre un store tiers avant que E4 n'en dépende | Recommandée |
| **D2** | Verticales : zéro-shot ou poids entraînés ? | **Tier 0 zéro-shot.** Entraîner un LoRA seulement là où le F1 Tier 0 mesuré tombe sous le seuil produit | Recommandée |
| **D3** | Diff de versions sur jetons pseudonymisés ? | **Validé**, sous deux conditions : résolution de la rotation de clés, et dégradation annoncée en mode `Mask` | Recommandée |
| **D4** | Quarantaine des uploads présignés ? | **Adoptée.** Le coût réel n'est pas la latence mais le cycle de vie : destruction de l'original + TTL | Recommandée |
| **D5** | Parité totale ou sélective ? | **Sélective** : E2, E3, E4 complets ; E1, E5 au minimum utile | Recommandée — **revient sur l'arbitrage initial**, décision au commanditaire |

### Invariants de programme

Applicables à **toutes** les specs, chacune devant porter un test négatif :

- **I1** — aucun texte non rédigé ne franchit une frontière de persistance ;
- **I2** — toute opération sur du contenu écrit son audit avant de rendre son résultat ;
- **I3** — tout est cloisonné par tenant ; l'absence se signale par 404, jamais 403 ;
- **I4** — la table de routes reste l'unique source de vérité, et OpenAPI en dérive.

---

## 5. Position produit retenue

La lecture des specs OpenAPI d'Enterprise a déplacé le positionnement.

**Ce qu'Enterprise couvre déjà** : extraction, toute la couche RAG, rédaction PII
(`mask | hash | token_replace | drop`), journal d'activité, métrage d'usage, versionnement et
diff de documents, cloisonnement par projet.

**Ce qu'il ne couvre pas** — et c'est le différenciateur :

| | Enterprise | hacienda |
| --- | --- | --- |
| Pseudonymisation réversible | absente | AES-256-SIV déterministe, rotation additive |
| Audit infalsifiable au niveau du contenu | journal d'activité, sans chaîne ni vérification | chaîne blake3, une entrée par span, vérifiable |
| Artefacts réglementaires | absents | DPIA, model card, DORA, AI Act |
| Zéro-egress | API hébergée | navigateur et sur site |
| Revue humaine | absente | file durable event-sourcée |

**Formulation retenue :** *« Vos documents deviennent interrogeables par une IA sans qu'aucune
donnée personnelle ne quitte votre périmètre — et vous pouvez le prouver. »* Le poids porte sur
la seconde moitié : la première est désormais une commodité vendue par l'amont.

---

## 6. Questions ouvertes

| Question | Qui décide | Bloque |
| --- | --- | --- |
| Nom exact et conditions du dépôt Xberg Enterprise | Commanditaire | L'évaluation de la voie « licencier » plutôt que « reprendre » |
| D5 — parité totale ou sélective | Commanditaire | La portée de E1 et E5 |
| README : réparer `alef.toml` ou passer 13 bindings en 🚧 | Commanditaire | Spec S4, et toute démarche commerciale |
| Backend de persistance (Postgres ? KMS ?) | Architecture | Rédaction de S2 et S3 |
| Fusion de PR #42/#43 | Programme | Rédaction de V1–V4 |

---

## 7. Ce qu'il faut faire en premier

Aligner le README sur la réalité, et exposer par API le métier déjà écrit — specs **P2**, **P4**,
**P5**. Le premier point protège la crédibilité ; le second transforme plusieurs milliers de
lignes de code testé et invisible en surface produit vendable, pour quelques centaines de lignes
de handlers.

Puis **S1**, seul chantier réellement bloquant : le rétro-ajouter après mise en production impose
de migrer les chaînes d'audit *et* de re-dériver tous les jetons émis.

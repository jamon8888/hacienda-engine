# Wayfinder #122 — Contrat d'autorisation et d'interface de réhydratation

- **Issue traitée** : jamon8888/hacienda-engine#122 (label `wayfinder:grilling`)
- **Carte parente** : #118 — *Wayfinder — Secrecy Client Tauri*
- **Dépend de (blocked-by)** : #126 (audit des capacités existantes, fermée le 2026-11-15), #120 (périmètre juridique et menace, fermée)
- **Issue liée** : #17 — *Short non-reversible pseudonym tokens as a separate mode* (ouverte, non implémentée)
- **Base inspectée** : `origin/main` @ `a65311c` (« chore(lint)… #141 », tip du 2026-11-16), lecture seule.
  Point notable : la surface MCP `pii_reveal` a été **retirée au commit précédent `db8d895`**
  (PR #132 « LocalFull et recherche RAG hybride tokenisée »), qui a aussi introduit le
  « contrat B » (`crates/hacienda-client-core`, `crates/hacienda-backend`). L'analyse ci-dessous
  reflète ce tip ; les affirmations de #122 sur un `pii_reveal` MCP exposé décrivent un état
  antérieur à `db8d895`.
- **Date** : 2026-11-16
- **Nature du livrable** : note de grilling (aucune modification de code, aucune PR/issue touchée)

> Convention de vocabulaire utilisée dans tout le document :
> - **pseudonymisation** : valeur → jeton (`Pseudonymiser::token`, `POST /v1/pii/token`).
> - **réhydratation / révélation** : jeton → valeur (`Pseudonymiser::reveal`, `POST /v1/pii/reveal`).
> - On distingue la **réhydratation-plaintext** (rend une valeur brute à un humain) de la
>   **réhydratation-référence** (rend seulement des références opaques, jamais de clair).

---

## 1. État actuel du contrat

Il n'existe **pas un** contrat de réhydratation mais **deux contrats parallèles, non
reliés entre eux**, plus un **troisième chemin, le viewer Tauri, qui implémente déjà la
cible « humain, usage unique, session liée » mais hors de tout type partagé**. La surface
MCP qui contournait l'autorisation (`pii_reveal` en `Caller::Trusted`) a, elle, été
**fermée sur le tip courant** (voir §1.1), ce qui retire la lacune la plus grave listée
par #122 — sans pour autant doter le dépôt d'un contrat unique.

### 1.1 Contrat A — Façade / REST / CLI / MCP (`hacienda-core` + `hacienda-api` + `hacienda-mcp`)

C'est le contrat historique « pseudonymisation as a service » (specs `P3`, `P3a`, `P3b`).

| Élément | Fichier / symbole | Constat |
| --- | --- | --- |
| Endpoint révélateur | `hacienda-api/src/handlers/pii.rs` → `reveal_token` | `POST /v1/pii/reveal`, corps `RevealTokenRequest { token }` (`hacienda-api/src/dto.rs`), `deny_unknown_fields` |
| Contrôle d'accès | `hacienda-core/src/facade.rs` → `reveal_token_with_auth` | exige le seul cap `Capability::PiiReveal` ; pas de `documents:process` dans la façade (la couche transport ajoute la route `Access::Capability(PiiReveal)` dans `routes.rs`) |
| Portée du cap | `hacienda-core/src/auth/mod.rs` | `pii:reveal` est un cap **global au principal**, non paramétré par document / champ / jeton |
| Résolution de clé | `HaciendaFacade::pseudonymiser_for(tenant)` | clé **scopée par tenant** (`P3a`), résolue depuis l'environnement (`EnvKeyResolver`) |
| Token | `hacienda-core/src/redaction/pseudonym.rs` | format `[CATEGORY:key_id:base32_ciphertext]`, AES-256-SIV, rembourré en buckets de 16 octets ; `key_id` **en clair** dans le jeton |
| Audit | `facade.rs` → `record_token_reveal` | 1 entrée `RedactionAction::Reveal` par jeton, `category = "pseudonym"`, `span_hash = blake3(clair)` ; **`principal` optionnel** |
| MCP | `hacienda-mcp/src/lib.rs` → `pii_reveal` | appelle `reveal_token_with_auth(Caller::Trusted, …)` ; `Caller::Trusted` **ne vérifie aucun cap** et laisse `principal = None` dans l'audit |
| Mint (valeur→jeton) | `POST /v1/pii/token` → `mint_pseudonym_token_with_auth` | exige seulement `documents:process`, **pas** `pii:reveal`, et **n'écrit aucun audit** (choix assumé, D-P3-1) |
| Scan révélant | `POST /v1/pii/scan` avec `include_text:true` | exige `pii:reveal` et écrit une entrée `Reveal` **par span** (`record_reveal`) |
| Rotation / statut clés | `GET /v1/keys` → `list_keys_with_auth` | `audit:read` ; retourne `KeyStatus { id, active }` ; `POST /v1/keys/rotate` **non implémenté** (`P3b §4`) |

**Qui peut réhydrater quoi (contrat A)** : tout principal disposant du cap `pii:reveal`
pour **son tenant** peut réhydrater **n'importe quel jeton** dont la clé est chargée pour
ce tenant — sans lien avec un document, un champ, une session ou une justification.

### 1.2 Contrat B — Profils client & backend (`crates/hacienda-client-core` + `crates/hacienda-backend`)

C'est le contrat récent issu de la branche LocalFull / RAG (PR #132, désormais visible
sur `origin/main`). Il raisonne en « profil d'installation » et **ne rend jamais de clair**.

| Élément | Fichier / symbole | Constat |
| --- | --- | --- |
| Profils | `crates/hacienda-client-core/src/lib.rs` → `InstallationConfig` | `LocalFull` vs `TeamOnPremise` |
| Demande | `RehydrationRequest { document_ids, tenant_scope, capability }` | autorisation au niveau **document**, pas du jeton |
| Capability | `RehydrationCapability` | `LocalEncryptedVault { vault }` (LocalFull) ou `AuditedServer { gateway, audit }` (Team) |
| Validation | `RehydrationRequest::validate_for` / `InstallationConfig::validate_rehydration` | LocalFull ⇒ vault doit correspondre, `tenant_scope` interdit ; Team ⇒ tenant doit correspondre, gateway doit correspondre, `audit` non vide |
| Résultat | `crates/hacienda-backend/src/lib.rs` → `Backend::rehydrate` | renvoie `RehydrationResponse` : **`tokenized_content` reste tokenisé** + `references: Vec<OpaqueTokenReference>` ; plaintext **non représentable** dans le type |
| Coffre → clair | `VaultRehydrationAdapter::reveal` | seul chemin qui rend du clair : `EphemeralPlaintext` (Zeroizing, pas de `Serialize`/`Debug`/`Display`) + `AuditReference` |
| Audit coffre | `VaultRehydrationAdapter::reveal` | entrée `Reveal`, `span_hash = blake3(clair)`, `pipeline_version = "hacienda-backend-vault-v1"`, **`principal: None`** |
| Capability affichage | `provider.rs` → `DisplayCapability` | type défini, **non câblé** dans `reveal`/`rehydrate` |

**Qui peut réhydrater quoi (contrat B)** : un appelant qui présente le bon
`RehydrationCapability` pour le bon profil peut réhydrater **les documents listés**
(formellement vérifiés), mais n'obtient que des **références** ; le clair passe par un
adaptateur de coffre séparé qui, lui, n'est **pas** couplé au document demandé.

### 1.3 Preuves d'audit réellement produites

Une entrée `reveal` (`hacienda-core/src/audit/entry.rs`, `AuditEntry`) contient :
`id`, `timestamp`, `category`, `action = Reveal`, `span_hash` (= blake3 du **clair**),
`span_length`, `confidence`, `source`, `pipeline_version`, `config_hash`, `principal`
(optionnel), `vertical`, `model`, `chain_hash`.

Ce qui **n'y est pas** : identifiant du jeton révélé, identifiant du document, justification,
durée d'expiration, identifiant de ticket de divulgation, résultat de révocation, une
quelconque indication de la granularité demandée.

### 1.4 Ce qui fonctionne déjà (à ne pas casser)

- **Cloisonnement par tenant** des clés : deux tenants portant la même valeur obtiennent
  des jetons différents (`P3a`) ; un jeton d'un autre tenant est « clé inconnue ».
- **Rotation additive** : une clé `retired` reste révélable (`retired_key_still_reveals`).
- **Échec fermé sur jeton inconnu/malformé** : `MalformedToken | UnreadableToken |
  KeyNotFound` sont tous ramenés à un **400 générique** (`hacienda-api/src/error.rs`) —
  pas d'oracle d'erreur.
- **Mint sans révélation** : `POST /v1/pii/token` rend possible le droit d'accès/effacement
  sans déchiffrer.
- **Contrat B ne peut pas fuiter de clair par sa réponse** : `RehydrationResponse` ne
  porte que du tokenisé + références.

---

## 2. Lacunes

Classées par gravité décroissante.

- **L1 — MCP `pii_reveal` contourne toute autorisation.** `Caller::Trusted` sur le
  transport stdio : aucun cap vérifié, `principal = None` dans l'audit. Un agent
  (Claude Desktop/Codex) branché sur `hacienda mcp serve` peut réhydrater et **aucune
  entrée d'audit ne nomme qui a révélé**. Incompatible avec la séparation agent/humain de
  #118 et avec l'écart listé dans #126.
- **L2 — Granularité tenant-large.** `pii:reveal` autorise **tous** les jetons du tenant.
  Aucun périmètre par utilisateur, dossier, document, champ ou jeton, contrairement à ce
  qu'exige la cible de #122 et la décision #120 (double approbation haute sensibilité).
- **L3 — Aucune justification.** Ni la requête REST, ni `RehydrationRequest`, ni l'entrée
  d'audit ne portent un motif de divulgation.
- **L4 — Aucune durée de vie / usage unique.** Le contrat A rend le clair sans TTL ni
  ticket à usage unique ; rien n'invalide une réponse capturée. La cible #122 demande
  « expiration courte, révocation immédiate, aucun cache durable ».
- **L5 — Aucune révocation d'urgence explicite.** La rotation est additive ; il n'existe
  pas d'opération « retirer une clé » (une clé non listée devient simplement `KeyNotFound`).
  Pas de notion de révocation d'un accès en cours, d'un jeton ou d'un ticket.
- **L6 — Deux contrats non unifiés.** Contrat A (jeton, `pii:reveal`) et contrat B
  (document, `RehydrationCapability`) ne partagent ni vocabulaire, ni audit, ni point
  d'entrée. Un client peut satisfaire l'un et ignorer l'autre ; la matrice de décision de
  `docs/research/2026-11-14-wayfinder-131-xberg-lancedb-rehydration.md` demandait déjà de
  « ne pas mélanger » les deux, mais aucun garde-fou de code ne l'impose.
- **L7 — Le clair du coffre (contrat B) n'est pas lié au document autorisé.**
  `VaultRehydrationAdapter::reveal` prend un `token`, pas un `document_id` ; l'autorisation
  par document de `Backend::rehydrate` et la révélation de clair sont deux appels
  déconnectés.
- **L8 — Audit sans identité pour le canal coffre.** `principal: None` (`hacienda-backend`)
  et `principal: None` (`Caller::Trusted`) ⇒ l'entrée `Reveal` prouve qu'une révélation a
  eu lieu, pas **qui** l'a demandée.
- **L9 — Rotation/backup/récupération de clés non résolues.** `POST /v1/keys/rotate`
  absent (`P3b §4`) ; `EnvKeyResolver` lit une variable d'environnement au démarrage. La
  révocation d'urgence et la rotation de clés demandées par #122 n'ont pas de mécanisme.
- **L10 — Cas d'un **jeton inconnu renvoyé par un fournisseur** non tranché.**
  Le gateway re-détecte les PII des réponses, mais la politique « jeton inconnu dans une
  réponse » (échec fermé, affichage « inconnu », pas de reconstruction) n'est pas
  implémentée côté API.
- **L11 — Export non couvert.** Les exports (`audit:export`) ne suivent pas le contrat de
  réhydratation : rien n'interdit qu'un export contienne un document tokenisé **plus** de
  quoi réhydrater hors ligne.
- **L12 — Mode non réversible court (#17) absent.** `RedactionMode` = `Mask | Hash |
  Pseudonymize | Remove | Custom` ; il n'existe pas de mode « pseudonyme court non
  réversible ». Un tel jeton ne **doit jamais** être révélable : le contrat doit distinguer
  les jetons réversibles des non réversibles, aujourd'hui indiscernables pour un opérateur.
- **L13 — Réponse du viewer Tauri non contractualisée.** #122 décrit session liée au token,
  capacité à usage unique et expiration ; ce chemin vit dans le client Tauri et n'est
  adossé à aucun type du dépôt (ni `DisplayCapability`, non câblé).

---

## 3. Options de contrat cible

### Option 1 — **Ticket de divulgation scopé, sur un contrat unique** (recommandée)

Un seul objet de contrat, quelle que soit la surface (REST, CLI, viewer Tauri, coffre) :
`RevealTicket`.

- **Objet** : `{ ticket_id, subject (principal), tenant, document_ids[], token_ids[]/field_scope,
  justification, requested_at, expires_at, one_shot, approvers[] }`.
- **Autorisation** : création du ticket = `pii:reveal` **et** appartenance au dossier
  (ACL métier) ; résolution du ticket → clair = uniquement le porteur du ticket, une
  seule fois, avant `expires_at`.
- **Révocation** : le ticket est révocable ; un ticket révoqué ne rend plus de clair.
- **Audit** : entrée `Reveal` enrichie de `ticket_id`, `document_id`, `token_id`,
  `justification`, `approvers`, `principal` (jamais `None` sur un chemin humain).
- **MCP** : `pii_reveal` **retiré** du manifeste agent. Aucun ticket ne peut être minté
  par `Caller::Trusted`.
- **Cas limites tranchés** : haute sensibilité ⇒ `approvers.len() >= 2` ; jeton inconnu ⇒
  400 fail-closed ; mode non réversible (#17) ⇒ jamais de ticket possible.

*Avantages* : un seul chemin, audit complet, répond à L1–L7 et L10–L12 d'un coup.
*Coûts* : toucher façade, API, MCP, audit et le contrat client-core ; migration.

### Option 2 — **Réhydratation médiée par coffre, clair jamais côté API**

On conserve la forme actuelle en séparant nettement les deux contrats et en interdisant le
clair hors coffre.

- L'API/façade ne renvoie **que** des références (`RehydrationResponse`) ; le clair n'est
  produit que par `VaultRehydrationAdapter`, derrière une `DisplayCapability` câblée et
  une session viewer.
- `pii:reveal` ne sert plus qu'à **ouvrir une session de viewer** ; chaque valeur rendue
  est limitée par la session (document + champs) et journalisée avec `principal`.

*Avantages* : proche du code existant, séparation claire déjà amorcée par le contrat B.
*Coûts* : laisse deux contrats ; le clair reste accessible à qui contrôle le coffre ;
L9 (rotation) toujours ouvert ; le périmètre par champ reste à définir.

### Option 3 — **ACL croisée document × champ × utilisateur, contrat A conservé**

On garde `POST /v1/pii/reveal` mais on le rend dépendant d'une ACL explicite.

- Nouvelle table `reveal_grant { tenant, user|role, document_id, field|category, expires_at }`.
- `reveal_token` doit résoudre le jeton → document(s) qui le contiennent, puis vérifier
  qu'il existe un grant couvrant ce document et cette catégorie pour l'appelant.
- Audit inchangé + `document_id` / `field` ajoutés.

*Avantages* : périmètre fin (par champ), réutilise l'existant.
*Coûts* : suppose un index jeton→documents (aujourd'hui absent) ; le jeton ne porte **pas**
  d'identifiant de document, donc la résolution est coûteuse ou impossible hors RAG ;
  ne règle pas L1 (MCP) sans décision séparée.

---

## 4. Questions de grilling

1. **Défaut** : la réhydratation est-elle **interdite par défaut** (opt-in explicite), ou
   autorisée par défaut et restreinte ensuite ? Qui a le droit, aujourd'hui, sans nouvelle
   configuration : personne ou tout `pii:reveal` ?
2. **Granularité minimale** : le plus petit périmètre autorisable est-il le **jeton**, le
   **champ/catégorie**, le **document**, ou le **dossier** ? #122 propose
   « passages/champs par défaut, dossier avec confirmation renforcée » — comment un jeton
   est-il rattaché à un champ sans index jeton→document ?
3. **Qui autorise** : le propriétaire du dossier, un délégué, un administrateur, ou
   l'utilisateur lui-même ? Existe-t-il une notion de rôle (avocat / assistant / expert)
   ou seulement des caps par service token ?
4. **Double approbation** : pour quels seuils (catégorie sensible, volume, export) deux
   approbateurs sont-ils requis ? Comment prouve-t-on *a posteriori* que les deux ont
   approuvé (chaîne d'audit, deux entrées liées) ?
5. **Preuve à produire** : l'entrée d'audit doit-elle contenir le `principal`, le
   `ticket_id`, la `justification`, la durée, et l'issue ? Le `span_hash` seul suffit-il à
   un audit juridique français, ou faut-il un identifiant lisible du document/champ ?
6. **Durée et usage** : quelle expiration maximale (10 min est proposé), et la divulgation
   est-elle strictement **à usage unique** ? Que se passe-t-il si la session Tauri recharge
   la page pendant la fenêtre ? Y a-t-il un cache renderer autorisé ?
7. **Révocation d'urgence** : que révoque-t-on — une clé, un jeton, un ticket, une
   session ? Quelle est la latence cible et que devient un clair déjà rendu ? Qui peut
   déclencher la révocation ?
8. **Rotation de clés** : `POST /v1/keys/rotate` doit-il exister, ou la rotation reste-t-elle
   une opération d'exploitation hors API (spécifié P3b §4) ? Comment révèle-t-on un jeton
   d'une clé **retirée** sans réintroduire le matériel ?
9. **Jeton inconnu / réponse fournisseur** : un jeton non résolvable renvoyé par Claude
   Codex est-il affiché « inconnu », masqué, ou bloque-t-il l'affichage ? Un jeton d'un
   autre tenant doit-il rester « clé inconnue » (D-P3-6) ou devenir une alerte de sécurité ?
10. **Export** : un export tokenisé peut-il être réhydraté hors ligne ? Le contrat doit-il
    produire un export *sans* map de réhydratation, ou un export avec scheme enregistré
    (`rehydration_scheme`, cf. recherche #131) ?
11. **Mode non réversible (#17)** : un pseudonyme court non réversible doit-il être
    interdit sur les surfaces de réhydratation, et comment un opérateur distingue-t-il un
    jeton réversible d'un jeton non réversible sans essayer de le déchiffrer ?
12. **MCP et agent** : confirme-t-on le retrait pur et simple de `pii_reveal` du manifeste
    agent, ou une variante « référence seule » (aucun clair) est-elle gardée pour les
    agents ? Qui, sinon, peut appeler `Caller::Trusted` et pourquoi ?
13. **Contrat B (profil client)** : `RehydrationCapability` et `RehydrationRequest`
    deviennent-ils la **seule** porte d'entrée (au prix d'unifiere), ou restent-ils un
    contrat de profil distinct ? Comment `AuditedServer { gateway, audit }` prouve-t-il
    l'identité de l'humain derrière la session ?
14. **Frontière de confiance** : en Team On-Premise, la réhydratation est-elle autorisée
    côté gateway (et le clair circule au client léger) ou uniquement dans un vault local
    inaccessible au client réseau ? En Local Full, le processus est réputé de confiance —
    cette hypothèse de menace est-elle acceptable pour les données visées (#120) ?

---

## 5. Recommandation

**Retenir l'Option 1 — « ticket de divulgation scopé sur un contrat unique »**, en
l'appliquant par étapes pour ne pas casser les garanties déjà en place (§1.4) :

1. **Fermer d'abord `pii_reveal` côté MCP** (L1) : le retirer du manifeste agent ou le
   rendre impossible pour un appelant non humain. C'est l'écart le plus grave et le moins
   coûteux à corriger ; il aligne le code sur la cible « jamais via MCP/agent » de #118/#126.
2. **Introduire `RevealTicket`** (L2–L5) portant `principal`, `document_ids`, `field/scope`,
   `justification`, `expires_at`, `one_shot`, `approvers`, et l'écrire en entier dans
   l'entrée `Reveal` (L8). Bannir `principal = None` sur tout chemin humain.
3. **Unifier les deux contrats** (L6–L7) autour du ticket : le contrat B
   (`RehydrationRequest`) devient la *demande de ticket*, `VaultRehydrationAdapter` devient
   l'*exécutant* du ticket — jamais un appel déconnecté prenant un jeton nu.
4. **Trancher explicitement les cas limites** : jeton inconnu fail-closed (L10), mode non
   réversible #17 interdit de réhydratation (L12), export sans map de réhydratation (L11).
5. **Reporter la rotation/révocation de clés** à leur lot dédié (L9) : la cible
   `S2 KMS-backed KeyResolver` de P3b §4 est le bon foyer ; tant qu'elle n'existe pas, la
   révocation d'urgence doit au moins pouvoir invalider les **tickets**, pas les clés.

**Justification** : l'Option 1 est la seule qui traite simultanément le périmètre fin, la
preuve d'audit nominative, l'expiration/usage unique et la fermeture du canal agent, qui
sont les quatre exigences non négociables de #122. Les Options 2 et 3 sont soit partielles
(2 laisse deux contrats et ne règle pas la rotation), soit insuffisantes techniquement
(3 suppose un index jeton→document qui n'existe pas). Les questions du §4 restent à
trancher avec l'auteur avant de figer les types.

---

### Annexe — chemins inspectés (lecture seule, `origin/main`)

- `hacienda-api/src/handlers/pii.rs`, `hacienda-api/src/dto.rs`, `hacienda-api/src/routes.rs`,
  `hacienda-api/src/error.rs`
- `hacienda-core/src/facade.rs`, `hacienda-core/src/auth/mod.rs`,
  `hacienda-core/src/auth/authz.rs`, `hacienda-core/src/audit/entry.rs`
- `hacienda-core/src/redaction/pseudonym.rs`, `hacienda-core/src/redaction/types.rs`
- `hacienda-mcp/src/lib.rs`, `hacienda-mcp/src/params.rs`
- `hacienda-cli/tests/pii_reveal.rs`
- `crates/hacienda-client-core/src/lib.rs`, `crates/hacienda-client-core/src/provider.rs`,
  `crates/hacienda-backend/src/lib.rs`
- `superpowers/specs/2026-08-01-P3-pseudonymisation-as-a-service.md`,
  `superpowers/specs/2026-08-14-P3a-tenant-scoped-pseudonym-keys.md`,
  `superpowers/specs/2026-08-14-P3b-pseudonym-key-management-api.md`
- `docs/research/2026-11-14-wayfinder-131-xberg-lancedb-rehydration.md`,
  `docs/superpowers/specs/2026-11-14-wayfinder-122-gateway-response-contract.md`

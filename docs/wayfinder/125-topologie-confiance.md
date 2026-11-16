# Wayfinder #125 — Topologie de confiance : mode local et mode équipe/on-premise

**Statut :** note de grilling (lecture seule, `origin/main` = `db8d895`, merge #132), non arbitrée.
**Parent :** #118 (carte Wayfinder) · **Bloquée par :** #126 (audit redaction/révélation), #124 (frontière Claude/Codex/MCP), #123 (sécurité RAG).
**Livrable :** cadrage du problème, options de topologie, menaces couvertes/non couvertes, questions à trancher. Pas une décision.
**Ancres vérifiées** (lecture seule `git show origin/main:<path>`) : `crates/hacienda-client-core/src/lib.rs`, `crates/hacienda-backend/src/lib.rs`, `crates/hacienda-rag/src/backends/lancedb.rs`, `apps/hacienda-secrecy-client/src-tauri/src/main.rs`, `docs/research/2026-11-14-wayfinder-130-lancedb-full-rag.md`, `docs/research/2026-11-14-wayfinder-131-xberg-lancedb-rehydration.md`.

---

## 1. Reformulation du problème

La question de #125 est une question d’**invariant unique** : à quoi ressemble la topologie *minimale* qui garantit **le même invariant de confidentialité** dans deux modes d’installation qui n’ont ni la même frontière de confiance, ni le même cycle de vie opérationnel ?

Les deux modes en jeu (contrat déjà posé par #131) :

- **Local Full** — client Tauri autonome, Hacienda/Xberg local, LanceDB embedded, coffre local chiffré, gateway local Claude/Codex. Le poste est mono-utilisateur et doit fonctionner **hors ligne**.
- **Team On-Premise** — client Tauri léger, Engine/Gateway serveur multi-tenant autoritaire, LanceDB data plane token-only, Postgres/object store control plane, réhydratation serveur contrôlée. Le réseau privé est requis.

Reformulé sans jargon : *le mode local promet de protéger l’utilisateur contre le fournisseur, l’équipe et l’index, en s’appuyant sur un seul poste ; le mode équipe promet de protéger chaque tenant contre les autres, le fournisseur et l’index, en s’appuyant sur un serveur. Quelle combinaison d’emplacements (texte en clair, coffre, index RAG, clés, passerelle, Engine) tient la promesse dans les deux cas, et où la promesse casse-t-elle ?*

L’issue demande explicitement de comparer six emplacements et cinq chemins de panne. C’est cette double comparaison qui structure la note.

### 1.1 Les six emplacements à situer

| Élément | Question de topologie |
|---|---|
| Texte en clair | Où peut-il exister, combien de temps, sous quelle forme (mémoire éphémère, fichier, transcript) ? |
| Coffre de pseudonymes | Où vit la correspondance `token → valeur brute`, et qui peut la lire ? |
| Index RAG | Où vivent embeddings/FTS, et contiennent-ils autre chose que du token-only ? |
| Clés | Où vit la clé de réversibilité (keyring OS, KMS/HSM, fichier) et comment tourne-t-elle ? |
| Passerelle Tauri | Quel processus termine la confiance, et quelle capacité y est vérifiée ? |
| Hacienda Engine | Qui est l’autorité d’autorisation/audit : le processus local ou la passerelle de tenant ? |

### 1.2 Les cinq chemins de panne à tester

1. **Serveur indisponible** — la passerelle/Engine de tenant ne répond pas.
2. **Clé indisponible** — keyring verrouillé, clé perdue, KMS injoignable.
3. **Politique absente** — pas de politique de redaction, pas d’ACL, pas de politique réseau.
4. **Mise à jour non vérifiée** — binaire ou modèle remplacé sans signature/vérification.
5. **Poste compromis** — code malveillant avec les privilèges de l’utilisateur (ou root).

---

## 2. Contraintes de sécurité (ce qui doit rester vrai dans les deux modes)

### 2.1 Invariant candidat (unicité recherchée)

> **Aucun texte en clair, ni la clé de réversibilité, ne franchit la frontière de confiance vers un fournisseur, un index interrogéable, un log, un export, une sauvegarde ou un autre tenant. Le fournisseur ne reçoit que du token-only et des métadonnées non secrètes. La réhydratation est humaine, à la frontière, éphémère, sous capacité et auditée, et n’alimente jamais le contexte LLM/agent.**

Cet invariant est *une seule phrase*, mais son **point d’application diffère** selon le mode : le processus local en Local Full, la passerelle de tenant authentifiée en Team. Toute la grilling #125 revient à décider si cette différence de frontière est compatible avec un invariant prétendu identique, ou s’il faut deux invariants distincts honnêtement nommés.

### 2.2 Contraintes issues de la carte #118 (déjà tranchées ailleurs)

- **Séparation stricte** texte en clair / texte pseudonymisé / coffre de réhydratation / index RAG / fournisseurs (préférence permanente #118).
- **Human-ward** : la réhydratation est un acte humain local, jamais via MCP ni agent (#126).
- **Aucun hook universel** dans Claude Desktop/ChatGPT Desktop : la garantie pré-envoi appartient au client/gateway Hacienda (#124). Le mode local n’est donc pas un raccourci de conformité.
- **RAG strict** : tenant effectif, ACL document/chunk, redaction du system prompt, stockage agent-visible token-only (#123).
- **Menaces retenues** (#120) : fournisseur, équipe, admin, processus, export, configuration ; divulgation humaine autorisée et doublement approuvée pour haute sensibilité.
- **Gateway** : contrat Anthropic-compatible `/v1/messages`, headers de capacité, SSE conservés (#121).
- **LanceDB** : data plane RAG token-only recommandé, derrière `RagStore` ; ne remplace ni gateway, ni coffre, ni audit, ni ACL (#130, #131).
- **Garantie juridique** : chaque promesse doit devenir un **contrôle technique vérifiable**, validé par conseil qualifié (France/UE).

### 2.3 Ce qui est déjà vrai dans le code (et qui est réutilisable)

Le contrat de topologie **existe déjà** et encode une bonne partie de la réponse :

- `crates/hacienda-client-core/src/lib.rs` définit `InstallationConfig::{LocalFull, TeamOnPremise}`, `TrustBoundary::{LocalProcess, AuthenticatedTenantGateway}`, `LanceDbTopology::{LocalEmbedded, ServerTokenOnly, DirectClient}`, `ProviderRoute::{RedactedViaGateway, DirectUnredacted}`, `RehydrationCapability::{LocalEncryptedVault, AuditedServer}`, `EncryptedLocalVault { id, path, key_reference }`, et un `validate()` qui **échoue fermé** sur : `DirectClientToLanceDb`, `InsecureProviderRoute`, `MissingVault`, `MissingTenantScope`, `InsecureEndpoint`, `InvalidStorageTopology`. Le `Debug` masque toute valeur sensible (<redacted>).
- `crates/hacienda-backend/src/lib.rs` : `Backend::new` valide le profil ; `Backend::rehydrate` exécute `validate_rehydration` **avant** tout traitement de token, vérifie le tenant effectif, le `document_id`, et exige que le contenu réhydraté soit **dérivé de la projection token-only stockée** (anti-smuggling de clair). `BackendError::RehydrationDenied` est l’échec unique.
- `apps/hacienda-secrecy-client/src-tauri/src/main.rs` : le profil Local Full est câblé avec `LanceDbTopology::LocalEmbedded` (chemin privé app data), `EncryptedLocalVault` référençant `os-keychain:local-full`, un `OsCredentialKeyStore` (keyring service `org.hacienda.secrecy-client`, entrée `local-full-pseudonym-key-v1`, clé 32 octets) et un `provider_send_enabled` **fail-closed** qui exige un détecteur local **complet** *et* un gateway configuré.
- `crates/hacienda-rag/src/backends/lancedb.rs` : `LanceDbStore` implémente `RagStore` (dense + FTS/BM25 + hybride) ; sparse/late-interaction renvoient `UnsupportedMode` au lieu de mentir sur les capacités.

**Conséquence** : #125 n’est pas « concevoir une topologie » mais **arbitrer entre une topologie déjà esquissée** (frontière différente par profil, data plane token-only) **et des variantes plus strictes**, puis **fermer les trous** qui empêchent aujourd’hui de prétendre à l’invariant unique.

### 2.4 Trous vérifiés aujourd’hui (à fermer, indépendamment de l’arbitrage)

| # | Trou | Ancre |
|---|---|---|
| T1 | **Aucun updater, aucune vérification de signature** dans le client Tauri : `tauri-plugin-updater` et `minisign` absents du `Cargo.lock` ; les modèles sont épinglés par digest mais le binaire applicatif n’est pas protégé par une mise à jour signée. | `apps/hacienda-secrecy-client/src-tauri/Cargo.toml`, `Cargo.lock` |
| T2 | **Aucune rotation ni récupération de clé** dans le client : `rotate`/`recover`/`backup` introuvables ; la clé est créée une fois dans le keyring, pas sauvegardée. Keychain perdu ⇒ tokens irréversibles. | `main.rs` (`OsCredentialKeyStore`) |
| T3 | **Durabilité du coffre non prouvée** : les liaisons document/version/vault vivent en `HashMap` de session ; le rechargement après redémarrage n’est pas établi (#131 addendum). | `main.rs` (`LocalFullDocumentService`) |
| T4 | **Pas de politique réseau/egress vérifiée** au-delà du flag `provider_send_enabled` : la « politique réseau complémentaire » de #121 n’a pas de point d’application dans le client. | `main.rs` |
| T5 | **Audit local** : `FileAuditStore`/Postgres coexistent ; pour un dossier opposable il faut trancher ce que l’audit prouve (motif, document, token, horodatage) sans stocker de clair. Lien direct avec #122. | `hacienda_core::audit` |

---

## 3. Matrice d’emplacements (état cible des deux profils)

| Élément | Local Full | Team On-Premise |
|---|---|---|
| Texte en clair | Mémoire éphémère (viewer ≤ 60 s, worker de redaction à l’ingestion) ; jamais indexé ni persisté en clair | Mémoire serveur scoped + viewer éphémère ; jamais indexé |
| Coffre de pseudonymes | Coffre local chiffré (`EncryptedLocalVault`) ; clé dans keyring OS | Control plane (Postgres/coffre) ; clé derrière KMS/HSM |
| Index RAG (LanceDB) | `LocalEmbedded`, chemin privé app data, **token-only** | `ServerTokenOnly` derrière la passerelle, **token-only**, un namespace/tenant |
| Clés | Keyring OS ; référence `os-keychain:local-full` ; pas de sauvegarde aujourd’hui | KMS/HSM (cible #16) ; jamais en base |
| Passerelle Tauri | Gateway `Local` loopback HTTP | Gateway `AuthenticatedTenantScoped` HTTPS + tenant épinglé |
| Hacienda Engine | In-process dans le client Tauri | Serveur autoritaire (control plane) |

Les cinq chemins de panne, dans cette matrice :

| Panne | Local Full | Team On-Premise |
|---|---|---|
| Serveur indisponible | Sans objet (offline) ; mode dégradé** local | **Fail-closed** : pas de réhydratation, pas d’egress ; à décider si un cache token-only lecture seule est toléré |
| Clé indisponible | Blocage ingestion + egress ; tokens existants **irrécupérables** aujourd’hui | Blocage ; clé KMS = surface de récupération à concevoir |
| Politique absente | Doit échouer fermé (pas de défaut permissif) | Idem, au niveau passerelle |
| Mise à jour non vérifiée | **Trou T1** : aucune signature de binaire | T1 côté client léger + T1 côté serveur |
| Poste compromis | Non couvert (voir Option B) | Non couvert pour le poste ; partiellement borné par le serveur |

---

## 4. Options de topologie

Quatre options, de la plus stricte à la plus délégatrice. Elles ne sont pas exclusives au niveau du *data plane* (A et C partagent le même), mais elles divergent sur **où s’arrête la confiance**.

### Option A — « Confiance zéro à la donnée, autorité au control plane » (recommandée comme invariant)

Le data plane (LanceDB, embeddings, FTS, sauvegardes) **n’est jamais de confiance** : il ne contient que du token-only et des métadonnées non secrètes. La confiance est portée exclusivement par le **control plane** : processus local en Local Full, passerelle de tenant authentifiée en Team. Le texte en clair n’existe qu’en mémoire éphémère côté propriétaire des octets ; la clé de réversibilité ne touche jamais le data plane.

- **Xberg** : exécuté du côté propriétaire des octets (in-process local, Engine en Team), redaction avant projection.
- **LanceDB** : `LocalEmbedded` token-only / `ServerTokenOnly` token-only. Un blob chiffré (original) reste **hors** de la table recherchable.
- **Réhydratation** : un **seul contrat** (pseudonymes Hacienda stables), capacité courte, éphémère, auditée, human-ward. Xberg `RehydrationMap` = option document-local explicitement marquée, jamais cross-document.
- **Clés** : keyring OS (Local) / KMS/HSM (Team) ; sinon l’invariant tombe.

**Menaces couvertes** : fournisseur, équipe, admin de data plane, fuite d’index/sauvegarde/log/export, cross-tenant, configuration permissive.
**Menaces non couvertes** : poste compromis (malware/root) dans les deux modes ; compréhension partielle du détecteur (capture ≠ détection, cf. #133) ; fournisseur qui corrèle des tokens (fuite métadonnées).
**Risques** : coût de mise en œuvre (KMS, updater signé, rotation, durabilité du coffre) ; nécessité de secret utilisateur pour la réhydratation locale, sinon le keyring seul ne protège pas d’un code local.

### Option B — « Confiance portée : processus local en Local Full, passerelle de tenant en Team »

C’est **exactement ce que le contrat actuel encode** : `TrustBoundary::LocalProcess` pour Local Full et `AuthenticatedTenantGateway` pour Team. Le poste mono-utilisateur est une frontière légitime ; le serveur est la frontière de l’organisation. L’invariant est *le même énoncé*, mais la **garantie repose sur l’honnêteté du poste** en mode local.

- Identique à A sur le data plane token-only, mais assume que le **processus local est de confiance** (texte en clair peut y résider le temps de la session, le coffre est déverrouillé par le keyring).
- Simple, aligné sur l’existant, excellent pour l’offline.

**Menaces couvertes** : fournisseur, équipe (en Team), index/sauvegarde/log, cross-tenant, export.
**Menaces non couvertes** : **poste compromis en Local Full** — un code local peut lire la mémoire et le keyring ; un admin local peut exfiltrer le coffre.
**Risques** : promettre un « invariant identique » tout en le reposant sur une hypothèse (poste honnête) qui n’est pas vraie dans le mode où il est le plus faible. Doit être **écrit noir sur blanc** comme limite, pas dissimulé.

### Option C — « Confiance déléguée au serveur (topologie unique autoritaire) »

Même le Local Full passe par un Engine (embarqué ou sidecar) qui est **la seule autorité** ; le client est toujours léger. Une seule topologie, un seul jeu de garanties.

- Corrige la difficulté d’un invariant à deux frontières en supprimant la seconde.
- **Casse l’exigence « local indépendant » et le fonctionnement hors ligne** de #118/#131 : le client ne peut plus travailler sans Engine.
- Crée un point de défaillance unique et un coût de packaging (sidecar, ressources).

**Menaces couvertes** : uniformité forte, cross-tenant, admin.
**Menaces non couvertes** : le serveur lui-même (admin/compromission) ; disponibilité hors ligne.
**Risques** : contradiction produit ; probablement à rejeter, mais utile comme **borne** : elle montre ce qu’on gagne (uniformité) et ce qu’on perd (offline).

### Option D — « Confiance document-local (Xberg RehydrationMap) + pseudonymes stables »

Variante hybride : en Local Full, les dossiers isolés utilisent la `RehydrationMap` Xberg **document-local** (map chiffrée dans le coffre local, token non corrélé entre documents) ; le multi-document et le mode Team utilisent le contrat Hacienda stable.

- Utile pour un dossier ponctuel où la non-corrélation inter-documents est un **avantage** de confidentialité.
- Introduit **deux contrats de réhydratation** et deux cycles de vie de clé ; risque de « révéler un token Hacienda avec une map Xberg » (interdit par #131).

**Menaces couvertes** : corrélation inter-documents (par construction), fournisseur, index.
**Menaces non couvertes** : confusion de contrats, erreur d’usage, perte de la map ; ne protège pas plus du poste compromis.
**Risques** : complexité de surface ; à n’admettre que comme **mode explicite, marqué, Local Full uniquement**, jamais en Team.

### 4.1 Synthèse

| Critère | A (zéro data plane) | B (frontière par profil) | C (serveur unique) | D (document-local) |
|---|---|---|---|---|
| Invariant identique dans les 2 modes | **Oui** (data plane) | Oui, mais hypothèse locale | Oui (uniformisé) | Partiel |
| Data plane jamais de confiance | **Oui** | Oui | Oui | Oui |
| Offline Local Full | **Oui** | **Oui** | Non | Oui |
| Poste compromis couvert | Non (borné) | Non | Non (déplacé au serveur) | Non |
| Un seul contrat de réhydratation | **Oui** | Oui | Oui | Non |
| Coût de mise en œuvre | Élevé | Faible | Moyen/élevé | Moyen |
| Aligné sur le code actuel | Partiel (à étendre) | **Oui** | Non | Partiel |

---

## 5. Questions de grilling (à trancher par le product owner)

Chaque question attend une décision. Une recommandation est proposée comme point de départ.

1. **L’invariant unique est-il formulé au niveau du data plane (aucun clair/clé dans l’index, les logs, les sauvegardes, les exports, les fournisseurs) et accepte-t-on que le *point d’application* diffère selon le profil ?**
   *Reco : oui — c’est la seule formulation qui survive aux deux modes ; l’énoncé doit être écrit une fois et cité partout.*

2. **En Local Full, l’actif protégé est-il l’utilisateur légitime (contre fournisseur/équipe/index) ou aussi contre un poste compromis (malware/root) ?**
   *Reco : protéger contre fournisseur/équipe/index, et **documenter le poste compromis comme hors périmètre** plutôt que de promettre l’impossible.*

3. **Le coffre local peut-il se déverrouiller avec le keyring OS seul, ou exige-t-on un secret utilisateur (passphrase/biométrie) pour la réhydratation ?**
   *Reco : keyring pour l’ingestion, **secret utilisateur pour la réhydratation** ; sans cela, un code local lit la clé.*

4. **Clé indisponible : perte de données irréversible assumée, ou sauvegarde/escrow de clé ?**
   *Reco : aucune perte silencieuse — exiger une **sauvegarde de clé chiffrée explicite** (escrow local ou d’équipe) avant de prétendre à la durabilité (T2).*

5. **Rotation : additive (les `retired` restent révélables) ou destructive ? À quelle fréquence, déclenchée par qui ?**
   *Reco : additive, alignée sur le contrat Hacienda existant ; seuil de temps **et** révocation sur incident ; ne pas construire sur `EnvKeyResolver` (cf. #122).*

6. **Serveur indisponible en Team : fail-closed total, ou cache local token-only en lecture seule ?**
   *Reco : **fail-closed** pour toute réhydratation et tout egress ; cache token-only en lecture seule autorisé uniquement s’il ne contient ni clair ni clé et s’il est marqué « non autoritaire ».*

7. **Politique absente (redaction/ACL/réseau) : confirme-t-on le fail-closed strict, y compris à la première ingestion ?**
   *Reco : oui — **aucun défaut permissif** ; l’absence de politique bloque l’egress et la réhydratation (T4).*

8. **Mise à jour : exige-t-on binaires signés et modèles épinglés par digest avant tout egress ? Qui signe, où vit la clé de vérification ?**
   *Reco : oui, obligatoire (T1) ; signature par clé projet, clé publique épinglée dans le binaire ; une mise à jour non vérifiée **invalide la frontière** et force le mode dégradé.*

9. **Poste compromis : on l’assume hors périmètre, ou on tente de le borner (confinement du worker de redaction, plaintext strictement éphémère, secret utilisateur) ?**
   *Reco : borner ce qui peut l’être (worker confiné, éphémère, secret utilisateur) et **mesurer la limite** dans le prototype #133 (marqueurs, évasion), pas la promettre.*

10. **LanceDB : un blob chiffré (original) peut-il cohabiter avec l’index dans le même store, ou exige-t-on une séparation physique ?**
    *Reco : **séparation** — data plane = table recherchable token-only ; originaux dans le coffre/object store, référencés de façon opaque.*

11. **Xberg `RehydrationMap` : autorisée en Local Full (dossiers isolés) et interdite en Team, ou un seul contrat partout ?**
    *Reco : un seul contrat (Hacienda stable) par défaut ; map Xberg = **option Local Full document-local explicitement marquée**, jamais cross-document, jamais en Team.*

12. **Réhydratation en Team : serveur-side obligatoire, ou un client peut-il détenir une clé de tenant ?**
    *Reco : **serveur-side** ; le client n’a qu’une capacité courte et un audit, jamais la clé (le contrat actuel de `Backend::rehydrate` le permet déjà).*

13. **Double approbation : quelles catégories/dossiers/seuils, et qui approuve en Local Full (utilisateur unique) ?**
    *Reco : double approbation pour catégories strictes et divulgation de masse en Team ; en Local Full mono-utilisateur, **confirmation renforcée + justification obligatoire** (position #122/#120).*

14. **Tenant en Local Full : tenant `default` unique, ou plusieurs coffres/tenants sur un même poste ?**
    *Reco : mono-tenant par poste pour le premier lot, avec **isolation par coffre** ; le multi-coffre est une extension à spécifier si le besoin cabinet apparaît.*

15. **Audit local : FileAuditStore simple ou chaîné/scellé, et que doit-il contenir pour être opposable sans stocker de clair ?**
    *Reco : chaîné, attribuable, portant motif + document + token + horodatage + capability ; jamais le clair. Trancher avec #122/#133.*

16. **Synchronisation Local Full → Team (opt-in) : autorisée telle quelle, ou acte explicite audité ?**
    *Reco : **acte explicite et audité** ; un dossier local ne change jamais de tenant sans décision humaine (anti-fuite).*

17. **Quel chemin de panne le prototype #125 doit-il démontrer en premier pour trancher ?**
    *Reco : les cinq, mais prioriser **clé indisponible** et **politique absente** (fail-closed structurel) avant **poste compromis** (limite à documenter).*

18. **Preuve : le prototype doit-il produire une matrice de non-egress et de non-clair par mode, comparable à #133 ?**
    *Reco : oui — même exigence de contrôle technique vérifiable (marqueurs synthétiques, observateur indépendant), appliquée à LanceDB, logs, exports et fournisseur.*

---

## 6. Recommandation argumentée

**Adopter l’Option A comme invariant, l’Option B comme frontière par profil, et refuser C et D par défaut.**

1. **Un invariant, deux frontières, nommées honnêtement.** L’invariant unique est celui du data plane (§2.1) : aucun clair ni clé dans l’index, les logs, les sauvegardes, les exports ou les fournisseurs. Le point d’application est le processus local en Local Full et la passerelle de tenant authentifiée en Team — c’est déjà le contrat de `hacienda-client-core`. On ne réécrit pas la topologie, on **cesse de laisser croire que les deux frontières offrent la même résistance** : la limite « poste compromis » doit être écrite.

2. **Le data plane reste zéro-confiance dans les deux modes.** LanceDB est `LocalEmbedded` token-only ou `ServerTokenOnly` token-only, jamais adressé directement par le client, jamais porteur d’originaux ni de clés. C’est cohérent avec #130/#131 et déjà validé par `validate()` (`DirectClientToLanceDb` échoue fermé).

3. **Un seul contrat de réhydratation.** Pseudonymes Hacienda stables par tenant/profil, capacité courte, éphémère, auditée, human-ward, re-dérivée de la projection token-only stockée (déjà dans `Backend::rehydrate`). Xberg `RehydrationMap` n’est admise qu’en Local Full document-local, marquée, jamais cross-document. On évite ainsi le trou « deux contrats concurrents » relevé en #122.

4. **Fermer les quatre trous avant de prétendre à l’invariant.** Sans (T1) mise à jour signée, (T2) rotation/récupération de clé, (T3) durabilité du coffre, (T4) politique réseau/egress appliquée, l’invariant est un slogan. Ce sont des fermetures d’écart **indépendantes de l’arbitrage** et immédiatement planifiables.

5. **Fail-closed sur les cinq chemins de panne.** Serveur indisponible, clé indisponible, politique absente, mise à jour non vérifiée : blocage, pas de fallback. Poste compromis : hors périmètre déclaré, borné par le confinement et l’éphémérité, mesuré par la preuve (#133), jamais promis.

6. **Prototype minimal pour trancher #125** (le « prototype doit comparer » de l’issue) :
   - une **matrice d’emplacements** des six éléments sur les deux profils ;
   - un **test de non-egress / non-clair** sur LanceDB, logs, exports et fournisseur, par profil ;
   - un **test cross-tenant** que deux tenants ne se réhydratent pas réciproquement ;
   - les **cinq scénarios de panne** exécutés, avec la décision fail-closed observée ;
   - un **test de refus** que `DirectClient`→LanceDB et `DirectUnredacted` sont bien inconstructibles.

**Ce que cette recommandation ne résout pas** (à laisser à l’arbitrage #125/#120) : le choix du secret utilisateur vs keyring seul pour la réhydratation locale, la politique exacte de double approbation, l’escrow de clé, et le seuil de « divulgation de masse ». Ces points viennent du modèle de menace (#120) et du contrat de réhydratation (#122), pas de cette note.

---

*Note de grilling en lecture seule sur `origin/main` = `db8d895`. Aucun fichier tracké modifié, aucun commit, aucune PR. Ancre principale non versionnée : `crates/hacienda-client-core/src/lib.rs` (contrat de topologie), complétée par `crates/hacienda-backend/src/lib.rs`, `crates/hacienda-rag/src/backends/lancedb.rs`, `apps/hacienda-secrecy-client/src-tauri/src/main.rs` et les recherches #130/#131.*

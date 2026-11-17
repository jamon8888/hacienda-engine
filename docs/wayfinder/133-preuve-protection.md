# 133 — Définir la preuve de protection des applications IA officielles

> Note de prototype (PROTOTYPE-ANALYSIS) — issue wayfinder #133, carte parente #118.
> Document de travail non versionné. En lecture seule sur le code : `apps/hacienda-secrecy-client/`
> (Tauri + GLiNER2 + coffre local, profil LocalFull).

## 1. Définition : ce que « preuve de protection » doit couvrir

La preuve de protection n'est **pas** une attestation d'architecture (« le proxy est en place »).
Elle est une **preuve bornée au test** : une démonstration reproductible, application par
application et canal par canal, qu'aucun contenu confidentiel en clair ne quitte la machine
de l'utilisateur sans avoir traversé la chaîne de protection Hacienda.

### 1.1 Assertions visées (ce que la preuve doit établir)

Pour chaque paire (OS, application officielle, canal réseau) de la matrice, la preuve doit
établir que, **pendant la fenêtre de test** et **dans les conditions de test** :

1. **Couverture d'interception** — tout égrégat réseau produit par le processus cible
   (principal, auxiliaires, enfants) est capturé par le redirecteur OS. Aucun chemin de
   sortie (TCP, UDP/QUIC, WebSocket, flux préexistants, changement de PID) ne contournait
   le proxy.
2. **Décodage complet** — chaque flux capturé a été décodé et inspecté (TLS résolu,
   HTTP/1.1, HTTP/2, SSE, WebSocket, multipart). Un flux capté mais non décodé **n'est pas
   couvert** et doit apparaître comme tel dans la matrice.
3. **Transformation effective** — pour les flux couverts, la redaction/pseudonymisation a
   bien été appliquée **avant** l'émission vers le fournisseur : aucun marqueur synthétique
   brut n'est observé en sortie.
4. **Fail-closed** — à la panne (arrêt brutal du redirecteur, du proxy, du détecteur ;
   verrouillage du coffre ; format inconnu ; redémarrage client), les sorties visées
   restaient **bloquées** jusqu'au rétablissement de la protection. Aucun fallback direct.
5. **Absence de boucle** — le proxy lui-même n'est pas intercepté (pas de boucle
   d'interception), et la couche de contrôle des sorties indépendante bloque l'émission
   directe.
6. **Périmètre honnête** — ce qui n'est **pas** couvert (canal non inspecté, store cloud
   déjà détenu par le fournisseur, version client non testée) est listé explicitement.
   La preuve est bornée à la matrice testée ; elle ne promet pas de protection universelle.

### 1.2 Ce que la preuve ne couvre PAS (limites intrinsèques)

- **Capture ≠ détection** : un contenu capturé peut échapper au détecteur (GLiNER2). La
  preuve mesure ce qui a été **observé**, pas ce qui a été **compris**.
- **Versions** : toute mise à jour de l'application officielle (nouveau canal, nouveau
  port, nouveau protocole, certificat épinglé) **invalide** la preuve pour cette version.
  La preuve est indexée par version client/OS/canal.
- **Sources cloud** : les données déjà présentes côté fournisseur échappent au contrôle
  local ; c'est une limite distincte, non fermée par ce prototype.
- **Garantie juridique** : la preuve est un **contrôle technique vérifiable**, à valider
  par conseil qualifié en France/UE ; elle n'est pas une certification de conformité.

### 1.3 Critères de la preuve (acceptation)

La preuve est considérée **adéquate pour la décision** si elle produit :

- une **matrice** version/OS/canal : capturé, décodé, transformé, bloqué, non couvert ;
- un **journal de marqueurs** : marqueurs synthétiques uniques injectés par canal,
  observés par un mécanisme **indépendant** du journal du proxy ;
- un **scénario de panne** exécuté et filmé/journalisé : zéro fuite pendant la fenêtre
  de dégradation ;
- un **procédure de reproduction** : script, versions épinglées, commandes, durée.

## 2. Mécanismes candidats de preuve

### 2.1 Attestation de chaîne (attestation)

L'application Hacienda produit une **attestation signée** (locale, par la clé du coffre)
qui certifie, pour une session donnée : la liste des processus cibles, les canaux
routés, l'état de chaque adaptateur (capturé/décodé/transformé/bloqué), l'absence
d'exception durant la fenêtre, le hash de la matrice. L'attestation est présentée à
l'utilisateur (et, si besoin, au conseil) comme preuve de bon fonctionnement.

#### Forces

- Facile à produire : le proxy connaît déjà l'état de sa propre matrice.
- Fournit un artefact lisible, horodaté, traçable (liée au journal).
- S'articule naturellement avec l'audit existant du client secrecy (audit sans contenu,
  hash blake3 des spans, action/catégorie).

#### Faiblesses

- **Auto-référentielle** : l'attestation est produite par le même système dont on veut
  prouver la protection. Un proxy en panne silencieuse, une régression de décodage,
  ou un contournement inconnu ne sont **pas détectés** par l'attestation elle-même.
- Ne prouve pas l'absence de fuite ; elle atteste l'état interne rapporté.
- Risque d'être perçue (à tort) comme une garantie forte, alors qu'elle n'est qu'un
  rapport d'état.

### 2.2 Journal scellé (sealed log)

Le proxy et la couche de contrôle des sorties écrivent dans un **journal append-only,
scellé** (chaîne de hash, hash de commitment périodique, signature locale) couvrant :
chaque flux (métadonnées du processus, destination, protocole, taille), chaque décision
(allow/block/transform), chaque exception, chaque marqueur émis/observé. Le scellement
rend détectable toute altération a posteriori.

#### Forces

- **Indépendance face à l'altération** : on peut vérifier après coup que le journal n'a
  pas été modifié (intégrité).
- Supporte l'audit : le conseil ou un tiers peut exiger la production du journal d'une
  session.
- Couvre le **temporel** : on peut reconstruire la chronologie exacte des décisions
  (utile en incident : « que s'est-il passé quand le proxy a gelé ? »).
- S'aligne avec l'existant : le client secrecy a déjà un `AuditStore` append-only sans
  contenu ; le journal scellé en est l'extension au niveau réseau.

#### Faiblesses

- Le journal est écrit **par le système accusé** : un contournement total (processus
  qui ne passe jamais par le proxy) n'y laisse **aucune trace** — l'absence n'est pas
  prouvée, seulement la présence des événements observés.
- Le scellement local (clé de la machine) ne résiste pas à un adversaire qui contrôle
  la machine ; c'est une preuve d'intégrité, pas une preuve de non-évasion.
- Volume : un journal complet par flux est volumineux ; il faut des règles de rétention
  et de minimisation (pas de contenu, seulement métadonnées + hash).

### 2.3 Vérification indépendante (marqueurs + observateur indépendant)

On injecte dans **chaque** canal des **marqueurs synthétiques uniques** (valeurs PII
fictives, trames SSE, uploads multipart). En sortie, un **observateur indépendant** —
mécanisme de capture distinct du proxy (ex. un second tap réseau, un serveur miroir de
fournisseur, un analyseur de paquets en parallèle) — cherche la présence de ces marqueurs
bruts. **Zéro marqueur brut observé** dans la matrice testée = preuve bornée de
transformation/blocage.

#### Forces

- **Indépendance réelle** : l'observateur n'est pas le proxy ; il observe l'effet, pas
  l'intention. C'est le seul mécanisme qui détecte un contournement **silencieux**
  (flux qui contourne le proxy, flux capté mais non transformé).
- **Test actif** : on ne se contente pas de lire l'état ; on cherche à faire fuir.
  C'est une preuve par tentative d'évasion, la plus proche d'une « preuve de protection ».
- Bornée et honnête : elle ne promet que ce qui a été testé (matrice explicite), ce qui
  correspond exactement à l'exigence de l'issue (« preuve bornée au test, pas une
  garantie universelle »).

#### Faiblesses

- **Coût** : il faut un observateur indépendant opérationnel (second tap, miroir de
  fournisseur) ; c'est de l'infrastructure de test à construire.
- **Couverture du test = couverture de la preuve** : un canal non injecté de marqueurs
  n'est pas prouvé. La matrice doit être exhaustive, ce qui est long (versions, OS,
  protocoles).
- **Échouement partiel** : si un marqueur fuite sur un canal, la preuve s'effondre pour
  ce canal (ce qui est le bon comportement — c'est exactement ce qu'on veut détecter).
- Ne prouve rien sur les canaux non testés ni sur les versions futures.

### 2.4 Synthèse

| Mécanisme | Détecte un contournement silencieux | Preuve d'absence de fuite | Coût | Rôle |
|---|---|---|---|---|
| Attestation | Non | Non | Faible | Rapport d'état, lisibilité |
| Journal scellé | Partiel (altération du journal seulement) | Non (absence non prouvable) | Moyen | Audit, intégrité temporelle |
| Vérification indépendante | **Oui** | **Oui, bornée au test** | Élevé | **Cœur de la preuve** |

## 3. Questions de grilling (stress-test)

1. Si le proxy lui-même est compromis (bug, régression, ou malveillant), quel mécanisme
   ci-dessus le détecte ? L'attestation non, le journal scellé seulement si l'altération
   est visible — la vérification indépendante, oui. **La preuve repose-t-elle trop sur
   l'hypothèse que le proxy est honnête ?**

2. L'observateur indépendant est lui-même sur la même machine, sous le contrôle de
   l'utilisateur. Un adversaire (l'application officielle qui met à jour, un processus
   malveillant local) peut-il **désactiver ou aveugler** l'observateur sans que la
   preuve s'en aperçoive ?

3. « Zéro marqueur brut observé » — que se passe-t-il si l'observateur **ne voit aucun
   marqueur du tout** (ni transformé ni brut) ? Est-ce que cela prouve le blocage, ou
   simplement que l'observateur est mal branché ? Faut-il un **marqueur de contrôle**
   que l'on sait fuir volontairement pour valider l'observateur ?

4. La matrice version/OS/canal est bornée. **Qu'est-ce qui invalide une preuve** quand
   Claude Desktop ou ChatGPT Desktop sort une mise à jour mineure ? Faut-il un
   **sentinelle de version** (hash du binaire, version détectée) qui force la
   re-validation, ou la preuve devient-elle obsolète silencieusement ?

5. Le client secrecy est **fail-closed** (envi vers le fournisseur désactivé si la
   détection locale n'est pas complète). La preuve de protection des apps officielles
   doit-elle exiger le **même** niveau de fail-closed au niveau réseau (blocage OS des
   sorties) — et si oui, quel mécanisme OS garantit ce blocage sans dépendre du proxy
   (Network Extension / WinDivert) ?

6. Le journal scellé est écrit par le système qu'il est censé auditer. **Quelle est la
   frontière minimale d'indépendance** acceptable : le journal est-il écrit par le proxy,
   par la couche de contrôle des sorties, ou par un **tiers process** distinct ? Un
   journal écrit par le proxy n'est-il pas déjà suspect ?

7. L'attestation signée par la clé du coffre local : la clé est sur la machine de
   l'utilisateur. **Un tiers peut-il faire confiance à cette attestation** sans
   contrôler la machine ? Si non, l'attestation n'est-elle qu'un artefact utilisateur
   (utile pour l'UI et le conseil) et pas une preuve au sens fort ?

8. Le prototype doit distinguer « gateway Anthropic-compatible » (Claude Code) et
   « interception de Claude Chat » (Desktop). **La preuve doit-elle produire des
   matrices séparées par application** (Claude Desktop, ChatGPT Desktop, Claude Code)
   ou une matrice unifiée par canal ? Un succès sur une app prouve-t-il quoi que ce
   soit sur l'autre ?

9. « Capture complète ≠ détection complète » : GLiNER2 peut rater des entités. **La
   preuve doit-elle inclure un test de complétude de détection** (jeu de marqueurs PII
   variés, taux de détection mesuré) en plus du test de transformation ? Et si le taux
   est < 100 %, la preuve est-elle un échec, ou un avertissement borné ?

10. Les marqueurs synthétiques sont des **fausses PII** injectées par le test.
    **Comment s'assurer qu'aucun marqueur ne persiste** dans les stores (RAG, coffre,
    index) après le test, et qu'aucun marqueur réel ne se faufile dans la production ?
    Faut-il un **mode « marqueurs »** distinct du mode normal, désactivable et vérifiable ?

11. La preuve est bornée au test. **Qui consomme cette preuve** : l'utilisateur (UI),
    le conseil juridique (dossier), un futur tiers certificateur ? Chacun a un besoin
    de granularité différent. La preuve doit-elle être **multi-niveaux** (résumé
    utilisateur / matrice complète / journal brut) ?

12. Coût et durée : une matrice exhaustive (2 apps × 2 OS × N canaux × M versions) est
    longue. **Faut-il définir un sous-ensemble minimal** (première paire OS/application
    de l'issue) qui suffit à la décision Wayfinder, et traiter le reste comme
    extension ? Quel est le **seuil de couverture** qui rend la preuve acceptable pour
    avancer ?

13. L'issue exige une **couche de contrôle des sorties indépendante** pour empêcher le
    contournement en cas de panne. **Cette couche est-elle elle-même prouvée** par la
    vérification indépendante, ou est-elle un postulat ? Si le proxy tombe, est-ce la
    couche indépendante qui bloque, et comment le test le démontre sans ambiguïté ?

14. **Reproductibilité** : la preuve doit être reproductible. Qui peut la reproduire
    (l'équipe, le conseil, un tiers) ? Faut-il un **script de preuve** versionné et
    épinglé (versions des libs, hash des binares, OS) qui génère la matrice + le
    journal + l'attestation en une passe ?

15. Si la vérification indépendante **échoue** sur un canal (un marqueur fuite),
    **quelle est la conséquence** : blocage de l'app concernée, dégradation du mode,
    échec du prototype ? La preuve doit-elle définir à l'avance un **seuil de
    tolérance** (zéro fuite tolérée, ou N canaux tolérés avec blocage) ?

## 4. Recommandation

**Adopter une preuve en trois couches, hiérarchisée par fiabilité, et pas un mécanisme
unique.**

1. **Cœur de la preuve = Vérification indépendante (2.3).** C'est le seul mécanisme qui
   démontre l'effet (aucun marqueur brut en sortie) plutôt que l'intention (état
   interne du proxy). Construire l'observateur indépendant comme premier livrable du
   prototype : un tap réseau parallèle + un miroir de fournisseur qui cherche les
   marqueurs. Sans lui, les deux autres mécanismes sont auto-référentiels.

2. **Socle d'audit = Journal scellé (2.2), écrit par la couche de contrôle des
   sorties, pas par le proxy.** Cela répond au grilling #6 : la frontière
   d'indépendance minimale est que le journal soit tenu par un processus distinct du
   proxy, qui reçoit les décisions (allow/block/transform) et les scelle. Le proxy ne
   peut pas écrire son propre audit de non-évasion.

3. **Surcouche de lisibilité = Attestation (2.1), clairement étiquetée comme rapport
   d'état.** L'attestation agrège matrice + journal + résultat de la vérification
   indépendante en un artefact signé et lisible, pour l'UI utilisateur et le dossier
   conseil. Elle **référence** la preuve indépendante ; elle ne la remplace pas.

**Périmètre minimal pour la décision Wayfinder** (réponse au grilling #12) : ne pas
viser la matrice exhaustive. Choisir la **première paire OS/application** (macOS +
Claude Desktop, canal Chat) comme périmètre de preuve minimal. La preuve est
acceptable pour la décision si, sur cette paire :

- la matrice capture/décodage/transformation/blocage est complète et honnête
  (non couvert explicitement listé) ;
- la vérification indépendante observe **zéro marqueur brut** sur tous les canaux
  testés de cette paire ;
- le scénario de panne (arrêt brutal, coffre verrouillé, format inconnu) démontre le
  fail-closed (sorties bloquées, pas de fallback) ;
- un **marqueur de contrôle** (fuite volontaire) valide que l'observateur voit
  bien ce qui doit fuir (réponse au grilling #3).

**Garde-fous transverses** :

- **Mode « marqueurs » distinct** du mode normal, activable uniquement en contexte de
  preuve, avec purge vérifiable des marqueurs des stores après test (grilling #10).
- **Sentinelle de version** : la preuve est indexée par version client/OS ; toute
  mise à jour de l'app invalide la preuve et force la re-validation (grilling #4).
- **Multi-niveaux** : la preuve produit un résumé utilisateur, une matrice complète,
  et un journal brut — consommés par l'UI, le conseil, et un futur tiers
  respectivement (grilling #11).
- **Échec = blocage** : si la vérification indépendante détecte une fuite sur un
  canal, ce canal est bloqué et la preuve est en échec pour lui ; zéro fuite tolérée
  en mode strict (grilling #15).

**Ce que cette recommandation ne résout pas** (à laisser à la décision Wayfinder) :
le choix de la première paire OS/application, la construction détaillée de l'observateur
indépendant (tap OS vs miroir fournisseur), et le seuil de complétude de détection
GLiNER2 acceptable. Ces points relèvent du prototype proprement dit, pas de cette
définition.

---

*Références : issue #133 (protocole de preuve), carte #118 (séparation stricte
clair/pseudonymisé/coffre/RAG/fournisseurs ; garantie = contrôle technique vérifiable),
`apps/hacienda-secrecy-client/` (fail-closed, audit sans contenu, clé en keyring OS,
modèles épinglés, viewer 60 s, capacités one-shot).*

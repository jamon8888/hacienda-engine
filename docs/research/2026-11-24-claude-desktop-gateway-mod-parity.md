# Claude Desktop gateway et rendu Mod : parité avec Codex

**Date de recherche :** 2026-11-24
**Périmètre :** sources officielles Anthropic actuelles via Context7 et état du dépôt indexé par XERJ. Il s’agit d’une analyse de conception, pas d’une implémentation.

## Résumé

La voie Claude Desktop est techniquement plausible pour la parité d’affichage : le réglage d’inférence tiers de Desktop peut router Claude Code vers un gateway, et Claude Code Mods expose un point `ui.render` pour `AssistantMessage` explicitement présent sur la surface `desktop` (l’onglet Code). Ce rendu est une projection visuelle seulement : il ne change ni le message enregistré ni le contexte du modèle. Le gateway existant du dépôt est déjà un point de contrôle approprié pour inspecter et pseudonymiser les messages avant l’egress. La valeur originale doit toutefois rester dans Hacienda et ne jamais être remise au Mod comme une table persistante ou comme texte destiné à être réinjecté dans Claude.

Cette conclusion s’applique à Claude Code dans l’onglet Code de Claude Desktop, pas à tous les messages de conversation Claude Desktop. La configuration locale/admin du gateway peut aussi désactiver l’environnement Anthropic hébergé et Remote Control selon la documentation du gateway.

## Configuration gateway de Claude Desktop

Les docs Claude Code distinguent les surfaces : le desktop app lit son routage depuis la **third-party inference configuration**, pas depuis `ANTHROPIC_BASE_URL` ni `settings.json`. Un administrateur peut déployer la configuration ; sinon l’utilisateur l’entre dans **Developer → Configure Third-Party Inference** après avoir activé Developer Mode sous Help → Troubleshooting. Une configuration administrée prend le pas sur le formulaire local. Lorsque cette configuration est active, les environnements cloud hébergés par Anthropic disparaissent du sélecteur et Remote Control est indisponible. [Anthropic — Connect Claude Code to a gateway, “Configure each surface > Desktop app”](https://code.claude.com/docs/en/llm-gateway-connect)

Le protocole du gateway expose le format Anthropic Messages : `/v1/messages`, avec `/v1/messages/count_tokens` facultatif. Pour les clients CLI configurés avec `ANTHROPIC_BASE_URL`, Anthropic demande de transmettre sans modification `anthropic-beta` et `anthropic-version`, et `Authorization` si une connexion claude.ai l’emploie ; les requêtes d’inférence utilisent `POST /v1/messages?beta=true`. Le document desktop indique une configuration de gateway dédiée à l’application : ne pas déduire de la variable d’environnement CLI que le formulaire Desktop est configuré. [Anthropic — Gateway protocol](https://code.claude.com/docs/en/llm-gateway-protocol)

**Conséquence pour Hacienda :** le gateway existant est compatible avec le point d’entrée Messages à la forme générale, mais le branchement Desktop doit être documenté/validé contre le protocole effectivement envoyé par la version Desktop ciblée (headers, query `beta`, streaming et corps). Le gateway doit continuer de refuser corps ou champs qu’il ne peut pas inspecter ; adapter le client ne doit pas contourner cette barrière.

## Contrat `ui.render` et surface prise en charge

Les types du Mod déclarent `AssistantMessage` dans l’union `RenderComponent`. `RenderPropsOf['AssistantMessage']` fournit `text` (markdown d’un bloc texte assistant), `isFirstOfReply` et `onScreen` en lecture seule. Le texte peut être réécrit pour le dessin. La documentation du type précise que cette réécriture change le rendu seulement et laisse le message stocké intact. `ui.render` s’exécute avant le dessin et le hook peut retourner `next(e)` avec des props réécrites ou produire son propre arbre. [Types officiels des Mods Claude Code](https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts) ; [Anthropic — Modify existing interface](https://code.claude.com/docs/en/plugins/mods/interface) ; [Anthropic — Mods reference](https://code.claude.com/docs/en/plugins/mods/reference)

La même déclaration de types nomme les surfaces `terminal`, `desktop`, `mobile`, `vscode`, et décrit `desktop` comme Claude Code Desktop, l’onglet Code. Elle dit aussi que le rendu est demandé par la surface distante et que celle-ci dessine l’arbre à l’emplacement prévu. Ainsi, l’extension documentée permet de restyler les blocs assistant dans l’onglet Code de Desktop ; elle ne démontre pas un point d’extension pour le transcript général de Claude Desktop ou ses autres surfaces. [Types officiels des Mods Claude Code](https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts)

L’API couvre un bloc `AssistantMessage` à la fois. Un Mod doit préserver la structure markdown et le comportement de continuation de `next`, gérer les rendus répétés/viewport, et ne pas présumer qu’un tour complet sera livré dans un unique callback. La compatibilité est à vérifier avec la version Desktop/Claude Code réellement déployée ; le dépôt des types sur `main` est une source vivante.

## Sécurité de la carte locale de redaction

Le rendu local réduit l’exposition **au modèle et au transcript**, mais n’est pas à lui seul une frontière de sécurité. Si le Mod reçoit les valeurs originales et stocke une carte `token → plaintext`, tout code ayant accès à son contexte d’exécution peut potentiellement lire ces valeurs, les journaliser, les exporter ou les exposer via son UI. La carte peut aussi survivre au tour, à la session ou au changement de tenant si son cycle de vie n’est pas borné. La politique de plugins et le consentement d’installation sont donc des contrôles de confiance pertinents, mais ne remplacent pas l’autorisation du serveur.

Architecture recommandée : le gateway renvoie au client uniquement des pseudonymes ; la carte réversible reste dans le backend/vault Hacienda, scoppée au tenant et au tour. Le Mod identifie les tokens connus dans le texte assistant puis demande au pont local une projection segmentée via un handle opaque/capacité courte et liée à la session. La réponse de révélation ne devrait exister qu’en mémoire éphémère au moment du dessin, sans être écrite dans les logs, l’historique Claude, les arguments d’outil ou le prochain prompt. Autoriser par token et par session, expirer/révoquer, journaliser digest et longueur sans plaintext, et échouer en conservant le pseudonyme si l’autorisation ou la projection échoue. Une révélation ne doit jamais être renvoyée à l’API Messages.

Cette proposition concorde avec l’existant : la note de frontière desktop décrit le rendu Claude comme visuel seulement et insiste sur les références opaques et capacités de viewer ; le backend possède un `VaultRehydrationAdapter` qui vérifie une capacité de coffre local, résout par tenant et écrit un audit de révélation digest-only avant de retourner du texte éphémère. XERJ : `docs/research/2026-11-24-native-desktop-pii-boundary.md:39` et `crates/hacienda-backend/src/lib.rs:362`.

## Interaction avec les routes Messages et le modèle viewer Codex

Le handler `POST /v1/messages` exige une identité authentifiée et la capacité `documents:process`, valide les chemins texte inspectables, redige le corps avant de construire la requête provider, puis diffuse la réponse. La route est enregistrée avec la même capacité. XERJ : `hacienda-api/src/handlers/claude_messages.rs:241` et `hacienda-api/src/routes.rs:491`.

Le gateway Codex existant suit une autre forme de protocole : le client de ChatGPT Desktop cible `/v1/responses` et `/v1/models`; la route Responses valide son en-tête secret local dédié tandis que `Authorization` doit rester disponible pour le transfert officiel. Ce partage d’origin sert aussi le viewer. XERJ : `hacienda-api/src/routes.rs:500` et `hacienda-api/src/routes.rs:518`.

Le viewer Codex utilise une session courte liée à un cookie pour ses API d’événements et de révélation. Les événements de projection sont typés `Mapping { turn_id, token, original }`, `Text { turn_id, text }` et `End { turn_id }`; le flux est associé au relay qui porte `turn_id` et `ViewerSessionStore`. XERJ : `hacienda-api/src/handlers/codex_viewer.rs:48` et `hacienda-api/src/handlers/codex_responses.rs:1013`.

Pour la parité Claude, réutiliser les principes de sécurité du viewer (session courte, origine locale, autorisation par tour, non-persistance du plaintext), pas nécessairement le transport d’événements tel quel. La carte Mapping ne doit pas devenir une sortie supplémentaire de `/v1/messages` ou une donnée visible de l’API modèle. Le Mod Claude est un consommateur de projection de présentation côté client ; il ne doit pas créer une deuxième voie de redaction ni modifier les événements/messages transmis à Claude. Le format exact de pont entre le Mod et Hacienda reste à concevoir et vérifier : les sources actuelles confirment le slot de rendu, elles ne spécifient pas un protocole Hacienda↔Mod.

## Critères d’acceptation pour la parité

1. **Routage pris en charge :** une configuration de gateway Desktop locale et administrée est documentée et testée sur la version Desktop ciblée ; les requêtes arrivent à Hacienda sur `/v1/messages` avec streaming et headers requis, sans chemin de contournement vers le provider.
2. **Redaction avant egress :** prompts et champs de réponse inspectables sont pseudonymisés avant tout appel provider ; les corps/blocs inconnus ou non inspectables échouent de façon fermée. La session Claude et son transcript ne contiennent que des pseudonymes.
3. **Contrat Mod :** le Mod s’enregistre sur `ui.render` avec `component: 'AssistantMessage'`, traite chaque bloc markdown et laisse les propriétés read-only intactes ; il est vérifié sur la surface `desktop` (Code tab), ainsi que sur les rerenders, scroll/viewport, streaming, erreurs et absences de tokens.
4. **Projection locale seule :** un token autorisé peut être rendu en gris (ou style visuel équivalent) à partir d’une capacité locale limitée au tenant/session/tour. La valeur claire n’apparaît jamais dans la requête provider, le transcript persistant, le contexte suivant, les outils, ni les logs.
5. **Contrôles reveal :** refus, expiration, révocation, token inconnu, session/tenant incorrects et backend indisponible conservent le pseudonyme ; révélation explicite et audit content-free sont requis ; le plaintext reste en mémoire éphémère.
6. **Aucune régression Codex :** `/v1/responses`, `/v1/models`, contrôle de session cookie-bound et événements de viewer existants continuent de fonctionner. Les sessions et cartes Claude ne sont jamais confondues avec les `turn_id`/sessions Codex.
7. **Limite de surface comprise :** le critère de parité porte sur Claude Code dans l’onglet Code de Claude Desktop. Toute revendication de parité avec le chat général Desktop exige une API de rendu officielle distincte et vérifiable.
8. **Tests de sécurité observables :** tests d’intégration prouvent l’absence de plaintext sur le réseau provider et dans les enregistrements persistants, le caractère fail-closed des contenus non supportés, ainsi que l’isolation tenant/session et la révocation des capacités.

## Sources

### Documentation Anthropic et Context7

- [Claude Code — Connect to an LLM gateway](https://code.claude.com/docs/en/llm-gateway-connect) (configuration distincte Desktop / CLI).
- [Claude Code — Gateway protocol](https://code.claude.com/docs/en/llm-gateway-protocol) (Messages API, routes et headers).
- [Claude Code — Mods overview](https://code.claude.com/docs/en/plugins/mods/overview) (Mod hooks).
- [Claude Code — Modify existing interface](https://code.claude.com/docs/en/plugins/mods/interface) (`ui.render`).
- [Claude Code — Mods reference](https://code.claude.com/docs/en/plugins/mods/reference) (render sites et composants).
- [Anthropic Claude Code types, `mods/types/claude-code.d.ts`](https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts) (`AssistantMessage`, props et surfaces incluant `desktop`).

Context7 a été interrogé le 2026-11-24 via la bibliothèque officielle `Claude Code` (`/websites/code_claude`, et `/anthropics/claude-code` pour les types de Mods). Les URLs ci-dessus pointent vers les sources Anthropic/première partie plutôt que vers des extraits de cache.

### Dépôt, repères XERJ

- `hacienda-api/src/handlers/claude_messages.rs:241` — handler et pipeline de redaction `/v1/messages`.
- `hacienda-api/src/routes.rs:491` — contrôle de capacité sur `/v1/messages`.
- `hacienda-api/src/routes.rs:500` — routes Codex `/v1/responses`, `/v1/models` et mécanisme gateway.
- `hacienda-api/src/routes.rs:518` — routes viewer `/viewer/events` et `/viewer/reveal`.
- `hacienda-api/src/handlers/codex_viewer.rs:48` — variantes d’événements de projection.
- `hacienda-api/src/handlers/codex_responses.rs:1013` — relay Responses, session viewer et `turn_id`.
- `crates/hacienda-backend/src/lib.rs:362` — adaptateur de révélation vault, vérification de capacité et tenant.
- `docs/research/2026-11-24-native-desktop-pii-boundary.md:39` — analyse antérieure de la frontière PII desktop, consultée en lecture seulement.

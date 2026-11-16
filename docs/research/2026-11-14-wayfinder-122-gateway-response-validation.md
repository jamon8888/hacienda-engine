# Wayfinder #122 — validation du gateway, des hooks Claude et de la réhydratation

**Date de recherche :** 2026-11-14<br>
**Périmètre :** validation documentaire et inspection du code local uniquement.<br>
**Contraintes respectées :** sources primaires officielles Anthropic/MCP et code local accessible ; aucune implémentation et aucun autre fichier modifié.

## Conclusion exécutive

Le plan Hacienda Secrecy Client est techniquement plausible, mais quatre corrections doivent être intégrées avant toute implémentation :

1. `UserPromptSubmit` peut inspecter et bloquer un prompt, mais ne peut pas le remplacer. Il ne peut donc pas être le moteur principal d’anonymisation.
2. `PreToolUse` peut réécrire l'entrée complète d'un outil et `PostToolUse` la sortie visible par Claude, mais ces hooks ne couvrent pas tous les chemins d'entrée/sortie. `MessageDisplay` ne modifie que l'affichage ; il ne modifie ni le transcript ni le contexte de Claude.
3. Un gateway `ANTHROPIC_BASE_URL` est officiellement supporté pour Claude Code. Il reçoit des réponses Anthropic Messages en SSE, mais doit relayer les événements progressivement, dans l'ordre, avec les `ping` et les événements finaux. Le gateway local inspecté bufferise actuellement la réponse complète : cette implémentation n'est pas compatible avec une expérience de streaming Claude Code sans adaptation.
4. MCP stdio expose bien des outils au modèle, mais une capacité `rehydrate` qui retourne de la PII brute serait incompatible avec l'invariant Hacienda « agent-facing token-only ». La réhydratation doit rester dans un canal humain séparé, non MCP.

La conclusion de sécurité est donc : **gateway pour la politique d'egress et la validation token-only ; Tauri pour la présentation colorée et la réhydratation humaine ; MCP agent pour des références et résultats tokenisés uniquement.**

## 1. Claude Code hooks

### Validation

| Hook | Ce que la documentation officielle confirme | Limite déterminante pour Hacienda | Verdict |
|---|---|---|---|
| `UserPromptSubmit` | Reçoit le texte soumis dans `prompt`; peut ajouter `additionalContext` ou bloquer le prompt avant Claude. | Ne peut pas remplacer le prompt. Même un prompt bloqué peut rester dans l'historique/transcript local ; `suppressOriginalPrompt` ne supprime que le texte du message de blocage, pas toutes les traces locales. | **Validé avec correction : garde fail-closed, pas anonymiseur.** |
| `PreToolUse` | S'exécute après génération des paramètres et avant l'appel outil ; peut autoriser, refuser, demander confirmation ou renvoyer `updatedInput`, qui remplace l'objet d'entrée complet. | Ne s'exécute que lorsqu'un outil est appelé. Les fichiers insérés via `@` sont ajoutés au contexte sans appel `Read` par défaut ; les processus/subprocessus et chemins d'accès indirects nécessitent des contrôles séparés. | **Validé pour les entrées outil, couverture partielle.** |
| `PostToolUse` | S'exécute après un outil réussi ; reçoit `tool_input` et `tool_response`; `updatedToolOutput` peut remplacer la sortie avant qu'elle soit envoyée à Claude. | L'outil a déjà produit ses effets : la réécriture ne défait pas une écriture, une commande, un upload, une requête réseau ou la télémétrie déjà enregistrée. | **Validé pour la sortie agent, pas pour annuler un effet.** |
| `MessageDisplay` | Reçoit le texte assistant par lots et peut renvoyer `displayContent` à afficher à la place du delta. | Affichage seulement : Claude et le transcript gardent le texte original ; les résultats d'outils et le texte saisi ne passent pas par ce hook ; il n'y a pas de champ officiel de couleur/HTML/`span`. En cas d'échec, le texte original est affiché. | **Validé pour une transformation visuelle textuelle, pas comme frontière de confidentialité.** |

Sources officielles : [Hooks reference — `UserPromptSubmit`](https://code.claude.com/docs/en/hooks#userpromptsubmit), [decision control](https://code.claude.com/docs/en/hooks#decision-control), [PreToolUse](https://code.claude.com/docs/en/hooks#pretooluse), [PostToolUse](https://code.claude.com/docs/en/hooks#posttooluse), [MessageDisplay](https://code.claude.com/docs/en/hooks#messagedisplay).

La documentation précise notamment que `UserPromptSubmit` ne remplace pas le prompt et que `PreToolUse`/`PostToolUse` sont les points de transformation recommandés pour les entrées/sorties outils ([table de contrôle](https://code.claude.com/docs/en/hooks#decision-control)). Elle précise également que `MessageDisplay` est display-only et que le texte original reste dans le transcript et dans ce que Claude voit ([MessageDisplay](https://code.claude.com/docs/en/hooks#messagedisplay)).

### Conséquence pour la coloration des tokens

Le hook `MessageDisplay` peut transformer un texte tokenisé tel que `[PERSON_1]` ou `[EMAIL_2]` en une autre représentation textuelle. La source officielle ne promet cependant pas un canal de style, de couleur, d'HTML ou de métadonnées sémantiques. Le contrat fiable est donc :

- réponse et transcript Claude : texte tokenisé brut, sans PII ;
- Tauri : rendu riche avec des spans colorés et des labels accessibles ;
- Claude Code terminal : éventuellement une transformation textuelle, mais aucune promesse commerciale de couleur sans test de la version ciblée ;
- jamais de réinjection de la valeur PII dans `displayContent`.

Le rapport local Hacienda arrive à la même frontière : `MessageDisplay` convient à la coloration/redaction d'affichage, mais le transcript et Claude conservent la réponse originale ([recherche locale Claude Code](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/docs/claude-code-official-research.md:52>), lignes 52–60).

## 2. Claude Desktop Code tab, configuration partagée et Chat classique

### Code tab

Anthropic documente que Claude Desktop exécute Claude Code dans le **Code tab**. Les sessions locales du Code tab et le CLI partagent les fichiers de configuration Claude Code, notamment `CLAUDE.md`, les serveurs MCP, les hooks, les skills, `~/.claude.json` et `~/.claude/settings.json`. Les versions embarquée dans Desktop et installée dans le terminal peuvent toutefois différer.

Source officielle : [Claude Code on desktop — shared configuration](https://code.claude.com/docs/en/desktop#shared-configuration), notamment la section qui indique que hooks et MCP sont partagés (lignes 1840–1851 de la page consultée), et [version du Claude Code dans le Code tab](https://code.claude.com/docs/en/desktop#claude-code-version-in-the-code-tab).

**Verdict : validé pour le Code tab local**, sous réserve de tester la version embarquée et les champs de hook effectivement supportés par cette version.

### Chat classique

La documentation Anthropic distingue explicitement le Code tab de l'interface Chat. Les hooks décrits dans la documentation sont des hooks Claude Code ; les serveurs MCP et extensions Desktop exposent des outils, ressources ou prompts, mais ne constituent pas un hook universel documenté avant chaque prompt Chat, chaque upload ou chaque rendu de message.

Le code local inspecté renforce cette conclusion, sans la transformer en garantie éternelle pour toutes les versions :

- le renderer Chat principal est chargé depuis `https://claude.ai` ;
- l'analyse du bundle n'a trouvé aucun callback public observé du type `beforeSendPrompt`, `transformAttachment` ou `renderMessage` dans les objets inspectés ;
- l'API Electron `webRequest` observée peut observer/annuler/rediriger et modifier des headers, mais ne fournit pas un remplacement générique du body de requête.

Voir [carte d'intégration REA locale](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/docs/claude-desktop-rea-integration-map.md:25>), lignes 25–35 et 45–53. Le document local précise que cette absence est limitée à la couverture et à la version inspectées ; elle ne constitue pas une API publique stable.

**Verdict : validé comme limite de conception actuelle.** Pour une promesse de confidentialité totale, le produit doit posséder son propre composer/rendu/transport Tauri ou utiliser le Code tab via les surfaces officiellement supportées. Une extension MCP seule ne suffit pas.

## 3. Anthropic API, SSE et gateway base URL

### Streaming SSE

La documentation officielle Anthropic confirme que `stream: true` utilise des Server-Sent Events. Le flux comporte notamment :

1. `message_start` ;
2. des blocs `content_block_start`, `content_block_delta` et `content_block_stop` ;
3. un ou plusieurs `message_delta` ;
4. `message_stop`.

Il peut aussi contenir des événements `ping` et `error`. Les types d'événements peuvent évoluer ; les consommateurs doivent gérer les types inconnus proprement. Les `usage` de `message_delta` sont cumulatifs.

Source officielle : [Anthropic — Streaming messages](https://platform.claude.com/docs/en/build-with-claude/streaming), sections « Message streaming » et « Event types » (lignes 72–75, 95, 132–159 de la page consultée).

**Précision importante sur “récupérer les tokens” :** le flux expose des deltas de contenu et des compteurs d'usage, pas une garantie d'accès aux identifiants exacts du tokenizer interne ni à une frontière stable entre tokens de modèle. Hacienda peut capturer les événements/deltas SSE et les marqueurs Hacienda, mais ne doit pas parler de « tokens LLM exacts » sans contrat séparé.

### Base URL gateway

Anthropic documente officiellement le raccordement de Claude Code à un gateway :

- `ANTHROPIC_BASE_URL` pointe vers le gateway ;
- le gateway fournit au minimum `/v1/messages` et peut fournir `/v1/messages/count_tokens` ;
- le gateway doit accepter le format Anthropic Messages ;
- les headers `anthropic-version` et `anthropic-beta` doivent être relayés de façon compatible ;
- Claude Code peut envoyer des headers de session/agent et des champs qui évoluent avec les versions.

Sources officielles : [Connect Claude Code to an LLM gateway](https://code.claude.com/docs/en/llm-gateway-connect), lignes 115–120, 148–169, 178–203, et [Claude Code gateway compatibility guide](https://code.claude.com/docs/en/llm-gateway-protocol), lignes 133–151, 188–224.

La documentation confirme également que Claude Code traite un `ANTHROPIC_BASE_URL` personnalisé comme un endpoint Anthropic-format et envoie les capacités correspondant à l'API Anthropic. Les listes de headers et champs doivent être considérées comme ouvertes ; une allowlist figée casse les nouvelles versions. Toute transformation du body doit conserver la cohérence des paires header/champ beta, sinon des erreurs `400` ou une désactivation silencieuse de capacité peuvent apparaître ([compatibility guide](https://code.claude.com/docs/en/llm-gateway-protocol#feature-pass-through), lignes 248–260).

### Limites du relay SSE

La documentation officielle de compatibilité est explicite :

- un gateway qui bufferise toute la réponse avant de l'envoyer fait patienter Claude Code ;
- la séquence complète doit parvenir dans l'ordre jusqu'à `message_delta` final et `message_stop`;
- retirer les `ping` peut déclencher le timeout d'inactivité ;
- le `Content-Type` doit rester `text/event-stream` pour le format Anthropic Messages ;
- les réponses Bedrock ne doivent pas être transformées en SSE si le client attend le format EventStream Bedrock.

Source : [gateway compatibility guide — Streaming](https://code.claude.com/docs/en/llm-gateway-protocol#streaming), lignes 152–165 et [response headers](https://code.claude.com/docs/en/llm-gateway-protocol#response-headers), lignes 225–233.

### Confrontation avec le code Hacienda local

Le code local démontre une base utile, mais aussi un écart bloquant pour le streaming Claude Code :

- le gateway cible `/v1/messages` et `/v1/chat/completions` ([`src/gateway/mod.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/gateway/mod.rs:118>), lignes 118–127) ;
- il inspecte le body de requête avant egress et la réponse après retour ([`server.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/gateway/server.rs:223>), lignes 223–230 et [`server.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/gateway/server.rs:273>), lignes 273–303) ;
- il bufferise la réponse complète avant de libérer le moindre octet ([`server.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/gateway/server.rs:273>), lignes 273–289) ;
- il réémet les événements SSE après inspection, en préservant la structure générale et les marqueurs, mais pas en streaming progressif ([`sse.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/gateway/sse.rs:1>), lignes 1–8 et 27–60 ; tests [`gateway_proxy.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/tests/gateway_proxy.rs:325>), lignes 325–366).

**Verdict :** le modèle de gateway/base URL est validé pour Claude Code, mais le comportement local « buffer complet puis replay » doit être considéré comme une preuve de parsing/redaction, pas comme une implémentation de production compatible avec le streaming interactif Claude Code. La future spec doit choisir explicitement entre :

- inspection incrémentale avec état de tokenisation et fail-closed avant émission de chaque segment ; ou
- une latence/latence de fin de réponse assumée et une surface client qui ne dépend pas du streaming Claude Code.

Le gateway officiel Claude Code montre aussi des chemins qui ne suivent pas nécessairement `ANTHROPIC_BASE_URL` : certaines vérifications, notamment la vérification WebFetch et certaines vérifications de fast mode, peuvent appeler directement `api.anthropic.com`; le trafic non essentiel doit être désactivé et testé séparément pour une politique d'egress stricte ([gateway connect](https://code.claude.com/docs/en/llm-gateway-connect#turn-off-traffic-outside-the-gateway-path), lignes 369–384). Une promesse « tout le trafic Claude » exige donc une matrice de chemins et des règles réseau en complément du gateway.

## 4. MCP stdio et outil de réhydratation

### Transport stdio

La spécification MCP officielle confirme que :

- le client lance le serveur MCP comme subprocess ;
- le serveur lit les messages JSON-RPC sur `stdin` et écrit les réponses sur `stdout` ;
- les messages sont JSON-RPC UTF-8 délimités par lignes ;
- `stdout` ne doit contenir que des messages MCP valides ;
- les logs peuvent aller sur `stderr`.

Source primaire : [MCP specification — transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports), lignes 24–40.

Le code local expose un chemin stdio via `run_stdio`, avec le transport construit sur `io::stdin()`/`io::stdout()` ([`src/mcp_server.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/mcp_server.rs:254>), lignes 254–258). L'extension Claude Desktop documente aussi le lancement local `hacienda-backend mcp serve --transport stdio` ([`mcp-extension/README.md`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/mcp-extension/README.md:15>), lignes 15–30 et 84–92).

**Verdict : validé.** Stdio convient à un serveur local lancé par Claude Desktop/Code ; il est implicitement lié au processus client et ne doit pas être traité comme un service multi-tenant distant.

### Exposition d'outils et réhydratation

La spécification MCP officielle autorise un serveur à exposer des tools appelables par le modèle via `tools/list` et `tools/call`. Elle recommande toutefois une présence humaine capable de refuser l'appel, une UI indiquant les outils exposés et des confirmations pour les opérations sensibles. Elle impose aux serveurs de valider les entrées, contrôler l'accès, limiter le débit, assainir les sorties et journaliser l'usage ([MCP tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools), lignes 25–33 et 397–409).

Cela signifie :

- **techniquement**, un outil `rehydrate` est exposable par MCP ;
- **architecturalement**, un outil retournant la PII brute à l'agent violerait le contrat Hacienda token-only ;
- **correctement**, MCP peut seulement créer une demande de divulgation, retourner un handle opaque et fournir un statut ; l'ouverture du contenu réhydraté doit se faire dans un viewer humain authentifié et séparé.

Le code local cible déjà cette séparation dans le gate : `CLOSED_TOOLS` contient `search`, `sync`, `status`, `attach_local_doc`, `request_human_disclosure` et `disclosure_status`, sans `rehydrate` ([`src/mcp_gate.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/mcp_gate.rs:16>), lignes 16–28). Les tests de contrat vérifient également l'absence de `rehydrate` ([`tests/mcp_contract.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/tests/mcp_contract.rs:50>), lignes 50–60 et 108–135).

### Incohérence à résoudre avant implémentation

Le dépôt local contient deux états documentés qui ne peuvent pas être affirmés simultanément comme « état courant » :

- le README de l'extension décrit un manifeste fermé sans réhydratation brute ([`mcp-extension/README.md`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/mcp-extension/README.md:1>), lignes 1–5 et 66–82) ;
- `docs/spec.md` décrit encore le serveur courant comme non conforme, avec un dispatch `rehydrate` et une capacité `Capability::Rehydrate` ([`docs/spec.md`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/docs/spec.md:142>), lignes 142–160 et 259–272) ;
- les tests et le gate fermé attendent désormais l'absence de `rehydrate`.

**Verdict : non validé comme fait unique tant que la branche cible n'est pas alignée.** Pour le plan Secrecy Client, la décision normative doit rester : **aucun outil MCP ne retourne une PII réhydratée**. La réhydratation CLI/viewer doit être explicitement hors de la surface agent, avec export humain contrôlé et audit.

## Décisions de spécification recommandées

Ces décisions sont des conclusions de recherche, pas des modifications effectuées dans ce tour :

1. **Point d'anonymisation principal :** gateway/Tauri avant l'appel provider ; `UserPromptSubmit` ne sert qu'à détecter et bloquer les bypass.
2. **Surface Code tab :** hooks Claude Code + MCP fermé, testés sur la version embarquée exacte ; ne pas étendre cette garantie au Chat classique.
3. **Réponse Claude :** le gateway valide le flux Anthropic SSE et préserve les tokens Hacienda ; il ne réinjecte jamais de PII brute.
4. **Couleur :** métadonnées/classes de présentation côté Tauri ; `MessageDisplay` seulement comme transformation textuelle optionnelle, sans dépendre d'un protocole de couleur non documenté.
5. **Streaming :** ne pas conserver comme contrat de production le buffer complet actuel ; le gateway doit relayer progressivement ou déclarer explicitement une surface non-streaming.
6. **MCP :** `request_human_disclosure`/`disclosure_status` peuvent retourner des handles et états non sensibles ; `rehydrate`, `disclosure_value`, `open_viewer` et équivalents restent absents de la surface agent.
7. **Egress total :** ajouter des tests/règles pour les chemins directs documentés par Claude Code et pour les outils/extensions qui sortent du chemin `ANTHROPIC_BASE_URL`.

## Sources primaires

### Documentation officielle en ligne

- [Anthropic — Streaming messages](https://platform.claude.com/docs/en/build-with-claude/streaming)
- [Claude Code — Hooks reference](https://code.claude.com/docs/en/hooks)
- [Claude Code — Desktop application](https://code.claude.com/docs/en/desktop)
- [Claude Code — Connect Claude Code to an LLM gateway](https://code.claude.com/docs/en/llm-gateway-connect)
- [Claude Code — Gateway compatibility guide](https://code.claude.com/docs/en/llm-gateway-protocol)
- [Model Context Protocol — Transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
- [Model Context Protocol — Tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
- [Anthropic Help Center — Local MCP servers on Claude Desktop](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop)

### Code et documentation locale consultés

- [`docs/claude-code-official-research.md`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/docs/claude-code-official-research.md:7>) — synthèse locale des limites de hooks, lignes 7–35 et 52–60.
- [`docs/claude-desktop-rea-integration-map.md`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/docs/claude-desktop-rea-integration-map.md:25>) — analyse statique versionnée de Claude Desktop, lignes 25–53.
- [`docs/adr/0006-provider-egress-gateway.md`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/docs/adr/0006-provider-egress-gateway.md:8>) — frontière d'egress proposée, lignes 8–26.
- [`src/gateway/mod.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/gateway/mod.rs:1>) — routes et invariants locaux, lignes 1–19 et 118–127.
- [`src/gateway/server.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/gateway/server.rs:179>) — redaction ingress, buffer response et inspection SSE, lignes 179–346.
- [`src/gateway/sse.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/gateway/sse.rs:1>) — parsing/réémission SSE, lignes 1–141.
- [`tests/gateway_proxy.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/tests/gateway_proxy.rs:210>) — tests d'egress, de réponse JSON et de replay SSE, lignes 210–366.
- [`src/mcp_gate.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/mcp_gate.rs:16>) — manifeste MCP fermé, lignes 16–28 et 313–317.
- [`src/mcp_server.rs`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/src/mcp_server.rs:254>) — transport stdio local, lignes 254–258.
- [`mcp-extension/README.md`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/mcp-extension/README.md:1>) — extension fermée et transport stdio, lignes 1–5, 66–92.
- [`docs/spec.md`](</Users/macbook/orca/workspaces/hacienda-backend/issue-15-secure-viewer-opencode/docs/spec.md:93>) — contrat cible token-only et registre des écarts, lignes 93–113, 142–176 et 259–287.

## Addendum d’implémentation — 2026-11-15

Le relay Anthropic Messages décrit comme manquant dans l’inspection du 2026-11-14 a depuis
été implémenté : `5d9fd01` raccorde le provider à `hacienda serve`, et `2e448ad` ajoute le
relay SSE borné et incrémental, la redaction des blocs texte/outils et la validation
progressive. Les tests unitaires correspondants sont dans
`hacienda-api/src/handlers/claude_messages.rs`, mais leur exécution Cargo reste à confirmer.
Le support Codex Responses reste au stade de recherche (`2024222`).

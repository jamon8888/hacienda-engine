# Contrat validé du gateway Claude et de la réponse tokenisée — Wayfinder #122

**Date :** 2026-11-14<br>
**Statut :** décision d’architecture validée avant implémentation<br>
**Périmètre :** Claude Code CLI, Claude Code dans Claude Desktop, client Tauri Hacienda,
serveur MCP Hacienda et API RAG existante.

## Verdict

La solution est techniquement viable, sous réserve de distinguer trois choses :

1. les **tokens Hacienda** (`[PERSON:... ]`, `[EMAIL:...]`) qui sont des chaînes
   pseudonymisées stables ;
2. les **deltas de texte Claude** reçus dans le flux SSE ;
3. les **tokens internes du modèle**, que l’API ne fournit pas comme événements de texte.

Le gateway peut recevoir et retransmettre les réponses Claude en streaming, conserver et
valider les tokens Hacienda, et produire une réponse strictement tokenisée. Il ne récupère
pas les identifiants BPE internes de Claude : l’API expose des fragments de texte et des
compteurs d’usage, pas un token ID par fragment.

## Validation officielle de l’intégration gateway

Claude Code documente un gateway Anthropic-compatible via `ANTHROPIC_BASE_URL`. Le gateway
doit notamment fournir `POST /v1/messages`; `/v1/messages/count_tokens` est optionnel.
Claude Desktop Code possède une configuration gateway dédiée dans son application ; il ne
faut pas supposer que les variables d’environnement de la CLI suffisent à configurer toute
l’application Desktop.

Sources :

- [Connexion Claude Code à un gateway](https://code.claude.com/docs/en/llm-gateway-connect)
- [Compatibilité protocolaire des gateways Claude Code](https://code.claude.com/docs/en/llm-gateway-protocol)
- [Configuration partagée Desktop/CLI](https://code.claude.com/docs/en/desktop#shared-configuration)
- [Streaming Messages Anthropic](https://platform.claude.com/docs/en/build-with-claude/streaming)

## Contrat de requête

Le gateway Hacienda doit traiter le body Anthropic comme une structure ouverte :

- préserver les headers `anthropic-version` et `anthropic-beta` ;
- préserver les couples header/champ body liés aux fonctionnalités Claude ;
- préserver le bloc d’attribution système ou demander au client de le désactiver avec
  `CLAUDE_CODE_ATTRIBUTION_HEADER=0` si la transformation rend sa position invalide ;
- préserver les identifiants de session utiles au routage et à l’audit ;
- transformer uniquement les contenus autorisés par le contrat de redaction ;
- refuser toute structure inconnue contenant potentiellement du contenu avant de l’avoir
  classifiée et redacted.

La redaction ne peut donc pas être une simple recherche/remplacement sur une chaîne JSON.
Elle doit être consciente des blocs de texte, appels d’outils, résultats d’outils, messages
système, fichiers, images et autres pièces jointes. Toute pièce jointe non inspectable doit
être refusée en mode strict.

Le gateway ne doit jamais transmettre au fournisseur :

- la valeur en clair derrière un token ;
- le texte brut d’un document ;
- un `system_prompt` non inspecté ;
- un résultat d’outil ou une pièce jointe non redacted ;
- une valeur confidentielle ajoutée dans les métadonnées ou headers de routage.

## Contrat de réponse streaming

Pour le format Anthropic Messages, le gateway doit relayer le flux sans buffering complet :

```text
message_start
  content_block_start
    content_block_delta*        ← texte / input JSON partiel
  content_block_stop
  message_delta*
message_stop
```

Il doit également préserver les `ping`, les erreurs SSE et les headers nécessaires au
client. Les événements inconnus doivent être relayés sans perte lorsque leur contenu est
déjà token-only ; sinon le gateway doit interrompre le flux en échec fermé.

Le gateway peut :

- valider que chaque delta respecte le schéma attendu ;
- repérer des tokens Hacienda connus dans les fragments de texte ;
- refuser une réponse qui contient une valeur PII en clair connue du coffre ;
- produire une trace d’audit sans contenu brut ;
- fournir au client Tauri une annotation de présentation séparée.

Il ne doit pas modifier la réponse provider avec du HTML, des spans ou des séquences ANSI.
Ces données pourraient être conservées dans le transcript, interprétées comme du contenu,
ou casser le protocole de streaming.

## Présentation et couleur

### Client Tauri

La couleur est garantie dans le renderer Tauri, sur la base de classes sémantiques :

```json
{
  "text": "[PERSON:k1:ABC...]",
  "token_class": "person",
  "token_id": "opaque-local-id",
  "display_label": "Identité pseudonymisée"
}
```

Le champ `token_id` de présentation ne doit pas être une clé permettant de retrouver la
valeur brute sans passer par l’autorisation du viewer. Les couleurs ne sont pas un contrôle
de sécurité ; l’interface doit aussi afficher un libellé et rester utilisable sans couleur.

### Claude Code

Le hook officiel `MessageDisplay` peut remplacer le texte affiché pendant le streaming,
mais il est explicitement display-only : Claude, le transcript et le mode verbose gardent
le texte original. Il reçoit le texte assistant, pas les résultats d’outils ni le texte
saisi par l’utilisateur. La documentation ne constitue pas un contrat de rendu HTML,
CSS, spans ou couleur ANSI.

Conséquence :

- `MessageDisplay` peut ajouter une présentation textuelle sûre, par exemple conserver les
  marqueurs `[PERSON:...]` ou ajouter des symboles accessibles ;
- la coloration riche et fiable appartient au viewer Tauri ;
- `MessageDisplay` ne doit jamais être le seul mécanisme de redaction ou de prévention de
  fuite.

Source : [Hooks Claude Code — MessageDisplay](https://code.claude.com/docs/en/hooks#messagedisplay).

## Réhydratation

La réhydratation est une opération du viewer humain, jamais une transformation du flux
Claude :

```text
token dans réponse Claude
  → clic/commande dans Tauri
  → demande d’autorisation dossier/champ
  → révélation Hacienda auditée
  → affichage temporaire dans le viewer isolé
```

Le texte révélé ne doit pas être renvoyé dans :

- le prompt ou l’historique Claude ;
- le résultat d’un outil MCP ;
- le contexte RAG ;
- les logs, traces, exports ou métadonnées d’annotation ;
- les données de présentation persistées.

Le serveur MCP actuel expose encore `pii_reveal` via le transport stdio et appelle la facade
avec `Caller::Trusted`. Ce chemin ne peut pas être utilisé comme viewer humain multi-utilisateur.
Avant implémentation du client, le manifeste agent doit retirer cette capacité ou la rendre
inaccessible au contexte Claude/Codex ; l’endpoint humain devra utiliser une session locale
authentifiée, des capacités limitées et une expiration.

## Limites prouvées

- Le gateway Anthropic est officiellement supporté pour Claude Code ; le format, les headers
  et la séquence SSE doivent être conservés.
- Le Code tab de Claude Desktop partage les hooks et configurations Claude Code, mais son
  routage gateway se configure via la configuration gateway de Desktop.
- Le Chat classique Claude Desktop et ses pièces jointes ne sont pas couverts par le hook
  `MessageDisplay` de Claude Code ; une garantie de secret doit donc passer par Tauri ou une
  surface Claude Code contrôlée.
- `UserPromptSubmit` peut bloquer mais ne remplace pas le prompt original ; il reste un
  garde-fou secondaire, pas le proxy de redaction principal.
- `PreToolUse` peut contrôler les appels d’outils avant exécution ; `PostToolUse` peut
  remplacer ce que Claude voit après exécution, mais ne peut pas annuler les effets déjà
  produits ni l’original déjà télémétré.
- Le flux SSE RAG actuel de Hacienda (`token`, `citation`, `usage`, `done`, `error`) est un
  contrat interne de synthèse RAG ; il ne doit pas être confondu avec le protocole
  Anthropic Messages du gateway.
- Le gateway local inspecté dans le workspace de recherche bufferise actuellement la réponse
  complète avant replay SSE. Ce comportement est acceptable comme prototype de parsing et de
  redaction, mais il n’est pas le contrat de production Claude Code : la prochaine
  implémentation doit choisir un relay incrémental sûr, ou déclarer explicitement une surface
  non-streaming distincte.
- Claude Code documente aussi des sorties auxiliaires qui peuvent contourner un simple
  `ANTHROPIC_BASE_URL` (par exemple certaines vérifications et WebFetch). La garantie
  « zéro-egress PII » exige donc une matrice de chemins et des règles réseau en complément du
  gateway.

## Décision de mise en œuvre différée

Avant tout code, les prochains tickets d’implémentation devront couvrir séparément :

1. adaptateur Anthropic Messages strict et tests de compatibilité SSE ;
2. redaction structurelle des requêtes Claude, y compris outils et pièces jointes ;
3. validation structurelle des réponses et détection de fuite ;
4. canal d’annotations Tauri sans modification du payload LLM ;
5. fermeture de `pii_reveal` côté MCP agent ;
6. viewer de réhydratation avec autorisation, expiration et audit ;
7. tests d’échec fermé, inconnus, retry, ping, compaction et évolution de schéma.

Aucune de ces tâches n’est considérée implémentée par cette spécification.

## Rapport de recherche associé

Les preuves détaillées, les lignes de code locales, les sources MCP et la comparaison avec le
gateway existant sont consignées dans
[`docs/research/2026-11-14-wayfinder-122-gateway-response-validation.md`](../../research/2026-11-14-wayfinder-122-gateway-response-validation.md).

## Addendum — état livré au 2026-11-15

Les points d’implémentation différée ci-dessus décrivent l’état précédant le code et sont
à relire avec cet addendum. Le gateway Claude Messages est désormais présent dans
`hacienda-api/src/handlers/claude_messages.rs` et activable par `hacienda serve` avec
`HACIENDA_ANTHROPIC_MESSAGES_API_KEY` et l’endpoint facultatif
`HACIENDA_ANTHROPIC_MESSAGES_ENDPOINT` (commits `5d9fd01`, `2e448ad`).

Le relais SSE est incrémental, borne la mémoire par bloc, valide l’ordre des événements,
traite les deltas de texte et de `tool_use`, rédige les erreurs et n’accepte les événements
inconnus que lorsqu’ils respectent le contrat token-only. Les en-têtes exposés au client
sont allowlistés. Des tests ciblés ont été ajoutés, mais aucun test Cargo ni Clippy n’a
encore fourni de preuve d’exécution dans cet environnement.

Le gateway Codex CLI (`/v1/responses`, protocole Responses SSE) reste une tâche
d’implémentation distincte ; le rapport associé est une recherche, pas une livraison.
La validation dans l’application Claude Desktop et l’audit des chemins d’egress auxiliaires
restent également ouverts.

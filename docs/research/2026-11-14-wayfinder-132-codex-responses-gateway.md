# Recherche AFK — Codex CLI comme client du gateway Responses Hacienda

**Date :** 2026-11-14  
**Question :** comment faire de Codex CLI un client d’un gateway Hacienda qui
redacte les PII, conserve le contexte et relaie le streaming, dans les profils
`LocalFull` et `TeamOnPremise` ?  
**Méthode :** sources primaires uniquement : documentation OpenAI, source du dépôt
officiel `openai/codex`, et inspection en lecture seule du checkout Hacienda. Aucun
code produit, document existant ou installation n’a été modifié par cette recherche.

## Verdict

Le contrat correct pour Codex CLI est **Responses API**, pas Chat Completions. Un
gateway Hacienda compatible Codex doit donc exposer un endpoint `POST /v1/responses`
(ou le même chemin relatif sous une base URL configurée), accepter un body Responses
structuré, renvoyer un flux SSE Responses, et transmettre une authentification Bearer
fournie par la configuration du provider. Le dépôt Codex officiel ne supporte plus
`wire_api = "chat"` ; `responses` est la seule valeur acceptée et la valeur par
défaut dans le code courant. [Source officielle Codex — `WireApi`](https://github.com/openai/codex/blob/main/codex-rs/model-provider-info/src/lib.rs#L110-L141)

Le transport Tauri actuel **ne suffit pas au Codex CLI** : il s’appelle encore
`OpenAiProviderTransport`, mais construit `messages[]` et parse des événements
Chat Completions (`choices[].delta.content`, `[DONE]`). Codex attend `input[]` et
les items Responses, puis des événements `response.*`. [Code local — transport Codex](../../../crates/hacienda-client-core/src/provider.rs#L802-L865),
[source officielle Codex — request HTTP Responses](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/endpoint/responses.rs#L80-L169)

La décision proposée est de traiter le gateway comme un **proxy Responses token-only** :
le gateway inspecte et redacted chaque champ sensible du body JSON, préserve la forme
et l’ordre des items, relaie les événements SSE compatibles sans les convertir en
Chat Completions, et maintient un contexte Hacienda opaque séparé du contenu brut.
La redaction doit rester structurelle et fail-closed ; une PII détectée dans un champ
non classifié, une pièce jointe non inspectable ou un événement inconnu contenant du
contenu doit interrompre la requête/le flux.

Les invariants produits et les deux profils viennent des spécifications locales
`#122`, `#124` et `#129` : gateway redacting, projection token-only, séparation du
viewer original, et LanceDB hors accès direct du client. [`#122`](../superpowers/specs/2026-11-14-wayfinder-122-gateway-response-contract.md),
[`#124`](../superpowers/specs/2026-11-14-wayfinder-124-integration-boundary-research.md),
[`#129`](../superpowers/specs/2026-11-14-wayfinder-129-editor-viewer-lancedb-contract.md)

## Contrat Codex confirmé

### Configuration provider : `wire_api`, `base_url`, route

La configuration Codex d’un provider contient `name`, `base_url`, `env_key`,
`http_headers`, `query_params`, les paramètres de retry/timeout et
`wire_api`. La référence officielle indique que `wire_api = "responses"` est la
seule valeur supportée et est la valeur par défaut ; elle documente aussi
`env_key` comme le nom de la variable d’environnement qui fournit la clé, ainsi
que les headers statiques ou issus de variables d’environnement. [Référence officielle Codex — configuration provider](https://developers.openai.com/codex/config-reference#configtoml)

Le `base_url` est une base, non l’URL finale de la requête. Le code officiel du
client concatène le chemin `/responses` à cette base et l’endpoint officiel
`ResponsesClient` fait `POST /responses`. [Source officielle Codex — construction d’URL](https://github.com/openai/codex/blob/main/codex-rs/codex-client/src/provider.rs#L40-L82),
[source officielle Codex — `POST /responses`](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/endpoint/responses.rs#L111-L162)

Le proxy Responses fourni dans le dépôt officiel est un exemple directement
applicable :

```toml
[model_providers.hacienda]
name = "Hacienda Responses Gateway"
base_url = "http://127.0.0.1:8000/v1" # LocalFull ; loopback seulement
wire_api = "responses"
env_key = "HACIENDA_GATEWAY_TOKEN"

[profiles.hacienda]
model_provider = "hacienda"
```

Le proxy officiel configure exactement une base terminant par `/v1` et n’accepte
que `POST /v1/responses`. C’est une preuve de forme de configuration, pas une
preuve qu’un chemin Hacienda particulier existe déjà. [Source officielle Codex — Responses API proxy](https://github.com/openai/codex/blob/main/codex-rs/responses-api-proxy/README.md#tl;dr)

**Décision proposée :** représenter l’URL configurée dans le profil Hacienda comme
la base Responses complète, par exemple `http://127.0.0.1:8000/v1` ou
`https://hacienda.example.test/gateway/v1`. Le gateway expose alors
`POST {base_url}/responses`, sans réécriture implicite de `/v1` dans le client.
Le code local stocke actuellement `http://127.0.0.1:8000` et
`https://hacienda.example.test/gateway` comme endpoints génériques ; l’ajout de
`/v1` et le routage Responses restent une décision d’implémentation future, non un
changement réalisé ici. [Code local — profils et sélection du gateway](../../../apps/hacienda-secrecy-client/src-tauri/src/main.rs#L1383-L1410),
[source officielle Codex — route relative `/responses`](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/endpoint/responses.rs#L145-L162)

### Authentification

Pour un provider custom, Codex peut lire le secret depuis `env_key`; le proxy
Responses officiel envoie alors `Authorization: Bearer <credential>` à l’amont.
La documentation de création d’une Response montre le même header Bearer côté API
OpenAI. [Référence officielle Codex — `env_key`](https://developers.openai.com/codex/config-reference#configtoml),
[source officielle Codex — auth du proxy](https://github.com/openai/codex/blob/main/codex-rs/responses-api-proxy/README.md#behavior),
[référence officielle Responses — authentification HTTP](https://developers.openai.com/api/reference/resources/responses/methods/create#request)

**LocalFull proposé :** `env_key = "HACIENDA_GATEWAY_TOKEN"` avec un token de
gateway local, même si la connexion est loopback ; l’URL HTTP ne serait autorisée
que pour `localhost`/`127.0.0.1`/`::1`. Le code Tauri exige déjà une credential
pour construire le transport et autorise HTTP seulement en loopback. [Code local — credential et validation de transport](../../../apps/hacienda-secrecy-client/src-tauri/src/main.rs#L1413-L1434),
[code local — validation d’URL](../../../crates/hacienda-client-core/src/provider.rs#L628-L640)

**TeamOnPremise proposé :** `env_key` pointe vers le secret de session/tenant
fourni par le contrôle d’identité ; l’URL est HTTPS et le gateway résout le tenant,
applique les ACL et porte l’audit. Le code local possède déjà une référence
`SecretReference` pour le gateway authentifié et valide HTTPS, mais son transport
actuel transforme cette credential en simple Bearer sans protocole de session
spécifique. [Code local — gateway authentifié](../../../apps/hacienda-secrecy-client/src-tauri/src/main.rs#L1399-L1408),
[code local — profil TeamOnPremise](../../../crates/hacienda-client-core/src/lib.rs#L341-L405),
[source officielle Codex — headers provider](https://developers.openai.com/codex/config-reference#configtoml)

**À décider :** le mode d’authentification TeamOnPremise (Bearer opaque, OAuth
session export, ou command-backed `auth`) et les headers de corrélation autorisés.
Les headers Hacienda (`tenant`, `conversation`, trace) ne doivent contenir ni PII
ni token réhydratable ; ils doivent être bornés, allowlistés et injectés côté
gateway. Codex permet des headers statiques ou issus de variables, mais ne définit
pas le schéma métier Hacienda. [Référence officielle Codex — `http_headers` et `env_http_headers`](https://developers.openai.com/codex/config-reference#configtoml)

### Body Responses et conservation du contexte

Le type de requête Responses officiel de Codex contient au minimum `model`, `stream`,
`input` (tableau d’items), `tool_choice`, `parallel_tool_calls`, `reasoning`,
`store`, `include`, et éventuellement `tools`, `text`, `client_metadata` et les
options de cache. Les champs et items doivent être traités comme une enveloppe
ouverte versionnée : un gateway ne doit pas aplatir `input[]` en une seule chaîne
`messages[].content`. [Source officielle Codex — `ResponsesApiRequest`](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/common.rs#L324-L354)

La Responses API supporte trois stratégies de contexte : rejouer les output items
dans `input`, chaîner via `previous_response_id`, ou utiliser une Conversation
persistante. `previous_response_id` ne reporte pas les instructions top-level ; une
implémentation doit donc préserver/rejouer explicitement les instructions autorisées
et ne doit jamais envoyer de valeur brute pour reconstruire le contexte. [Guide officiel — état de conversation](https://developers.openai.com/api/docs/guides/conversation-state?api-mode=responses),
[guide officiel — migration vers Responses](https://developers.openai.com/api/docs/guides/migrate-to-responses)

**Décision proposée pour Hacienda :** le gateway conserve un `context_id` Hacienda
opaque pour le routage/audit, mais laisse Codex conserver la sémantique Responses.
Il relaie `input`, `tools`, `reasoning`, `text`, `store`, `previous_response_id`
ou `conversation` quand ces champs sont présents et autorisés, après redaction
structurelle. Il ne doit pas substituer son propre transcript texte à l’historique
Responses. La persistance documentaire reste token-only côté Hacienda/LanceDB ; le
viewer original reste hors contexte agent. [`#129` — versions, LanceDB et viewer](../superpowers/specs/2026-11-14-wayfinder-129-editor-viewer-lancedb-contract.md#deux-profils-d-installation)

**À décider :** `store=false` par défaut avec replay contrôlé des items chiffrés,
ou `store=true` avec politique de rétention du gateway ; autoriser ou non
`previous_response_id`/`conversation` lorsque l’identifiant vient d’un autre
tenant ; et si les réponses doivent être persistées par le gateway pour garantir
une reprise après reconnexion. OpenAI documente que les Responses sont stockées par
défaut et que `store=false` désactive cette persistance ; ce choix n’est donc pas
neutre pour la conformité. [Guide officiel — rétention et contexte](https://developers.openai.com/api/docs/guides/migrate-to-responses#4-decide-when-to-store-state)

## SSE : familles nécessaires au client Codex

Le transport HTTP `stream=true` est un flux Server-Sent Events. Le guide officiel
identifie comme cycle minimal `response.created`, `response.output_text.delta`,
`response.completed`, et `error`. Le gateway doit conserver le framing SSE
(`event:`/`data:`/ligne vide), l’ordre et les `sequence_number`; il ne doit pas
convertir les événements en `choices[].delta`. [Guide officiel — streaming Responses](https://developers.openai.com/api/docs/guides/streaming-responses#read-the-responses)

| Famille | Événements utiles | Traitement Hacienda proposé |
| --- | --- | --- |
| Cycle et finalisation | `response.created`, `response.in_progress`, `response.completed`, `response.incomplete`, `response.failed`, `error` | Valider le schéma, relayer les identifiants opaques et les statuts ; terminer proprement ou fail-closed. [`streaming-events`](https://developers.openai.com/api/reference/resources/responses/streaming-events) |
| Texte assistant | `response.output_item.added`, `response.content_part.added`, `response.output_text.delta`, `response.output_text.done`, `response.content_part.done`, `response.output_item.done` | Redacter chaque `delta` et chaque valeur finale ; conserver indices et ordre. [`output_text.delta`](https://developers.openai.com/api/reference/resources/responses/streaming-events#responseoutput_textdelta), [`output_item.done`](https://developers.openai.com/api/reference/resources/responses/streaming-events#responseoutput_itemdone) |
| Raisonnement exposé par l’API | `response.reasoning_summary_part.added`, `response.reasoning_summary_text.delta/done`, `response.reasoning_text.delta`, `response.output_item.done` | Ne jamais supposer que le raisonnement est absent ; redacter ou refuser selon politique. Pour `encrypted_content`, conserver le blob opaque sans l’interpréter. [`reasoning_summary_text.delta`](https://developers.openai.com/api/reference/resources/responses/streaming-events#responsereasoning_summary_textdelta) |
| Outils/function calling | `response.function_call_arguments.delta/done`, `response.custom_tool_call_input.delta/done` | Les arguments sont du JSON partiel : bufferiser par item/call, redacter structurellement, valider avant de laisser Codex poursuivre. Ne jamais exécuter un outil au gateway sans policy Hacienda. [`function_call_arguments.delta`](https://developers.openai.com/api/reference/resources/responses/streaming-events#responsefunction_call_argumentsdelta) |
| Refus et MCP | `response.refusal.delta/done`, `response.mcp_call_arguments.delta/done`, `response.mcp_call.completed/failed` | Relayer après validation ; les arguments MCP restent une surface de contenu et ne doivent pas contourner la redaction. [`refusal.delta`](https://developers.openai.com/api/reference/resources/responses/streaming-events#responserefusaldelta), [`mcp_call_arguments.delta`](https://developers.openai.com/api/reference/resources/responses/streaming-events#response.mcp_call_argumentsdelta) |
| Métadonnées et compaction | `response.metadata`, événements de compaction, `response.output_item.done` pour le contenu final chiffré | Allowlist stricte des métadonnées ; ne jamais placer PII dans les annotations, headers ou logs. Conserver les événements nécessaires au contexte et aux retries. [`streaming-events`](https://developers.openai.com/api/reference/resources/responses/streaming-events) |

Le code Codex officiel consomme explicitement les familles texte, `output_item`,
raisonnement, custom tool input, `response.failed`, `response.completed` et
`response.incomplete`; il tolère d’autres événements Responses structurés. Ce
code est la référence d’interopérabilité côté CLI. [Source officielle Codex — parseur SSE Responses](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/sse/responses.rs#L400-L580)

**Règle de relay proposée :** le gateway peut transmettre sans transformation les
champs purement structurels et les valeurs opaques, mais doit redacter les champs
textuels, les arguments d’outils et tout `output` avant émission. Un événement
Responses inconnu ne doit pas être supprimé silencieusement : s’il ne peut pas être
classifié comme token-only, le flux s’arrête avec une erreur Responses générique.
Cette stratégie suit l’exigence locale de `#122` sans dépendre d’un numéro de
version SSE figé. [`#122` — contrat de réponse tokenisée](../superpowers/specs/2026-11-14-wayfinder-122-gateway-response-contract.md#contrat-de-réponse-streaming)

## Audit du code courant : Codex Tauri n’est pas un client Codex CLI

Le code local a déjà un enum `ProviderKind::Codex`, les IDs de conversation/contexte,
une redaction avant transport et une réponse tokenisée. Il sélectionne aussi un
gateway par profil `LocalFull` ou `TeamOnPremise`, avec fail-closed si la configuration
ou le détecteur PII manque. [Code local — contrats provider](../../../crates/hacienda-client-core/src/provider.rs#L60-L105),
[code local — chemin Tauri](../../../apps/hacienda-secrecy-client/src-tauri/src/main.rs#L1323-L1341)

Mais l’adaptateur Codex courant :

- sérialise `messages: [{role: "user", content: ...}]` au lieu de `input` avec des
  Response Items ;
- envoie un `endpoint` déjà fourni par Hacienda, sans contrat explicite de base URL
  `/v1` + suffixe `/responses` ;
- transmet un Bearer, mais sans `Accept: text/event-stream` explicite ni prise en
  charge de la configuration provider Codex (`wire_api`, `env_key`, retries) ;
- parse des objets `choices[].delta.content` et ignore `[DONE]`, donc ne voit pas
  `response.output_text.delta`, les output items, les tool calls, le raisonnement,
  `response.failed` ou `response.incomplete` ;
- a un `ProviderTransport` synchrone qui collecte une réponse complète, alors que
  Codex CLI consomme un flux SSE incrémental avec timeout d’inactivité/retries.

Ces différences sont observables dans le code local et incompatibles avec le contrat
officiel Responses. [Code local — body/headers Chat Completions](../../../crates/hacienda-client-core/src/provider.rs#L802-L865),
[code local — parsing SSE Chat Completions](../../../crates/hacienda-client-core/src/provider.rs#L1040-L1090),
[source officielle Codex — timeout et route Responses](https://github.com/openai/codex/blob/main/codex-rs/model-provider-info/src/lib.rs#L193-L228)

**Conclusion d’implémentation :** un nouvel adaptateur `CodexResponsesProvider`
est nécessaire. Il doit recevoir le body Responses token-only, faire `POST
{base_url}/responses`, demander SSE, décoder en flux, redacter par événement ou
par unité logique sûre, et préserver les items/indices/IDs. L’actuel adaptateur
Chat Completions ne peut rester qu’une surface legacy séparée ; il ne doit pas être
annoncé comme support Codex CLI. [Source officielle Codex — client Responses](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/endpoint/responses.rs#L60-L169)

## LocalFull et TeamOnPremise

### LocalFull

La spécification locale place Hacienda/Xberg, LanceDB Embedded, le coffre et le
viewer sur le poste ; Claude/Codex ne reçoivent que la projection tokenisée. Le
gateway peut écouter en loopback HTTP, mais le provider Codex doit pointer vers sa
base Responses et ne doit jamais recevoir le chemin de coffre, les octets originaux
ou une `RehydrationMap`. [`#129` — LocalFull](../superpowers/specs/2026-11-14-wayfinder-129-editor-viewer-lancedb-contract.md#local-full)

```text
Codex CLI
  -> http://127.0.0.1:<port>/v1/responses
  -> Hacienda gateway local : redaction + contexte + audit
  -> provider configuré : Responses API
```

Le CLI et le viewer Tauri sont deux surfaces séparées : le viewer peut demander une
réhydratation auditée, mais aucune valeur réhydratée ne doit entrer dans `input`,
les tool calls, le transcript Codex ou les logs. [`#122` — réhydratation](../superpowers/specs/2026-11-14-wayfinder-122-gateway-response-contract.md#réhydratation)

### TeamOnPremise

La spécification locale fait de Tauri un client de session/viewer ; le gateway
on-premise résout le tenant, applique les ACL et écrit l’audit, tandis que LanceDB
reste server-side token-only. Le provider Codex doit donc utiliser HTTPS, une
credential de session tenant-scoped et une base URL Responses du gateway ; aucun
client ne doit se connecter directement à LanceDB. [`#129` — TeamOnPremise](../superpowers/specs/2026-11-14-wayfinder-129-editor-viewer-lancedb-contract.md#team-on-premise)

```text
Codex CLI / Tauri
  -> https://gateway.example.test/gateway/v1/responses
  -> auth + tenant ACL + redaction + audit
  -> provider Responses
```

Le transport Codex ne doit pas transmettre un `tenant_id` arbitraire dans le body
ou les headers comme preuve d’autorité : le tenant doit venir de l’authentification
du gateway et des ACL. Cette contrainte est une décision Hacienda dérivée de la
frontière de confiance locale, pas une fonctionnalité fournie par Codex. [`#124` — confiance MCP/HTTP](../superpowers/specs/2026-11-14-wayfinder-124-integration-boundary-research.md#sécurité-et-frontière-de-confiance)

## Confirmé, proposé, à décider

| Sujet | Confirmé par source | Décision proposée / reste ouvert |
| --- | --- | --- |
| Wire API | Codex CLI actuel accepte seulement `responses`. [Codex source](https://github.com/openai/codex/blob/main/codex-rs/model-provider-info/src/lib.rs#L110-L141) | Utiliser `wire_api = "responses"`; ne pas implémenter un shim Chat Completions pour le CLI. |
| Base URL | Codex concatène `/responses`; l’exemple officiel utilise une base `/v1`. [Codex proxy](https://github.com/openai/codex/blob/main/codex-rs/responses-api-proxy/README.md#tl;dr) | Faire terminer les endpoints provider Hacienda par `/v1`; décider si les profils génériques existants sont migrés ou si un champ `responses_base_url` est ajouté. |
| Route | `POST /v1/responses` est la route autorisée par le proxy officiel. [Codex proxy](https://github.com/openai/codex/blob/main/codex-rs/responses-api-proxy/README.md#behavior) | Exposer exactement cette route relative ; refuser Chat Completions, query strings non allowlistées et endpoints directs provider depuis Tauri. |
| Auth | `env_key` existe ; l’auth attendue est Bearer. [Codex config](https://developers.openai.com/codex/config-reference#configtoml), [Responses create](https://developers.openai.com/api/reference/resources/responses/methods/create#request) | Token loopback LocalFull et session tenant-scoped TeamOnPremise ; choisir le mécanisme OAuth/command-backed Team. |
| Body | Responses utilise `input`/output Items typés, outils, raisonnement et contrôles de texte. [Codex request](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/common.rs#L324-L354), [Responses API](https://developers.openai.com/api/docs/guides/migrate-to-responses) | Préserver la forme structurée ; redacter chaque surface textuelle et argument ; ne pas aplatir. |
| Contexte | `previous_response_id`, replay des items et Conversations sont documentés. [Conversation state](https://developers.openai.com/api/docs/guides/conversation-state?api-mode=responses) | `context_id` Hacienda pour routage/audit, IDs Responses préservés si autorisés ; décider `store`/rétention et stratégie de reprise. |
| SSE | Événements lifecycle, texte, items, outils, reasoning, refus, erreurs et compaction sont documentés. [Streaming events](https://developers.openai.com/api/reference/resources/responses/streaming-events) | Relay SSE Responses validé, ordre/sequence conservés, unknown fail-closed ; décider quels événements non utilisés sont seulement relayés ou refusés. |
| Compatibilité actuelle | Le Tauri local est Chat Completions. [Code local](../../../crates/hacienda-client-core/src/provider.rs#L802-L865) | Ajouter un transport Responses dédié et des tests contractuels avec fixtures officielles ; aucun code n’est modifié par cette note. |

## Décisions d’implémentation proposées

1. **Ajouter une frontière provider Responses distincte.** Le type de transport doit
   porter `base_url`, `wire_api`, credential provider, headers allowlistés, timeouts
   SSE et retries ; `wire_api=chat` doit être rejeté pour Codex CLI. [Codex config](https://developers.openai.com/codex/config-reference#configtoml)
2. **Faire du body Responses une structure opaque mais validée.** Autoriser les
   champs documentés, préserver les items et l’ordre, et versionner la validation.
   Toute redaction porte sur les valeurs, jamais sur une conversion en `messages`.
   [Responses request](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/common.rs#L324-L354)
3. **Rédiger avant sortie et avant persistance.** Inspecter `input`, instructions,
   tools, arguments, `client_metadata`, output items, deltas de texte/reasoning et
   erreurs contenant un message ; ne jamais persister de plaintext dans LanceDB,
   transcript ou annotation UI. [`#122`](../superpowers/specs/2026-11-14-wayfinder-122-gateway-response-contract.md#contrat-de-requête)
4. **Relaier le SSE incrémental Responses.** Ne pas bufferiser la réponse complète
   comme le fait le chemin local actuel ; appliquer une politique de buffering
   bornée par item pour les JSON partiels d’arguments, avec échec fermé si la
   redaction sûre n’est pas possible. [Streaming officiel](https://developers.openai.com/api/docs/guides/streaming-responses), [Codex SSE parser](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/sse/responses.rs#L400-L580)
5. **Séparer transcript et contexte Hacienda.** Conserver les IDs Responses et
   `context_id` opaque dans une table d’audit token-only ; ne jamais utiliser un
   ID comme capacité de révélation. [`#129`](../superpowers/specs/2026-11-14-wayfinder-129-editor-viewer-lancedb-contract.md#mark-tiptap-sensible)
6. **Tester la matrice de compatibilité.** Fixtures minimales : texte simple,
   multi-delta PII split, output item, tool call JSON split, reasoning summary,
   refus, `response.failed`, `response.incomplete`, `error`, événement inconnu,
   reconnexion SSE, `store=false` + replay encrypted reasoning, et cross-tenant
   ID rejection. [Événements officiels](https://developers.openai.com/api/reference/resources/responses/streaming-events)
7. **Traiter le MCP séparément.** Le Codex CLI peut avoir des outils MCP, mais le
   provider Responses ne doit pas devenir un tunnel vers `pii_reveal`; la capacité
   viewer humain doit rester hors contexte agent et passer par la politique Hacienda.
   [`#124`](../superpowers/specs/2026-11-14-wayfinder-124-integration-boundary-research.md#surface-mcp-réellement-disponible)

## Ce qui reste à spécifier avant code

- Le schéma exact de `ResponsesGatewayRequest` Hacienda et la liste de champs
  Responses autorisés par modèle/version.
- La politique `store`, `previous_response_id`, `conversation` et rétention en
  LocalFull versus TeamOnPremise.
- Le mécanisme de credential TeamOnPremise et la rotation/retry sans rejouer une
  requête pouvant contenir des tool calls.
- Le mapping des annotations token-only vers Tauri, puisque Codex CLI ne fournit
  pas le canal riche de marks Tiptap du viewer local.
- Le support ou refus des images/files/hosted tools : la Responses API les définit,
  mais Hacienda doit inspecter chaque octet ou refuser la surface avant egress.
  [Create a Response — inputs et outils](https://developers.openai.com/api/reference/resources/responses/methods/create)
- Un contrat de compatibilité avec le rythme réel du flux : idle timeout,
  `stream_max_retries`, `request_max_retries`, et conservation des headers de
  corrélation. [Configuration Codex — retries/timeouts](https://developers.openai.com/codex/config-reference#configtoml)

## Sources primaires

- [Codex CLI — configuration reference](https://developers.openai.com/codex/config-reference)
- [Codex source — provider wire API](https://github.com/openai/codex/blob/main/codex-rs/model-provider-info/src/lib.rs)
- [Codex source — Responses HTTP endpoint](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/endpoint/responses.rs)
- [Codex source — Responses request](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/common.rs)
- [Codex source — Responses SSE parser](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/sse/responses.rs)
- [Codex official Responses API proxy](https://github.com/openai/codex/blob/main/codex-rs/responses-api-proxy/README.md)
- [OpenAI Responses API — create](https://developers.openai.com/api/reference/resources/responses/methods/create)
- [OpenAI Responses API — streaming guide](https://developers.openai.com/api/docs/guides/streaming-responses)
- [OpenAI Responses API — streaming events](https://developers.openai.com/api/reference/resources/responses/streaming-events)
- [OpenAI — conversation state](https://developers.openai.com/api/docs/guides/conversation-state?api-mode=responses)
- [OpenAI — migration to Responses](https://developers.openai.com/api/docs/guides/migrate-to-responses)

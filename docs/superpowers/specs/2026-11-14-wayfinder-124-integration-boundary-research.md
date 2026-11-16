# Wayfinder #124 — frontière d’intégration Claude Desktop / Anthropic / Codex / MCP

**Date de l’enquête :** 2026-11-14<br>
**Périmètre :** checkout courant de `hacienda-engine`, historique et configuration locale.<br>
**Méthode :** lecture du code, des tests, des specs/plans, des manifests et de l’historique
local. Aucune action GitHub et aucune modification de code/configuration.

## Conclusion opérationnelle

Le dépôt possède déjà une frontière MCP exploitable par un hôte local :

```text
Claude Desktop ou Codex CLI
        │ MCP stdio / JSON-RPC
        ▼
`hacienda mcp serve`
        │ `HaciendaMcp` → `HaciendaFacade` → pipeline PII/audit
        ▼
résultats redacted-by-default
```

Cette frontière n’est pas une intégration Anthropic. Dans le chemin implémenté ici,
Claude Desktop est l’hôte MCP qui appelle les outils ; le serveur Hacienda ne parle pas
directement à l’API Anthropic. Le dépôt ne contient pas de SDK/provider Anthropic, de
secret Anthropic, de fichier `claude_desktop_config.json`, ni d’enregistrement Hacienda
prêt à l’emploi dans Codex.

La seconde frontière, indépendante, est l’API REST de synthèse RAG : elle accepte une
configuration LLM générique et envoie prompt/contexte redacted à `liter-llm` via
`GuardedLlm`. Elle n’est pas exposée comme outil MCP aujourd’hui.

## État du checkout et historique récent

Le checkout courant est `jamon8888/lance` à `613e6de` (2026-08-26). La ref locale
`issue-117` pointe vers `58238a1` (2026-11-14), un enfant direct du checkout courant et
donc un changement ultérieur non présent dans l’arbre étudié (`git rev-list --left-right
--count HEAD...issue-117` donne `0 1`). Cette ref historique ajoute une extension MCP
importante — `get_capabilities`, entrées bytes/URI structurées, et `rmcp 3.5.1` — mais elle
ne doit pas être décrite comme présente dans le checkout courant. Voir le commit
`58238a1` et sa nouvelle note `hacienda-mcp/docs/xberg-1.3.10-parity.md` dans cette ref.

## Matrice présent / absent

| Surface | Présent dans le checkout courant | Absent ou contraint |
| --- | --- | --- |
| **MCP Hacienda** | Crate `hacienda-mcp`, membre du workspace ; commande `hacienda mcp serve` ; 8 outils ; transport stdio. [`Cargo.toml`](../../../Cargo.toml#L1-L4), [`hacienda-cli/Cargo.toml`](../../../hacienda-cli/Cargo.toml#L27-L36), [`hacienda-cli/src/cli.rs`](../../../hacienda-cli/src/cli.rs#L68-L99) | Pas de transport HTTP/Streamable HTTP, pas de resources, prompts, tasks ou completions. [`hacienda-mcp/Cargo.toml`](../../../hacienda-mcp/Cargo.toml#L23-L32), [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L358-L382) |
| **Claude Desktop** | Cas d’usage explicitement visé : l’hôte lance `hacienda mcp serve` comme processus enfant et parle JSON-RPC sur stdin/stdout. [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L22-L31), [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L96-L104) | Aucun fichier de configuration Claude Desktop dans le checkout ; la spec ne donne qu’un exemple de config (`command: "hacienda"`, `args: ["mcp", "serve"]`). Le test littéral dans Claude Desktop est resté non vérifié ; la validation existante est protocolaire/stdio et mécanique. [`superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`](../../../superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md#L35-L44), [`superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`](../../../superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md#L249-L259), [`superpowers/plans/2026-07-30-hacienda-program-plan.md`](../../../superpowers/plans/2026-07-30-hacienda-program-plan.md#L1051-L1074) |
| **Anthropic** | Aucune intégration directe identifiée. Le seul chemin LLM actuel est REST/RAG : `AnswerRequest.llm` reprend `xberg::LlmConfig` et est passé à `liter-llm`; `GuardedLlm` redacts prompt et chunks avant l’appel. [`hacienda-api/src/dto.rs`](../../../hacienda-api/src/dto.rs#L1045-L1075), [`hacienda-api/src/handlers/rag_stream.rs`](../../../hacienda-api/src/handlers/rag_stream.rs#L1-L12), [`crates/hacienda-rag/src/stream.rs`](../../../crates/hacienda-rag/src/stream.rs#L111-L186) | Pas de crate/configuration/secret Anthropic, pas d’outil MCP `rag answer`, pas de pont Claude Desktop → API Anthropic appartenant à Hacienda. L’endpoint de synthèse reste REST et requiert `documents:process`. [`hacienda-api/src/routes.rs`](../../../hacienda-api/src/routes.rs#L300-L308) |
| **Codex CLI** | Le harnais de développement prévoit `AGENTS.md`, `.codex/` et une génération ai-rulez à partir de `.ai-rulez/config.toml`; la norme est documentée pour Codex/OpenCode. [`docs/setup-harnais-ia-fr.md`](../../../docs/setup-harnais-ia-fr.md#L1-L34), [`docs/setup-harnais-ia-fr.md`](../../../docs/setup-harnais-ia-fr.md#L191-L201) | Les fichiers générés `AGENTS.md` et `.codex/` sont absents du checkout courant et ignorés. `.ai-rulez/config.toml` ne déclare que le serveur MCP Playwright, pas `hacienda mcp serve`. [`docs/setup-harnais-ia-fr.md`](../../../docs/setup-harnais-ia-fr.md#L61-L75), [`.ai-rulez/config.toml`](../../../.ai-rulez/config.toml#L57-L65), [`.gitignore`](../../../.gitignore#L151-L155) |
| **API/SDK** | API REST OpenAPI et SDK Python/TypeScript générés ; les intégrations LangChain utilisent l’API et reçoivent du contenu déjà redacted. [`integrations/README.md`](../../../integrations/README.md#L1-L23), [`integrations/README.md`](../../../integrations/README.md#L25-L39) | Le MCP n’est pas un SDK HTTP et aucun client Anthropic n’est ajouté aux SDKs. La spec M1 demande explicitement de garder MCP et SDK programmatiques séparés. [`superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`](../../../superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md#L304-L320) |

## Surface MCP réellement disponible

`hacienda-mcp` expose exactement les outils suivants, tous comme wrappers de la même
`HaciendaFacade` que l’API/CLI : `documents_process`, `pii_scan`, `pii_redact`,
`pii_reveal`, `audit_verify`, `audit_entries`, `compliance_report` et `get_version`.
L’inventaire et les correspondances sont documentés dans le module. [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L33-L47)

Les tests présents dans le crate vérifient notamment :

- une valeur de contrôle n’apparaît ni dans le bloc texte ni dans `structured_content` de
  `pii_redact` ; [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L455-L505)
- `pii_scan` omet le texte des entités (`SpanText::Omit`) ; [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L140-L160), [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L508-L529)
- l’absence de configuration PII échoue explicitement au lieu de retourner du texte brut ; [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L531-L545)
- l’ensemble des outils enregistrés est verrouillé par un test d’inventaire à huit noms. [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L547-L569)

`documents_process` accepte actuellement une liste de chaînes représentant des chemins
locaux ou des URI `file://`. Il n’accepte pas de mode de redaction par appel ; la politique
reste celle de la configuration serveur. [`hacienda-mcp/src/params.rs`](../../../hacienda-mcp/src/params.rs#L5-L23)

Le résultat MCP est dupliqué en bloc texte JSON et en `structured_content`, donc les deux
sont des surfaces de divulgation prises en compte par le code. [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L385-L395)

## Ce qui n’est pas exposé par MCP

La route REST contient une surface bien plus large, notamment RAG et la synthèse
`/v1/rag/collections/{name}/answer`; le serveur MCP courant ne la reflète pas. La spec M1
avait envisagé une génération depuis `ROUTE_TABLE`, environ 40 routes, des resources et un
transport HTTP, mais son tableau de séquencement reconnaît que M1a a livré huit outils
écrits à la main et laisse ces éléments ouverts. [`superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`](../../../superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md#L177-L228), [`superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`](../../../superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md#L324-L338)

La feature MCP native de xberg est volontairement désactivée. Le workspace explique que
ce serveur xberg retourne du contenu documentaire non redacted et contournerait
`HaciendaFacade`; le crate Hacienda ne doit donc ni l’activer ni l’importer. [`Cargo.toml`](../../../Cargo.toml#L186-L190), [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L7-L20)

## Sécurité et frontière de confiance

### Stdio local : confiance du processus, pas authentification par appel

Tous les handlers MCP appellent la facade avec `Caller::Trusted`. Le modèle de sécurité
du core définit cela comme un appel in-process où la frontière de confiance est la
frontière du processus ; aucune capability n’est appliquée et aucun principal n’est
attribué. [`hacienda-core/src/auth/mod.rs`](../../../hacienda-core/src/auth/mod.rs#L213-L250), [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L22-L31)

Conséquences concrètes pour un hôte MCP local :

- `documents_process` peut lire les chemins locaux fournis par l’appelant ; le commentaire
  explique que l’absence de restriction HTTP est intentionnelle parce qu’il n’y a pas de
  wire réseau dans ce transport. [`hacienda-mcp/src/params.rs`](../../../hacienda-mcp/src/params.rs#L8-L18)
- `pii_reveal` peut retourner le plaintext si une clé de pseudonymisation est configurée ;
  ses erreurs de token sont volontairement génériques pour éviter un oracle. [`hacienda-mcp/src/lib.rs`](../../../hacienda-mcp/src/lib.rs#L182-L217)
- la présence d’un MCP server local ne doit donc pas être interprétée comme une isolation
  par capability entre plusieurs utilisateurs du même processus.

### Réseau : le contrôle existe pour REST, pas pour MCP

Le serveur REST refuse un bind non-loopback sans authentification et refuse également le
placeholder de token public de `config/production.toml`. [`hacienda-cli/src/commands.rs`](../../../hacienda-cli/src/commands.rs#L1421-L1479)
Les routes HTTP utilisent ensuite `AuthState`/bearer tokens et `Caller::Principal`; une
requête sans contexte n’est interprétée comme `Trusted` que lorsque l’auth middleware a
explicitement désactivé l’authentification. [`hacienda-api/src/handlers/mod.rs`](../../../hacienda-api/src/handlers/mod.rs#L28-L60), [`hacienda-api/src/routes.rs`](../../../hacienda-api/src/routes.rs#L422-L437)

Il n’existe pas d’équivalent MCP HTTP dans le checkout : `hacienda-mcp` active seulement
`server`, `macros` et `transport-io`, et commente le transport HTTP comme différé.
[`hacienda-mcp/Cargo.toml`](../../../hacienda-mcp/Cargo.toml#L23-L32)

### Audit : preuve locale, mais persistance volatile dans ce chemin

`HaciendaFacade::new` utilise un `InMemoryAuditStore` ; la documentation du core précise
que l’état est perdu à la destruction et qu’il faut `with_stores` + `FileAuditStore` pour
survivre à un redémarrage. [`hacienda-core/src/facade.rs`](../../../hacienda-core/src/facade.rs#L178-L193)
La commande MCP construit la facade de façon longue durée et sert jusqu’à la déconnexion,
mais ne ferme pas explicitement les stores à la fin du processus. [`hacienda-cli/src/commands.rs`](../../../hacienda-cli/src/commands.rs#L1482-L1514)
La spec M1 et le changelog signalent la même lacune : la session MCP peut auditer son
processus courant, mais un redémarrage perd la chaîne/review en mémoire. [`CHANGELOG.md`](../../../CHANGELOG.md#L372-L389)

## Frontières recommandées pour Wayfinder #124

1. **Claude Desktop / Codex :** traiter Hacienda comme un serveur MCP local stdio. Leur
   responsabilité est l’enregistrement du processus et le consentement local ; la
   responsabilité Hacienda est de maintenir les outils redacted-by-default derrière
   `HaciendaFacade`.
2. **Anthropic :** ne pas confondre l’hôte Claude Desktop avec un provider intégré au
   serveur. Si le ticket veut appeler l’API Anthropic depuis Hacienda, il s’agit d’un
   nouveau chemin provider/credentials, distinct du MCP actuel, qui doit conserver le
   garde-fou `GuardedLlm` et définir l’audit de sortie réseau. Aujourd’hui, aucun tel
   chemin n’est présent.
3. **MCP distant :** ne pas exposer le stdio/`Caller::Trusted` sur un socket. Il faudrait
   un transport MCP HTTP avec propagation de `Caller::Principal`, authentification et
   capabilities identiques aux routes REST ; la spec M1 le formule comme M1b mais ce n’est
   pas implémenté. [`superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`](../../../superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md#L220-L240)
4. **Validation à ne pas sur-promettre :** la compatibilité MCP/stdio est démontrée par le
   code et les tests locaux ; une validation de bout en bout dans l’application Claude
   Desktop, ainsi qu’une configuration Codex partagée, restent à faire hors du checkout.

## Addendum — gateway Claude officiel et séparation MCP

La documentation officielle Claude Code confirme qu’un client Claude Code peut pointer vers
un gateway Anthropic-compatible via `ANTHROPIC_BASE_URL`, avec `/v1/messages` et un relay SSE
conforme. Cette possibilité ne change pas le constat du présent audit : aucun gateway provider
n’est implémenté dans le checkout Hacienda et elle ne transforme pas le serveur MCP stdio en
proxy LLM.

La cible doit donc conserver deux frontières indépendantes :

- **MCP stdio** : outils locaux redacted-by-default, sans valeur PII réhydratée ;
- **Gateway Anthropic** : transport provider, redaction structurelle, validation du flux et
  contrôle d’egress.

La spec détaillée du gateway et de la réponse tokenisée est
[`2026-11-14-wayfinder-122-gateway-response-contract.md`](2026-11-14-wayfinder-122-gateway-response-contract.md).

## Addendum — état au 2026-11-15

Le constat « aucun gateway provider n’est implémenté » est historique et a été dépassé :
`hacienda serve` peut maintenant activer le gateway Anthropic Messages via une configuration
serveur fail-closed (`5d9fd01`), et le relais SSE est incrémental (`2e448ad`). Cela reste
distinct du serveur MCP stdio, conformément à la recommandation de séparation ci-dessus.

La recherche Codex CLI a confirmé son contrat Responses API et est publiée dans
`docs/research/2026-11-14-wayfinder-132-codex-responses-gateway.md` (`2024222`). Le gateway
Responses et une configuration Codex partagée ne sont pas encore implémentés. Aucun essai
de bout en bout dans Claude Desktop n’a été réalisé.

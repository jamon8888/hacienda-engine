# Wayfinder #123 — sécurité effective du RAG hybride

Date de l’audit : 2026-11-14<br>
Dépôt audité : `jamon8888/hacienda-engine`, branche `lance` (`613e6de`)<br>
Méthode : lecture du code, des migrations et des tests présents dans le dépôt. Aucun fichier de code n’a été modifié.

## Conclusion exécutive

Le chemin HTTP `POST /v1/rag/collections/{name}/answer` possède une vraie barrière avant l’appel LLM : la collection est résolue, la requête est validée, le store récupère les chunks, puis `GuardedLlm` rédige le prompt utilisateur et chaque `chunk.content` avant de déléguer à `answer_stream`. Le test structurel interdit en plus les appels directs au client LLM hors du module propriétaire.

La garantie n’est toutefois pas complète :

1. `system_prompt`, pourtant transmis au fournisseur LLM, n’est pas rédigé par `GuardedLlm`.
2. Le scoping tenant n’est pas effectif via l’authentification réelle : `Token::to_auth_context` reconstruit toujours le tenant `default`, et `ApiKeyTokenResolver` perd le `tenant_id` de la clé.
3. Même avec ce défaut corrigé, le tenant `default` peut adresser directement un nom physique de la forme `acme:collection`, car son chemin de scoping laisse le nom inchangé. La liste masque ce nom, mais les opérations directes ne le rejettent pas.
4. Les filtres sont des contraintes de recherche, pas une ACL documentaire : tout principal autorisé sur une collection peut demander les chunks de cette collection; il n’existe pas de politique document/chunk par principal.
5. L’ingestion RAG et `POST .../retrieve` peuvent conserver/retourner du texte brut. La redaction est garantie à la frontière LLM du workflow `answer`, pas à la frontière de stockage ni de restitution RAG.

## Capacités de retrieval

Le contrat expose cinq modes dans [`crates/hacienda-rag/src/query.rs:15-43`](../../crates/hacienda-rag/src/query.rs#L15-L43) : `vector`, `full_text` (lexical), `hybrid`, `sparse` et `late_interaction`. `RetrieveQuery::validate` impose `top_k` 1..200, un multiplicateur de candidats 1..20, la dimension du vecteur, les entrées requises selon le mode et la validité du filtre ([`query.rs:114-208`](../../crates/hacienda-rag/src/query.rs#L114-L208)).

Les capacités sont backend-dépendantes :

| Backend | Vector | Lexical | Hybrid | Sparse | Late interaction | Filtres |
| --- | --- | --- | --- | --- | --- | --- |
| `InMemoryVectorStore` | oui | non | non | oui | oui | oui |
| `PgVectorStore` | oui | oui | oui | non | non | oui |

Sources : [`memory.rs:243-251`](../../crates/hacienda-rag/src/backends/memory.rs#L243-L251), [`pgvector.rs:278-292`](../../crates/hacienda-rag/src/backends/pgvector.rs#L278-L292). Le backend mémoire refuse explicitement lexical/hybride ([`memory.rs:398-407`](../../crates/hacienda-rag/src/backends/memory.rs#L398-L407)); le backend PostgreSQL refuse sparse/late-interaction et route vector, full-text et hybrid séparément ([`pgvector.rs:527-552`](../../crates/hacienda-rag/src/backends/pgvector.rs#L527-L552)). Il n’y a donc pas de dégradation silencieuse vers un autre mode.

Dans PostgreSQL, le lexical utilise `content_tsv` et `plainto_tsquery('english', ...)`; l’hybride fusionne les listes vectorielle et lexicale par reciprocal-rank fusion `k=60` ([`pgvector.rs:29-39`](../../crates/hacienda-rag/src/backends/pgvector.rs#L29-L39), [`pgvector.rs:885-935`](../../crates/hacienda-rag/src/backends/pgvector.rs#L885-L935)). Le mode hybride exige en pratique `query_text` et `query_vector` ([`pgvector.rs:892-906`](../../crates/hacienda-rag/src/backends/pgvector.rs#L892-L906)), alors que la validation IR ne rend obligatoire que le texte ([`query.rs:166-171`](../../crates/hacienda-rag/src/query.rs#L166-L171]) : l’écart est rejeté au store par une erreur, mais pourrait être resserré dans la validation commune.

## Filtres et périmètre document/chunk

Le langage de filtre autorise explicitement `doc.*` et `chunk.*`, avec une whitelist : texte, titre, MIME, identifiants externes, labels, entités, métadonnées, ordinal et métadonnées de chunk ([`filter.rs:38-97`](../../crates/hacienda-rag/src/filter.rs#L38-L97)). Les opérateurs sont égalité, appartenance, plage, tableau, recherche textuelle et booléens ([`filter.rs:100-165`](../../crates/hacienda-rag/src/filter.rs#L100-L165)). Les caps sont profondeur 8, 64 nœuds, 4 `text_match` et 1024 octets par requête textuelle ([`filter.rs:11-18`](../../crates/hacienda-rag/src/filter.rs#L11-L18), [`filter.rs:167-246`](../../crates/hacienda-rag/src/filter.rs#L167-L246)).

Le backend mémoire filtre les candidats avant le scoring ([`memory.rs:418-428`](../../crates/hacienda-rag/src/backends/memory.rs#L418-L428]). PostgreSQL ajoute le filtre dans chacune des deux recherches candidates, avant le classement/fusion ([`pgvector.rs:938-972`](../../crates/hacienda-rag/src/backends/pgvector.rs#L938-L972), [`pgvector.rs:975-1005`](../../crates/hacienda-rag/src/backends/pgvector.rs#L975-L1005)). Les champs sont compilés depuis une whitelist et les chemins JSON sont bindés, non interpolés; les valeurs sont des paramètres SQL ([`pgvector.rs:1248-1255`](../../crates/hacienda-rag/src/backends/pgvector.rs#L1248-L1255), [`pgvector.rs:1257-1360`](../../crates/hacienda-rag/src/backends/pgvector.rs#L1257-L1360)). Cela fournit une garantie d’intégrité de requête et de sélection, pas une autorisation par document.

Le résultat peut inclure `content`, les métadonnées de chunk et un résumé documentaire selon `include_content`/`include_document` ([`types.rs:333-358`](../../crates/hacienda-rag/src/types.rs#L333-L358)). `answer` force `include_content=true`, mais ne transforme pas les métadonnées en politique d’accès ([`rag_stream.rs:132-140`](../../hacienda-api/src/handlers/rag_stream.rs#L132-L140)).

## Tenant, collection, document et chunk

`hacienda-rag` est volontairement sans politique tenant : le trait et les types n’ont pas de `tenant_id` ([`crates/hacienda-rag/src/lib.rs:1-10`](../../crates/hacienda-rag/src/lib.rs#L1-L10)). L’API tente donc de cloisonner par nom : les tenants non-default obtiennent `tenant:name`, et tous les handlers RAG passent par `scope_collection_name` ([`hacienda-api/src/handlers/rag.rs:71-121`](../../hacienda-api/src/handlers/rag.rs#L71-L121), usages notamment [`rag.rs:377-399`](../../hacienda-api/src/handlers/rag.rs#L377-L399)). Les listes filtrent les noms physiques avant de les renvoyer ([`rag.rs:407-480`](../../hacienda-api/src/handlers/rag.rs#L407-L480)).

Deux lacunes rendent cette isolation non effective en production multi-tenant :

- Le modèle `Token` ne contient pas de tenant. `Token::to_auth_context` appelle `AuthContext::new`, qui choisit le tenant `default` ([`hacienda-core/src/auth/authn.rs:11-25`](../../hacienda-core/src/auth/authn.rs#L11-L25), [`authn.rs:76-85`](../../hacienda-core/src/auth/authn.rs#L76-L85), [`hacienda-core/src/auth/mod.rs:169-178`](../../hacienda-core/src/auth/mod.rs#L169-L178)).
- Les API keys ont bien un `tenant_id` en mémoire et en PostgreSQL ([`hacienda-core/src/auth/mod.rs:388-397`](../../hacienda-core/src/auth/mod.rs#L388-L397), [`hacienda-core/migrations/0008_api_keys_tenant.sql:1-13`](../../hacienda-core/migrations/0008_api_keys_tenant.sql#L1-L13)), mais `ApiKeyTokenResolver` retourne `Token::new(...)` sans reporter ce champ ([`hacienda-core/src/auth/authn.rs:240-277`](../../hacienda-core/src/auth/authn.rs#L240-L277)). Les tests RAG contournent explicitement cette résolution en injectant manuellement `AuthContext::with_tenant`, et documentent le défaut ([`hacienda-api/src/handlers/rag.rs:974-979`](../../hacienda-api/src/handlers/rag.rs#L974-L979)).

Il existe en outre un alias de nom dangereux : pour `default`, `scope_collection_name` retourne le nom appelant inchangé ([`rag.rs:88-93`](../../hacienda-api/src/handlers/rag.rs#L88-L93)). Ainsi, si `acme` possède `contracts`, le nom physique est `acme:contracts`; un appelant du tenant `default` peut fournir littéralement `acme:contracts` à `get`, `retrieve`, `upsert`, `reindex` ou `delete`, qui réutilisent tous cette fonction. La protection présente ne concerne que la liste (`strip_tenant_prefix`), et le test existant ne vérifie que l’absence du nom dans cette liste ([`rag.rs:954-964`](../../hacienda-api/src/handlers/rag.rs#L954-L964)).

Une fois une collection choisie, le périmètre document/chunk est cohérent dans les chemins normaux : le backend mémoire stocke les chunks sous la collection ([`memory.rs:293-345`](../../crates/hacienda-rag/src/backends/memory.rs#L293-L345)); PostgreSQL porte `collection` sur documents et chunks, et les recherches imposent `c.collection = $1` ([`0100_init.sql:34-48`](../../crates/hacienda-rag/migrations/0100_init.sql#L34-L48), [`0100_init.sql:63-97`](../../crates/hacienda-rag/migrations/0100_init.sql#L63-L97), [`pgvector.rs:826-846`](../../crates/hacienda-rag/src/backends/pgvector.rs#L826-L846)). Les accès par identifiant vérifient aussi la collection pour le document ([`pgvector.rs:667-712`](../../crates/hacienda-rag/src/backends/pgvector.rs#L667-L712)). Il n’y a cependant aucune ACL document/chunk au-delà de la collection et du capability `documents:process`.

## Chemin `retrieve → answer → LLM`

Le routeur protège toutes les routes RAG, y compris `answer`, par `Capability::DocumentsProcess`, avec refus par défaut des routes non enregistrées ([`hacienda-api/src/routes.rs:257-307`](../../hacienda-api/src/routes.rs#L257-L307), [`hacienda-core/src/auth/authz.rs:218-272`](../../hacienda-core/src/auth/authz.rs#L218-L272)). Le handler `answer` :

1. calcule le nom de collection scoped et vérifie son existence;
2. force `include_content=true`, valide la requête et appelle `store.retrieve`;
3. construit `RetrievedContext`;
4. construit un `FacadeRedactor` basé sur `HaciendaFacade::redact_text_with_auth`;
5. appelle `GuardedLlm::stream` ([`hacienda-api/src/handlers/rag_stream.rs:114-161`](../../hacienda-api/src/handlers/rag_stream.rs#L114-L161)).

`redact_text_with_auth` exige `documents:process`, sélectionne le pipeline du tenant et audite les spans rédigés ([`hacienda-core/src/facade.rs:1180-1223`](../../hacienda-core/src/facade.rs#L1180-L1223)). `GuardedLlm` rédige le prompt utilisateur puis chaque `RetrievedChunk::content` avant même de construire le stream ([`crates/hacienda-rag/src/stream.rs:133-185`](../../crates/hacienda-rag/src/stream.rs#L133-L185)). `answer_stream` compose le contexte uniquement à partir de `content` et l’envoie dans le message système générique ([`stream.rs:207-244`](../../crates/hacienda-rag/src/stream.rs#L207-L244)); l’appel réseau réel est privé au même module ([`stream.rs:280-329`](../../crates/hacienda-rag/src/stream.rs#L280-L329)).

## Redaction : garanties et lacunes

Garanties effectives dans le chemin HTTP actuel :

- le prompt utilisateur et tous les contenus de chunks passent par le redactor avant l’appel réseau;
- une erreur de rédaction stoppe la construction du stream et donc l’appel LLM;
- le seul appel HTTP actuel construit `GuardedLlm`;
- le test structurel scanne les `src/` du workspace et bloque les appels directs courants à `liter_llm`/`chat_stream` ([`crates/hacienda-rag/tests/llm_call_enforcement.rs:18-78`](../../crates/hacienda-rag/tests/llm_call_enforcement.rs#L18-L78)).

Lacunes :

- `LlmAnswerConfig::system_prompt` est décrit comme un override utilisé verbatim ([`crates/hacienda-rag/src/stream.rs:97-109`](../../crates/hacienda-rag/src/stream.rs#L97-L109)). `GuardedLlm` rédige seulement `prompt` et `chunk.content` ([`stream.rs:162-180`](../../crates/hacienda-rag/src/stream.rs#L162-L180)); `build_token_stream` envoie ensuite `system_content` tel quel au fournisseur ([`stream.rs:300-315`](../../crates/hacienda-rag/src/stream.rs#L300-L315)). Le champ `system_prompt` est accepté par l’API ([`hacienda-api/src/dto.rs:1045-1075`](../../hacienda-api/src/dto.rs#L1045-L1075)). Un appelant peut donc placer une donnée sensible dans ce champ et contourner la garantie de rédaction.
- `answer_stream` est public et documente qu’il ne rédige rien ([`stream.rs:189-206`](../../crates/hacienda-rag/src/stream.rs#L189-L206)). La garantie est donc celle du workflow HTTP courant, pas une impossibilité absolue pour un autre consommateur de bypasser le guard.
- Le test structurel est textuel et exclut `tests/`, `benches/` et `build.rs`; il documente aussi les contournements par import aliasé ([`llm_call_enforcement.rs:23-31`](../../crates/hacienda-rag/tests/llm_call_enforcement.rs#L23-L31)). C’est une défense CI, pas une propriété complète du type system.
- Le texte RAG n’est pas rédigé à l’upsert et peut être renvoyé brut par `retrieve`; l’invariant est donc « pas de texte brut vers le fournisseur via `answer` », non « corpus RAG toujours rédigé » ([`hacienda-api/src/handlers/rag.rs:218-280`](../../hacienda-api/src/handlers/rag.rs#L218-L280), [`rag.rs:377-399`](../../hacienda-api/src/handlers/rag.rs#L377-L399)).
- L’audit dédié « contenu envoyé à un LLM » prévu par la spécification P6 reste explicitement différé; seuls les audits de rédaction sont présents ([`superpowers/specs/2026-08-13-P6-llm-call-enforcement-point.md:166-183`](../../superpowers/specs/2026-08-13-P6-llm-call-enforcement-point.md#L166-L183)).

## Verdict

Le RAG est correctement filtré et le chemin `retrieve → answer` protège aujourd’hui `prompt` et `chunk.content` avant `chat_stream`. La sécurité effective est néanmoins **partielle**, et non une isolation multi-tenant/document complète : le transport du tenant dans l’auth réelle est cassé, le namespace de collection du tenant `default` est ambigu, les filtres ne constituent pas une ACL, et `system_prompt` est une fuite directe potentielle vers le LLM.

Vérification dynamique tentée : `cargo test -p hacienda-rag --features streaming` n’a pas pu démarrer, Cargo n’ayant pas accès à la révision `9bfbc10...` de `https://github.com/jamon8888/xberg.git` (dépôt introuvable). Les résultats ci-dessus sont donc fondés sur l’inspection statique et les tests présents, non sur une exécution verte dans cet environnement.

## Addendum d’état — 2026-11-15

Ce document reste un audit historique du commit `613e6de`. Depuis, le contexte tenant et la
validation des noms de collection ont été corrigés (`7ba6851`), puis les sorties de retrieval
ont été rédigées avant sérialisation (`9970b93`). Le constat concernant `system_prompt` reste
ouvert : `GuardedLlm` ne rédige toujours pas `LlmAnswerConfig.system_prompt` avant l’appel
provider. Voir aussi la spécification consolidée
[`2026-11-14-wayfinder-123-rag-hybrid-security-audit.md`](../superpowers/specs/2026-11-14-wayfinder-123-rag-hybrid-security-audit.md).

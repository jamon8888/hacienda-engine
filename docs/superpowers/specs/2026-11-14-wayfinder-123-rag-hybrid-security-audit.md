# Audit de sécurité effective du RAG hybride — Wayfinder #123

**Date :** 2026-11-14<br>
**Périmètre :** code source, tests et migrations/configuration du dépôt courant uniquement.<br>
**Méthode :** lecture directe des chemins `retrieve → answer`, des backends RAG, des filtres,
du cloisonnement et du garde d’appel LLM.<br>
**Convention suivie :** note d’investigation dans `docs/superpowers/specs/`, avec préfixe de
date et titre explicite, comme `2026-08-24-xberg-upstream-upgrade-investigation.md`.

## Conclusion exécutive

Le chemin nominal est correctement durci sur plusieurs points : les routes RAG sont protégées
par `documents:process`, les noms de collection sont préfixés par tenant avant les appels au
store, les filtres ont une whitelist et des plafonds de complexité, et la route `answer` force
le contenu des chunks puis passe par `GuardedLlm`, qui rédige le prompt et chaque contenu de
chunk avant construction de la requête LLM.

La sécurité effective n’est toutefois pas complète. Deux écarts sont prioritaires :

1. **Isolement tenant contournable pour le tenant `default` :** les routes adressées par nom ne
   rejettent pas un nom déjà préfixé (`acme:contracts`). Le tenant `default` peut donc adresser
   directement la collection physique d’un autre tenant, alors que le filtre de liste la cache.
2. **`system_prompt` non rédigé :** `GuardedLlm` ne rédige que `prompt` et
   `RetrievedChunk::content`; le `system_prompt` fourni par le client est transmis tel quel au
   LLM.

Un troisième écart structurel aggrave le premier : le contexte d’authentification produit par
les tokens utilise toujours le tenant par défaut. `ApiKey` possède bien `tenant_id`, mais
`Token::to_auth_context` appelle `AuthContext::new`, qui force `default`. Les tests multi-tenant
du RAG contournent volontairement cette résolution en injectant directement un `AuthContext`.

## Garanties observées

### Modes lexical, vectoriel et hybride

- `RetrieveMode` distingue `Vector`, `FullText`, `Hybrid`, `Sparse` et `LateInteraction`
  (`crates/hacienda-rag/src/query.rs:15-43`). La validation impose les entrées attendues :
  texte pour lexical/hybride, vecteur pour vectoriel selon le backend, et contrôle de dimension
  pour tout `query_vector` (`crates/hacienda-rag/src/query.rs:114-207`).
- Le backend mémoire annonce explicitement l’absence de lexical et d’hybride
  (`crates/hacienda-rag/src/backends/memory.rs:237-251`) et renvoie
  `UnsupportedMode` plutôt que de dégrader silencieusement (`crates/hacienda-rag/src/backends/memory.rs:398-407`).
- Le backend pgvector annonce `full_text` et `hybrid`, mais pas `sparse` ni
  `late_interaction` (`crates/hacienda-rag/src/backends/pgvector.rs:272-292`). Il vérifie la
  capacité du mode puis sélectionne explicitement les chemins vectoriel, full-text ou hybride
  (`crates/hacienda-rag/src/backends/pgvector.rs:527-552`).
- Le lexical s’appuie sur des colonnes `tsvector` générées pour le texte du chunk et le texte
  du document (`crates/hacienda-rag/migrations/0100_init.sql:34-49,63-93`). L’hybride exécute
  les deux ensembles de candidats puis les fusionne par reciprocal-rank fusion
  (`crates/hacienda-rag/src/backends/pgvector.rs:885-935`).
- La route HTTP charge d’abord la spécification de la collection, valide la requête, puis
  appelle `retrieve` sur le même nom scoped (`hacienda-api/src/handlers/rag.rs:357-399`).

**Lacune résiduelle :** la validation générique ne vérifie pas la capacité du backend
(`crates/hacienda-rag/src/query.rs:114-120`). Le refus arrive plus tard dans le backend. C’est
un comportement sûr (pas de fallback silencieux), mais l’API ne prévient pas avant l’exécution
qu’un store donné ne supporte pas lexical/hybride.

### Filtres

- Les champs sont limités aux namespaces `doc.*` et `chunk.*`, avec une whitelist explicite et
  des chemins contrôlés pour `metadata.*`/`chunk_metadata.*`
  (`crates/hacienda-rag/src/filter.rs:38-97`).
- La validation borne profondeur, nombre de nœuds, nombre de `text_match` et taille des
  requêtes textuelles (`crates/hacienda-rag/src/filter.rs:11-18,167-245`).
- Le backend mémoire applique le filtre avant le scoring et le backend pgvector l’ajoute aux
  requêtes vectorielle et full-text; le chemin hybride filtre séparément les deux listes avant
  fusion (`crates/hacienda-rag/src/backends/memory.rs:418-428`,
  `crates/hacienda-rag/src/backends/pgvector.rs:801-857,938-1005`).
- La traduction SQL n’interpole pas les valeurs; les noms de champs viennent d’un `match` sur
  la whitelist et les segments JSON sont bindés comme paramètres
  (`crates/hacienda-rag/src/backends/pgvector.rs:1248-1255,1257-1444`). Les tests couvrent
  notamment un champ façonné comme une injection SQL
  (`crates/hacienda-rag/src/filter.rs:274-285`).

### Scoping collection → document → chunk

- Le trait `RagStore` est explicitement single-tenant et ne porte aucun `tenant_id`; le
  cloisonnement est délégué à l’appelant (`crates/hacienda-rag/src/store.rs:10-17`).
- L’API préfixe les noms non-default par `<tenant>:` avant les opérations de création, lecture,
  suppression, ingestion, reindexation et retrieval (`hacienda-api/src/handlers/rag.rs:71-94,
  136-152,169-185,201-215,239-279,314-399`). Les opérations document/chunk du backend sont
  ensuite contraintes par la collection fournie; par exemple pgvector borne les suppressions,
  le fetch document et la mise à jour d’embeddings par `collection`
  (`crates/hacienda-rag/src/backends/pgvector.rs:487-509,682-745`).
- Le schéma durable rattache les documents à une collection et les chunks à la fois à un
  document et à une collection (`crates/hacienda-rag/migrations/0100_init.sql:34-45,63-70`).
- Les tests couvrent la non-collision de deux tenants et le filtrage de la liste de collections
  (`hacienda-api/src/handlers/rag.rs:991-1047,1049-1109`).

## Lacunes et chemins de contournement

### P0 — Le tenant `default` peut adresser une collection scoped d’un autre tenant

`scope_collection_name` laisse tout nom inchangé pour `default`
(`hacienda-api/src/handlers/rag.rs:88-93`). Le filtrage inverse refuse bien les noms contenant
`:` pour `list_collections` (`hacienda-api/src/handlers/rag.rs:96-119,434-480`), mais les routes
directes ne passent pas par ce filtre : elles font `get_collection`, `drop_collection`,
`list_documents`, `retrieve`, etc. avec le résultat de `scope_collection_name`
(`hacienda-api/src/handlers/rag.rs:169-185,201-215,505-525,377-399`).

Conséquence : si une collection physique `acme:contracts` existe, un appel du tenant `default`
avec `{name = "acme:contracts"}` cible cette collection. Le test existant prouve seulement que
la collection est absente de la **liste** default (`hacienda-api/src/handlers/rag.rs:954-964`),
pas que les routes directes la refusent. Le même défaut autorise potentiellement lecture,
ingestion, suppression ou retrieval selon la route.

### P0 — Les tokens ne transportent pas le tenant résolu jusqu’au handler

`Token::to_auth_context` construit le contexte avec `AuthContext::new`
(`hacienda-core/src/auth/authn.rs:76-84`), et `AuthContext::new` est défini pour utiliser le
tenant `default` (`hacienda-core/src/auth/mod.rs:169-177`). Pourtant `ApiKey` contient
`tenant_id` (`hacienda-core/src/auth/keys.rs:180-199`) et `ApiKeyTokenResolver` ne le transmet
pas lorsqu’il construit le `Token` (`hacienda-core/src/auth/authn.rs:262-277`).

Les tests RAG multi-tenant injectent donc directement `AuthContext::with_tenant` et documentent
que la résolution réelle des tokens tombe encore sur `default`
(`hacienda-api/src/handlers/rag.rs:974-989`). En production, deux clés de tenants différents
peuvent donc être vues par le chemin RAG comme le même tenant `default`, ce qui invalide la
garantie d’isolement attendue.

### P0 — Le `system_prompt` échappe à la redaction avant LLM

La route `answer` force `include_content = true`, récupère les chunks, puis passe le prompt et
les chunks à `GuardedLlm` (`hacienda-api/src/handlers/rag_stream.rs:125-161`). Le garde rédige
le prompt et chaque `RetrievedChunk::content` (`crates/hacienda-rag/src/stream.rs:150-185`),
mais son propre périmètre précise que `document`, `external_id` et `chunk_metadata` sont
forwardés inchangés et surtout ne mentionne pas `LlmAnswerConfig::system_prompt`
(`crates/hacienda-rag/src/stream.rs:162-167`).

Le DTO accepte un `system_prompt` fourni par le client et le handler le passe tel quel dans la
configuration (`hacienda-api/src/dto.rs:1057-1074`,
`hacienda-api/src/handlers/rag_stream.rs:147-161`). `build_token_stream` l’emploie directement
comme contenu du message système avant `chat_stream`
(`crates/hacienda-rag/src/stream.rs:280-327`). Toute donnée personnelle placée dans ce champ
franchit donc le garde sans redaction. La garantie actuelle ne couvre que le prompt utilisateur
et le contenu des chunks.

### P1 — Le contrat public permet encore de bypasser le garde

`answer_stream` est public et documente que ses appelants doivent rédiger eux-mêmes
(`crates/hacienda-rag/src/stream.rs:189-211`). `GuardedLlm` améliore le chemin interne, mais ne
rend pas impossible l’appel direct depuis un autre crate. Le test de workspace interdit les
créations directes du client LLM hors de `stream.rs`, ce qui est utile, mais reconnaît lui-même
qu’il ne couvre pas les imports aliasés et ne scanne pas `tests/`, `benches/` ni `build.rs`
(`crates/hacienda-rag/tests/llm_call_enforcement.rs:18-31,45-78,96-152`).

Dans le dépôt actuel, la route HTTP utilise bien `GuardedLlm`; la lacune concerne les
consommateurs externes ou futurs du crate et la différence entre une convention documentée et
une garantie compilée.

### P1 — La destination LLM est contrôlée par la requête

`AnswerRequest` accepte `xberg::LlmConfig` et le handler le transmet verbatim
(`hacienda-api/src/dto.rs:1057-1074`, `hacienda-api/src/handlers/rag_stream.rs:150-152`). Le
client LLM est ensuite créé avec `api_key`, `base_url`, timeouts, retries et modèle provenant de
cette configuration (`crates/hacienda-rag/src/stream.rs:280-297`). Aucun allowlist de fournisseur,
de `base_url` ou de politique d’egress n’apparaît dans ce chemin. La redaction réduit le risque
de PII, mais ne garantit pas que les données — même redigées — ne partent que vers un endpoint
approuvé.

### P1 — Pas de tenant_id en base ni de contrainte composite document/chunk

Le schéma durable encode le tenant uniquement dans le nom de collection : les tables
`rag_collections`, `rag_documents` et `rag_chunks` n’ont pas de colonne `tenant_id`
(`crates/hacienda-rag/migrations/0100_init.sql:19-27,34-49,63-93`). `rag_chunks.collection` est
une copie dénormalisée et possède une FK vers `rag_collections`, tandis que `document_id` a une
FK séparée; la migration ne définit pas de contrainte composite imposant que la collection du
chunk égale celle de son document (`crates/hacienda-rag/migrations/0100_init.sql:63-70`).

Le chemin applicatif écrit des valeurs cohérentes et filtre par collection, mais la sécurité
repose donc sur la discipline de l’API et l’intégrité des écritures, pas sur une isolation tenant
en base ou une RLS. Une écriture SQL directe, une migration incorrecte ou un futur backend mal
branché ne bénéficient pas d’un garde de tenant natif.

### P2 — Le retrieval peut renvoyer du contenu brut à un détenteur de `documents:process`

`RetrieveQuery` expose `include_content`, et le backend mémoire n’ajoute le contenu au résultat
que selon ce drapeau (`crates/hacienda-rag/src/backends/memory.rs:197-220`). La route `retrieve`
ne rédige pas sa réponse; la redaction est introduite uniquement dans le chemin `answer`
(`hacienda-api/src/handlers/rag.rs:377-399`, `hacienda-api/src/handlers/rag_stream.rs:142-161`).
Comme toutes les routes RAG, `retrieve` est seulement protégé par `documents:process`
(`hacienda-api/src/routes.rs:257-307`). Ce n’est pas une fuite vers le LLM, mais c’est une
frontière de confidentialité à documenter : un appelant autorisé peut demander les chunks
originaux sans passer par `GuardedLlm`.

## Chemin retrieve → answer : état vérifié

```text
route auth (documents:process)
  → tenant context
  → nom de collection scoped
  → get_collection
  → RetrieveQuery::validate
  → RagStore::retrieve
  → force include_content=true
  → RetrievedContext + prompt + system_prompt
  → GuardedLlm::stream
       → redact(prompt)
       → redact(each chunk.content)
       → answer_stream
            → context content + prompt/system_prompt
            → liter_llm::chat_stream
```

Les flèches jusqu’à `RagStore::retrieve` sont vérifiées dans
`hacienda-api/src/handlers/rag_stream.rs:120-140`; le garde et le passage à `answer_stream`
sont vérifiés dans `hacienda-api/src/handlers/rag_stream.rs:142-165` et
`crates/hacienda-rag/src/stream.rs:168-185`; la construction du message LLM est vérifiée dans
`crates/hacienda-rag/src/stream.rs:300-327`.

## Tests et niveau de confiance

Les tests présents couvrent :

- validation des modes, dimensions, filtres et limites (`crates/hacienda-rag/src/query.rs:224-360`
  et `crates/hacienda-rag/src/filter.rs:255-373`);
- filtres et refus lexical/hybride du backend mémoire
  (`crates/hacienda-rag/src/backends/memory.rs:673-731`);
- scoping de noms et non-collision de deux tenants dans les handlers
  (`hacienda-api/src/handlers/rag.rs:920-1109`);
- présence de la capacité `documents:process` dans la table de routes et test de refus
  (`hacienda-api/src/routes.rs:257-307,1917-1947`);
- redaction du prompt et de chaque chunk, arrêt avant construction du stream lorsqu’une
  redaction échoue (`crates/hacienda-rag/src/stream.rs:471-647`);
- absence de client LLM direct dans les autres sources Rust via un test de scan
  (`crates/hacienda-rag/tests/llm_call_enforcement.rs:45-152`).

Ces tests ne couvrent pas les deux scénarios critiques révélés par l’audit :

- accès direct du tenant `default` à `acme:<collection>` par les routes adressées par nom;
- `system_prompt` contenant une valeur contrôlée par la redaction mais non modifiée avant le
  LLM;
- résolution d’une vraie clé API vers son `tenant_id` dans `AuthContext`.

## Verdict Wayfinder #123

**Non conforme pour une revendication de sécurité effective multi-tenant et zéro-egress PII.**
Le cœur lexical/vectoriel/hybride et les filtres sont raisonnablement défensifs, et le chemin
HTTP nominal protège prompt + contenu des chunks avant l’appel LLM. Mais l’isolement tenant réel
est actuellement neutralisé par la résolution d’authentification et par le bypass du tenant
`default`; de plus, le `system_prompt` n’est pas couvert par la redaction et la destination LLM
reste librement configurable.

### Priorités de remédiation suggérées

1. Propager `ApiKey.tenant_id` jusqu’à `AuthContext::with_tenant`, puis ajouter un test route
   avec deux vraies clés/tenants.
2. Interdire les noms contenant le séparateur de scoping à l’entrée, ou rendre le scoping
   bijectif pour `default` et faire rejeter tout nom déjà scoped dans toutes les routes directes.
3. Rédiger `system_prompt` avant de construire `LlmAnswerConfig`, ou supprimer l’override libre
   et composer un prompt système côté serveur.
4. Décider et tester une politique d’egress LLM : endpoint allowlisté, configuration serveur,
   ou déclaration explicite que la fonctionnalité autorise un endpoint fourni par le client.

## Addendum — état au 2026-11-15

Cet audit décrit les failles constatées avant les commits de correction. Le tenant issu de
l’authentification est maintenant propagé au contexte et les noms de collections contenant
le séparateur réservé sont rejetés (`7ba6851`). Les surfaces textuelles renvoyées par le
retrieval sont rédigées avant sérialisation et échouent fermées si la pipeline est absente
ou échoue (`9970b93`).

La couverture n’est pas complète : `GuardedLlm` rédige le prompt utilisateur et le contenu
des chunks, mais `LlmAnswerConfig.system_prompt` est encore transmis tel quel à
`answer_stream` (`crates/hacienda-rag/src/stream.rs`). Il faut donc conserver la conclusion
« aucune garantie générale zéro-egress » jusqu’à rédaction/contrôle de ce champ, validation
des destinations LLM, et tests d’intégration multi-tenant exécutés. L’absence d’ACL au niveau
document/chunk demeure aussi distincte du cloisonnement de collections.

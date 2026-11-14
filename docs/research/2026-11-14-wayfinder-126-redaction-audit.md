# Wayfinder #126 — audit redaction, pseudonymisation, révélation et audit

Date : 2026-11-14<br>
Dépôt audité : `jamon8888/hacienda-engine`<br>
Méthode : lecture du code, des contrats rustdoc, des routes et des tests existants. Aucun code source n’a été modifié.

## Synthèse exécutive

Le chemin nominal Hacienda est correctement construit autour de `HaciendaFacade` :
`RedactionEngine` refuse la pseudonymisation sans clé, produit cinq modes de sortie, ne
met pas le span en clair dans l’audit, et la façade remplace le contenu avant de renvoyer
un résultat. L’API REST applique une table de routes deny-by-default, sépare
`documents:process`, `pii:reveal` et `audit:read`, et masque les détails internes dans les
erreurs. MCP et CLI passent par la même façade ; MCP n’active pas le serveur xberg brut.

Les garanties sont néanmoins conditionnelles et présentent quatre limites significatives :

1. La file de revue humaine persiste et expose `entity.text` en clair sous
   `audit:read`, sans `pii:reveal`. C’est un écart direct avec un invariant « le texte en
   clair ne sort jamais ».
2. Le coffre de clés n’est pas implémenté : `KeyResolver` est une couture injectable,
   mais le chemin fourni est `EnvKeyResolver` et des variables d’environnement. Le
   `--vault` CLI est un paquet de documents redigés, pas un coffre de clés séparé.
3. La rotation est additive mais opérateur-dépendante : l’ancienne clé doit rester
   provisionnée et déclarée comme `retired`; il n’existe pas de route `/v1/keys/rotate`.
   Le cache par tenant doit être reconstruit après rotation.
4. La réhydratation n’est pas une restauration byte-for-byte : la pseudonymisation
   révèle la forme normalisée, les champs ouverts `metadata.additional` et plusieurs
   fonctionnalités xberg non activées ne sont pas parcourus, et la profondeur récursive
   est plafonnée à 64.

## Flux et garanties observés

### RedactionEngine

`RedactionEngine::new` exige un `Arc<Pseudonymiser>` lorsque le mode est
`Pseudonymize`; il renvoie `MissingPseudonymKey` plutôt que de retomber silencieusement
sur `Mask` (`hacienda-core/src/redaction/engine.rs:28-51`). Le mode par défaut est
`Mask`, précisément parce qu’il est irréversible et ne dépend d’aucun secret
(`hacienda-core/src/redaction/types.rs:5-27`, `69-77`).

`redact` saute les spans invalides ou hors limites, remplace les spans valides, et
produit une entrée d’audit par span. L’entrée contient catégorie, action, source,
empreinte BLAKE3, longueur, confiance, horodatage et hash de chaîne, jamais le texte
original (`hacienda-core/src/redaction/engine.rs:54-121`; types :
`hacienda-core/src/redaction/types.rs:87-108`). Cela évite le crash, mais des spans
invalides sont ignorés : un détecteur défectueux peut donc réduire la couverture sans
faire échouer l’opération (`engine.rs:80-87`).

Les modes disponibles sont `Mask`, `Hash` (seulement les quatre premiers octets du
digest), `Pseudonymize`, `Remove` et `Custom` (`hacienda-core/src/redaction/engine.rs:124-155`).
Le template `Custom` peut inclure `{{TEXT}}`; il faut donc le traiter comme une sortie
potentiellement en clair si la configuration l’autorise.

### Pseudonymiser / révéler

Le token est `[CATEGORY:key_id:base32]`, chiffré en AES-256-SIV avec le label de catégorie
comme données associées. La construction est déterministe : égalité de valeur = égalité
de token, ce qui conserve la co-référence mais révèle l’égalité. Un bourrage par blocs de
16 octets masque la longueur exacte par bucket (`hacienda-core/src/redaction/pseudonym.rs:125-173`,
`516-531`). Le token reste donc une pseudonymisation, pas une anonymisation ; le porteur
de la clé peut déchiffrer hors ligne tout le corpus traité par cette clé
(`pseudonym.rs:527-531`).

La normalisation NFKC/minuscules/espaces et la normalisation des téléphones rendent
certains formats équivalents, mais la valeur révélée est la forme normalisée : casse et
espacement originaux sont irrécupérables (`pseudonym.rs:94-123`, `626-643`). C’est la
limite principale de « réhydratation » sémantique.

Les clés sont de 64 octets, protégées par `Zeroizing`, non clonables/sérialisables et
absentes de `Debug` (`pseudonym.rs:259-316`). `KeyResolver` prend explicitement un
`TenantId`; pour le tenant par défaut les noms historiques restent compatibles, tandis
que les autres tenants utilisent `HACIENDA_TENANT_<TENANT>_...` et échouent sans fallback
vers une autre clé (`pseudonym.rs:318-357`, `375-428`). Cela ferme la corrélation
inter-tenant, sous réserve d’une configuration de clés effectivement séparée.

`Pseudonymiser::new` charge activement la clé active et toutes les clés retirées dès
l’admission (`pseudonym.rs:542-599`). `reveal` ne peut utiliser que la clé active ou les
clés retirées déjà chargées; une mauvaise clé, un token altéré ou un label falsifié sont
regroupés sous `UnreadableToken` (`pseudonym.rs:626-700`). La révélation perd donc les
détails de diagnostic au niveau transport, ce qui est souhaitable contre l’oracle.

### HaciendaFacade et autorisation

La façade cache les pipelines et pseudonymisers par tenant, charge le tenant par défaut
au démarrage et construit les autres tenants à la première requête
(`hacienda-core/src/facade.rs:46-84`, `283-374`, `377-497`). Les appels authentifiés
portent un `TenantCtx`; les appels `Caller::Trusted` du CLI/MCP retombent explicitement
sur le tenant `default` (`hacienda-core/src/auth/mod.rs:213-262`). Il n’existe pas encore
de moyen de faire agir un appel in-process Trusted sur un tenant arbitraire
(`auth/mod.rs:246-254`).

`process_batch_with_auth` détecte, soumet éventuellement la revue, remplace
`document.content`, puis parcourt les champs structurés et écrit l’audit
(`hacienda-core/src/facade.rs:1014-1058`). `redact_text_with_auth` vide toujours le
texte des entités renvoyées (`facade.rs:1169-1223`). `scan_text_with_auth` n’inclut les
spans qu’avec `SpanText::Include`, qui exige `documents:process` et `pii:reveal` et
enregistre une entrée `Reveal` par span (`facade.rs:1081-1166`, `1355-1421`).

La révélation de token exige `pii:reveal`, déchiffre, puis écrit une entrée `Reveal`
avec `blake3(plaintext)`, catégorie `pseudonym`, principal et tenant
(`facade.rs:1225-1251`, `1316-1353`). Le plaintext revient donc au demandeur autorisé;
il n’est pas persisté par l’audit, mais l’empreinte permet de relier la révélation à la
redaction originale.

### Écart critique : revue humaine

Avant le remplacement de `document.content`, la façade appelle `submit_for_review`.
Pour chaque détection sous le seuil, cette méthode copie `entity.text` dans
`ReviewRequest::text_snippet` (`facade.rs:1039-1043`, `1980-2020`). Les routes de lecture
de revue exigent seulement `audit:read` (`facade.rs:542-555`; route table :
`hacienda-api/src/routes.rs:176-202`) et le DTO sérialise `text_snippet` en clair
(`hacienda-api/src/dto.rs:408-454`).

Conclusion : le chemin principal `documents`/`pii/redact` masque le texte, mais la revue
est une sortie claire persistée et exposée. Recommandation P0 : soit supprimer le
snippet, soit le pseudonymiser/redacter avant stockage, soit introduire une capacité
distincte de revue PII et auditer chaque lecture; `audit:read` seul ne doit pas suffire
si l’invariant global est strict.

### REST API

Les routes PII sont explicites : `/v1/pii/scan` et `/v1/pii/redact` sous
`documents:process`, `/v1/pii/reveal` sous `pii:reveal`, `/v1/pii/token` sous
`documents:process`, et `/v1/keys` sous `audit:read`
(`hacienda-api/src/routes.rs:107-131`, `379-385`). L’API refuse les champs inconnus et
n’accepte pour les documents que des octets base64 inline; aucun chemin URI/fichier n’est
désérialisé sur le réseau (`hacienda-api/src/dto.rs:1-12`, `20-62`). Cela réduit SSRF et
lecture de fichiers arbitraire.

Les handlers PII délèguent à la façade; `include_text` est faux par défaut, et les
réponses de scan/redaction/révélation ne renvoient que l’information contractuelle et le
tip d’audit (`hacienda-api/src/handlers/pii.rs:20-109`, `164-245`). Les erreurs internes
sont ramenées à des messages génériques sans document, span, nom de fichier ni chemin
(`hacienda-api/src/error.rs:1-13`, `168-285`).

L’audit REST est tenant-scopé et hash-chainé, mais le périmètre annoncé est
`this_node`, pas une vue globale de déploiement; les pages sont bornées à 1000, alors que
l’export complet est par nature non borné (`hacienda-api/src/handlers/audit.rs:19-24`,
`47-50`, `74-104`). Les champs d’audit exposent hash, longueur, principal et catégorie,
pas le plaintext (`hacienda-api/src/dto.rs:362-405`).

### Local / on-premise / CLI

Le CLI et MCP sont des appelants `Trusted`: la frontière de processus devient la
frontière de confiance (`hacienda-core/src/auth/mod.rs:213-262`; CLI :
`hacienda-cli/src/commands.rs:172-193`). `hacienda serve` est loopback par défaut et
refuse un bind réseau sans auth, ou avec le token placeholder public
(`hacienda-cli/src/commands.rs:1421-1479`). L’auth désactivée sur loopback donne toutefois
un accès Trusted complet, y compris la révélation.

Le CLI possède une dérogation explicite `--no-redact`, refusée sans
`--i-accept-unredacted-pii` (`hacienda-cli/src/cli.rs:555-561`; contrôle d’exécution :
`hacienda-cli/src/commands.rs:614-627`). Elle est donc intentionnelle et non silencieuse,
mais elle invalide toute promesse absolue si l’opérateur l’active.

Le `--vault` CLI écrit des documents Markdown redigés, un manifeste et un registre PII;
il peut inclure l’audit si `--audit-out` est utilisé, mais ce n’est pas un coffre de clés
(`hacienda-cli/src/cli.rs:516-523`, `hacienda-cli/src/commands.rs:776-783`). Le seul
résolveur livré est environnemental; `KeyResolver` permettrait un KMS/Vault/HSM, mais
aucun adaptateur concret de coffre séparé n’est présent dans le dépôt
(`hacienda-core/src/redaction/pseudonym.rs:318-357`, `431-486`).

### MCP et LLM

MCP expose huit outils et tous passent par `HaciendaFacade`; il n’active pas le serveur
xberg brut qui renvoie du contenu non redigé (`hacienda-mcp/src/lib.rs:7-20`, `33-47`).
Le transport implémenté est uniquement stdio, sans authentification par appel; tous les
outils utilisent `Caller::Trusted` (`mcp/src/lib.rs:24-31`, `71-103`). C’est adapté à un
client local lancé comme enfant, pas à un service MCP réseau multi-tenant.

`documents_process` accepte des chemins locaux/`file://`, contrairement à l’API HTTP
base64-only (`hacienda-mcp/src/params.rs:5-22`). `pii_scan` omet le texte; `pii_redact`
redacte; `pii_reveal` masque les erreurs de token et écrit une révélation
(`hacienda-mcp/src/lib.rs:140-217`). MCP ne propose pas actuellement `mint_token`,
`list_keys`, `audit_export` ni les opérations de revue, donc sa parité est partielle
malgré la parité revendiquée des outils listés.

Pour la voie RAG, `GuardedLlm` redige le prompt et le contenu de chaque chunk avant de
construire le flux LLM (`crates/hacienda-rag/src/stream.rs:111-185`). Il ne redige pas
les champs `chunk_metadata`, `external_id` ou `document`, car `answer_stream` n’utilise
actuellement que `content`; ce contrat est fragile si la composition du prompt évolue
(`stream.rs:162-167`). Le handler REST instancie le guard avant tout appel LLM
(`hacienda-api/src/handlers/rag_stream.rs:142-165`). Le fournisseur reste configurable
via `LlmConfig`: cette garantie protège l’egress contre le PII détectable, mais ne rend
pas automatiquement le fournisseur local/on-premise.

## Rotation et récupération

La rotation est conçue comme additive : le `key_id` est visible dans le token, les nouveaux
tokens utilisent la clé active, les anciens restent lisibles tant que la clé est chargée
(`pseudonym.rs:186-200`, `542-599`). `TenantPseudonymiserRegistry::admit` remplace le
cache pour réadmettre un tenant après rotation (`pseudonym.rs:728-779`), mais la façade
ne fournit pas de route de rotation et le CLI passe toujours une liste `retired` vide
(`hacienda-cli/src/commands.rs:43-61`). Une rotation sans provisioning durable des clés
retirées rend les tokens historiques irrécupérables; une rotation n’est pas un
re-chiffrement du corpus.

L’audit fichier est durable, partitionné par `(node, tenant)`, récupéré lazy au premier
accès, et refuse les opérations après une écriture empoisonnée jusqu’à réouverture
(`hacienda-core/src/audit/store_file.rs:250-299`, `338-404`, `407-435`). La récupération
des segments est donc bornée par tenant, mais `HaciendaFacade::close()` ne scelle que le
tenant du caller; en multi-tenant, il faut appeler `close_with_auth` par tenant
(`hacienda-core/src/facade.rs:821-872`).

## Limites de réhydratation documentaire

La façade redige en profondeur tables, pages, notes, contenu formaté, métadonnées
principales, annotations, formulaires, révisions, URI, mots-clés, résumés, captions,
QR payloads, OCR imbriqué et membres d’archives (`hacienda-core/src/facade.rs:1523-1566`,
`1645-1839`). Les enfants et OCR sont récursifs, avec plafond `MAX_REDACTION_DEPTH = 64`
(`facade.rs:34-44`, `1783-1824`). Au-delà, l’opération échoue plutôt que de laisser
passer une structure non redigée.

La couverture n’est pas totale : `metadata.additional` reste un `HashMap<String, JSON
Value>` non parcouru, et plusieurs champs/feature gates xberg (`elements`, structure
tree, `ocr_elements`, `chunks`, translation, etc.) sont explicitement hors périmètre
(`facade.rs:1556-1566`). La recommandation P1 est de remplacer cette liste ouverte par
une politique de parcours typée/allowlist testée, ou de refuser l’export lorsque des
valeurs JSON opaques sont présentes.

## Recommandations priorisées

- **P0 — revue claire** : ne plus persister `entity.text` en clair, ou isoler cette voie
  derrière une capacité PII dédiée, avec audit de lecture et masquage par défaut.
- **P0 — coffre** : fournir un `KeyResolver` KMS/Vault/HSM concret, rotation par tenant,
  rétention/escrow des anciennes clés et procédure de restauration testée; ne pas
  confondre le `--vault` documentaire avec ce coffre.
- **P1 — MCP multi-tenant** : si MCP sort du poste local, ajouter authentification,
  tenant explicite et capacités par appel; sinon documenter et verrouiller stdio/local-only.
- **P1 — réhydratation** : couvrir/refuser `metadata.additional` et les champs xberg
  activables; ajouter des tests de non-fuite par sérialisation REST/MCP/CLI.
- **P1 — GuardedLlm** : rediger aussi les métadonnées si elles peuvent rejoindre un
  prompt, et empêcher structurellement les appels directs à `answer_stream` hors guard.
- **P2 — preuves opérationnelles** : exposer/automatiser la rotation des segments et la
  fermeture de tous les tenants, plus des tests d’intégration avec un vrai store durable.

## Validation

Les tests statiques présents couvrent notamment la non-fuite MCP, le plafond de profondeur,
la chaîne d’audit et le guard LLM (`hacienda-mcp/src/lib.rs:477-545`,
`hacienda-core/src/facade.rs:3350-3383`, `crates/hacienda-rag/src/stream.rs:471-648`).
Les tests ciblés n’ont pas pu être exécutés dans cet environnement : Cargo tente de
récupérer `xberg` depuis `https://github.com/jamon8888/xberg.git` à la révision
`9bfbc10acca5c1648547a4d861153d2f5c363340`, qui répond `Repository not found`. Cette
limite ne change pas les constats de lecture mais empêche de confirmer un build/test vert.

## Addendum d’implémentation — 2026-11-15

La couverture des métadonnées `title`, `subject` et des valeurs récursives de
`metadata.additional` a été ajoutée à la façade, avec des assertions de non-fuite
(`6e6f9eb`). Cela répond à la lacune de métadonnées décrite ci-dessus pour les champs
effectivement parcourus par cette voie. Les champs Xberg conditionnels ou absents de la
configuration d’extraction restent hors preuve ; l’audit d’origine demeure le constat daté,
pas une attestation de couverture exhaustive actuelle.

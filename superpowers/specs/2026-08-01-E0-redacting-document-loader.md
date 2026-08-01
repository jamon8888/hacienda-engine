# E0 — *Document loader* rédacteur (LangChain, LlamaIndex)

**Date :** 2026-08-01
**Statut :** Proposé
**Piste :** E (parité) · **Vague :** 1 · **Chemin critique :** oui
**Programme :** `2026-08-01-hacienda-platform-parity-program.md` §6, décision D1
**Dépend de :** P1, P2 · **Valide le garde P1 pour :** E4

---

## 1. Problème et rôle

Deux besoins distincts se cachent derrière « RAG » :

- « je garde mon LangChain et mon pgvector, je veux que ce qui y entre soit rédigé et tracé » ;
- « je veux que hacienda **soit** mon moteur » (→ E4).

Le premier est massivement plus fréquent et infiniment moins cher à servir. L'amont l'a compris :
`xberg-io/xberg` publie huit intégrations first-party — LangChain, LlamaIndex readers et node
parser, CrewAI, txtai, SurrealDB, Spring AI, n8n — versionnées en verrou avec le cœur (analyse
§9.11.2).

**Mais la raison d'être de cette spec dans le programme est technique, pas commerciale.** E0 est
la **mise à l'épreuve du garde P1 contre un store tiers**, sur de vrais corpus, avant que E4 —
six à dix semaines — ne s'y appuie. Faire E4 d'abord ferait découvrir les défauts du garde dans
le composant le plus coûteux du programme.

## 2. Objectifs / Non-objectifs

**Objectifs**

- Un chargeur Python (PyPI) et un chargeur TypeScript (npm) appelant l'API hacienda.
- Documents et chunks rendus **déjà rédigés**.
- Chaque lot porte l'identifiant de chaîne d'audit qui l'atteste.

**Non-objectifs**

| Différé | Raison |
| --- | --- |
| Store vectoriel | → E4. Le client garde le sien : c'est le principe. |
| Embeddings côté hacienda | Le framework du client les calcule sur du texte déjà rédigé |
| CrewAI, txtai, Spring AI, n8n | Après LangChain et LlamaIndex, si la demande existe |
| Récupération | Le client interroge son propre store |

## 3. Forme

```text
HaciendaLoader(base_url, token, config)
  .load(paths | bytes | uris)  ->  [Document(page_content=<rédigé>, metadata={...})]

metadata porte :
  hacienda_audit_tip     tête de chaîne attestant ce lot
  hacienda_config_hash   configuration de rédaction appliquée
  hacienda_entity_count  nombre de spans rédigés
  hacienda_redaction_mode
```

**Décision D-E0-1 — le chargeur ne rédige pas lui-même.** Il appelle l'API. Une rédaction
réimplémentée en Python divergerait du moteur Rust au premier changement de motif, et deux
implémentations d'un contrôle de conformité est une de trop. Le chargeur est un client, pas un
second moteur.

**Décision D-E0-2 — `hacienda_audit_tip` est dans les métadonnées de chaque document.** C'est ce
qui distingue ce chargeur d'un simple client d'extraction : un utilisateur peut prendre un chunk
sorti de son vector store six mois plus tard, présenter son tip, et prouver via P2 sous quelle
configuration il a été rédigé. Sans cela, le chargeur ne vend rien que l'API ne vende déjà.

**Décision D-E0-3 — le chunking est fait par hacienda, pas par le framework.** Contre-intuitif,
et c'est le point de conception le plus important de la spec.

Si le framework découpe après coup, il découpe du texte rédigé — donc potentiellement au milieu
d'un jeton de pseudonymisation, produisant des chunks porteurs de fragments de jetons qui ne se
révèlent plus et ne co-réfèrent plus. Le découpage doit connaître les frontières de spans.
`xberg/chunking` est disponible au tag épinglé ; l'API expose le découpage, le chargeur le
consomme.

**Décision D-E0-4 — échec fermé sur lot partiel.** Si un document d'un lot échoue à la
rédaction, le lot entier échoue. Rendre les autres inviterait à ingérer un corpus incomplet en
croyant l'avoir traité. C'est la règle déjà appliquée par la façade.

## 4. Ce que le chargeur ne doit jamais faire

- **Mettre en cache du contenu non rédigé**, y compris en mémoire au-delà de l'appel.
- **Réessayer un lot ayant échoué à la rédaction** sans que l'appelant le demande : une
  réessai silencieuse masquerait une panne du détecteur.
- **Rendre `entities[].text`** : l'API le vide déjà côté core, le chargeur ne le reconstruit pas.

## 5. Versionnage et distribution

**Décision D-E0-5 — versionné avec l'API, pas avec le cœur.** Le chargeur parle à une API
versionnée `/v1`. Le modèle amont — verrou avec la version du cœur, via un script de
synchronisation — convient à des bindings natifs ; il ne convient pas à un client HTTP, qui doit
pouvoir corriger un bug sans release du moteur.

Publication : PyPI `langchain-hacienda`, npm `@hacienda/langchain`. Tests d'intégration contre un
serveur hacienda réel en CI, jamais contre un simulacre — un simulacre de l'API ne prouverait
rien du garde, qui est l'objet du test.

## 6. Tests — le test qui compte

| Test | Assertion |
| --- | --- |
| `control_corpus_is_unrecoverable_from_a_real_pgvector` | **Le test central.** Corpus témoin ingéré via le chargeur dans un pgvector réel, puis interrogé par similarité avec des requêtes ciblant les valeurs témoins. Aucune ne les rend. C'est la validation du garde P1 que E4 héritera. |
| `audit_tip_in_metadata_verifies_via_P2` | Bout en bout. |
| `chunk_boundaries_never_split_a_pseudonym_token` | D-E0-3. |
| `partial_batch_failure_fails_the_whole_batch` | D-E0-4. |
| `loader_never_caches_unredacted_content` | Inspection mémoire et disque après appel. |
| `entities_text_is_absent_from_loader_output` | Pas de reconstruction côté client. |

## 7. Critères de sortie

- Un corpus témoin ingéré via le chargeur dans un pgvector client n'est récupérable en clair par
  **aucune** requête de similarité.
- L'entrée d'audit correspondante est vérifiable via P2 depuis le tip porté par les métadonnées.
- Aucun jeton n'est coupé par une frontière de chunk.
- **Le garde P1 est déclaré éprouvé** — condition d'entrée de E4.

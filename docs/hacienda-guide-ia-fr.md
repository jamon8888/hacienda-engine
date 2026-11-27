# Guide de contexte Hacienda — français

Ce guide est un point d’entrée pour les agents qui travaillent sur Hacienda. Pour la liste complète des fonctionnalités, leurs limites, les intégrations réellement disponibles et les propositions d’amélioration, consulter le [catalogue produit](features-and-roadmap.md). Le catalogue et le code actuel priment sur les anciens plans et specs.

## Modèle produit

Hacienda protège les données sensibles au cours du traitement documentaire : extraction Xberg, détection PII, fusion de spans, redaction, audit et, selon le parcours, revue humaine, conformité ou indexation RAG.

Le pipeline PII fusionne les spans avant la redaction afin de préserver les offsets et la priorité des reconnaissances qui se chevauchent. La pseudonymisation est le seul mode réversible et dépend de clés AES-SIV correctement configurées. La révélation est une opération distincte soumise à la capacité `pii:reveal`.

## Composants principaux

| Composant | Responsabilité |
| --- | --- |
| `hacienda-core/` | Domaine, PII/redaction, auth, audit, revue, conformité, jobs, presets et versions |
| `hacienda/` | Façade Rust publique et exposition de Xberg |
| `hacienda-api/` | API HTTP Axum et contrat OpenAPI |
| `hacienda-cli/` | CLI `hacienda` |
| `hacienda-mcp/` | Outils MCP pour agents |
| `crates/hacienda-rag/` | Collections, chunking, récupération et backends RAG |
| `crates/hacienda-email/`, `crates/hacienda-legal/` | Traitement des e-mails et recherche juridique |
| `crates/hacienda-wasm/` | PII/redaction compilée en WASM pour le navigateur |
| `apps/hacienda-studio/` | Application navigateur, encore en développement |
| `apps/hacienda-secrecy-client/` | Application desktop Tauri |

Les manifestes Cargo, `package.json` et `Taskfile.yml` font foi pour la composition et les commandes exactes. La carte est décrite dans [`AGENTS.md`](../AGENTS.md).

## Frontières de sécurité à préserver

- Les entrées API ne permettent pas au client de faire lire un chemin local arbitraire ni une URL arbitraire par le serveur.
- Toute route protégée doit être enregistrée dans la table de routes centrale avec les capacités requises. L’OpenAPI est dérivé de ce contrat.
- Le bind HTTP non-loopback exige l’authentification.
- Les stores RAG ne redigent pas automatiquement les données à l’ingestion. Utiliser du contenu déjà redigé si les données brutes ne doivent pas être persistées.
- La route de réponse RAG redige le prompt et les extraits récupérés avant l’appel au modèle configuré.
- Les stores en mémoire et les jobs en mémoire ne survivent pas au redémarrage. Vérifier le backend de persistance réellement configuré.
- Les événements d’audit sont des preuves de traitement : préserver leur chaîne, leur vérification et leurs portées tenant/profil.
- Les modèles NER sont optionnels et peuvent nécessiter des assets installés séparément.

## Où chercher

- Fonctionnalité, limites produit et améliorations proposées : [`features-and-roadmap.md`](features-and-roadmap.md).
- Configuration, auth, bind et pseudonymisation : [`configuration.md`](configuration.md).
- Contrat API : [`hacienda-api/README.md`](../hacienda-api/README.md) et `GET /openapi.json`.
- Intégrations et limites constatées : [`integrations/README.md`](../integrations/README.md) et [`integrations/LIMITATIONS.md`](../integrations/LIMITATIONS.md).
- Architecture et documents d’analyse datés : [`architecture/README.md`](architecture/README.md).
- Harnais agentique et codemap : [`code-map-harness.md`](code-map-harness.md).

Avant de coder, vérifier l’implémentation du composant et sa configuration actuelle. Les anciens documents de plan peuvent décrire des routes, des adaptateurs ou des garanties qui n’existent pas encore.

# Parcours modèle réel (Mission P2) — GLiNER2 + Xberg embeddings

Ce document décrit comment rendre le parcours modèle réellement vérifiable,
sans WASM et sans mock sur le chemin production, et comment exécuter le test
réel quand les poids sont disponibles.

## Dépendances déclarées

Xberg est consommé depuis l'upstream officiel, déjà déclaré dans les
manifests (aucun ancien fork `jamon8888/xberg`):

- `Cargo.toml` (workspace): `xberg-io/xberg@507fca3d3cc5b8e71f0e416af4a8aec15e18d2cd`
- `hacienda-core/Cargo.toml`: même révision, features `ner` (wasm32) / `ner-candle` (native)
- `apps/hacienda-secrecy-client/src-tauri/Cargo.toml`: même révision, features
  `tokio-runtime`, `embeddings`, `chunking`, `ort-dynamic`, pdf/office/email/etc.

## État de la préparation (warm-up)

### Embeddings (Xberg)

`LocalFullDocumentService::install_embedding_model` appelle
`xberg::embeddings::warm_model(&model, cache_dir)` et ne passe
`embedding_ready` à `true` qu'après un `warm_model` réussi. L'ingestion et la
recherche hybride refusent (`"Activate the local multilingual retrieval model first"`
/ `"Install or activate the local multilingual retrieval model first"`) tant
que ce drapeau est faux — fail-closed, jamais de prêt-à-l'emploi avant un
vrai warm-up.

Commande Tauri : `install-local-rag-model`. Elle installe/réchauffe le modèle
d'embedding multilingue Xberg sous `<app-data>/models/xberg-embeddings`.
Aucun cache n'est requis à la compilation ; il est requis seulement pour
exécuter le vrai chemin embedding + hybrid search.

### Détecteur NER (GLiNER2, Candle)

`HaciendaLocalRedactor::from_key_store_with_runtime` valide les assets du
modèle (tailles + SHA-256 + architecture `config.json` / `encoder_config/config.json`
/ `tokenizer_config.json`) avant de construire le pipeline. `pii_detection_complete`
n'est mis à `true` qu'après validation réussie. `provider_send_enabled` exige
à la fois un détecteur complet et une passerelle configurée. Fail-closed.

## Exécuter le test réel (GLiNER2)

Le test existant
`pinned_gliner2_model_redacts_claude_codex_requests_and_responses` est
`#[ignore]`d avec une raison explicite :
*"loads the pinned GLiNER2 checkpoint; set HACIENDA_NER_MODEL_DIR"*.
Il n'est jamais exécuté en CI (aucun poids disponible) et n'est jamais mocké.

### Prérequis

Un dossier de checkpoint GLiNER2 installé localement, contenant au minimum :
`config.json`, `model.safetensors`, `tokenizer.json`,
`tokenizer_config.json`, `encoder_config/config.json`.

### Commande

```bash
HACIENDA_NER_MODEL_DIR=/chemin/vers/gliner2-pii \
  ./scripts/smoke-real-model.sh
```

Ce script :

1. Vérifie la présence des assets du checkpoint (fail clair si manquant).
2. Exécute le test `#[ignore]`d contre le vrai backend Candle GLiNER2.
3. Documente l'étape suivante : réchauffer le cache d'embedding Xberg via la
   commande Tauri `install-local-rag-model`.

Sans `HACIENDA_NER_MODEL_DIR`, le script sort avec code 0 après avoir affiché
la marche à suivre (les tests dépendant des poids restent ignorés).

## Tests qui exigent les poids

Tous les tests exigeant les poids sont `#[ignore]`d avec une raison
explicite et ne s'exécutent que quand `HACIENDA_NER_MODEL_DIR` est fourni.
Aucun mock n'est utilisé sur le chemin production.

## Vérification ciblée

```bash
cargo fmt --all -- --check
cargo clippy -p hacienda-core --all-targets -- -D warnings
cargo clippy -p hacienda-rag --features lancedb --all-targets -- -D warnings
cargo clippy -p hacienda-backend --all-targets -- -D warnings
cargo test -p hacienda-rag --features lancedb
cargo test -p hacienda-backend --all-targets
```

La crate Tauri (`apps/hacienda-secrecy-client`) est un workspace séparé ; sa
clippy vérifie avec le manifeste dédié. Sa compilation complète nécessite les
binaires préconstruits ONNX Runtime pour `ort-sys`.

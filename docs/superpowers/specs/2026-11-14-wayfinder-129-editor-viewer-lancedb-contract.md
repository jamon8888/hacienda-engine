# Éditeur Tiptap, persistance LanceDB et viewer multi-format — Wayfinder #129

**Date :** 2026-11-14<br>
**Statut :** exigence produit et contrat d’architecture avant implémentation<br>
**Périmètre :** client Tauri local/on-premise, Hacienda Engine, éditeur Tiptap,
LanceDB, coffre documentaire et viewer humain.

## Décision produit

L’utilisateur doit pouvoir :

1. sélectionner une portion de texte dans un document ;
2. la déclarer comme PII ou information sensible ;
3. ajouter manuellement une valeur sensible qui n’a pas été détectée ;
4. choisir entre pseudonymiser, masquer, supprimer ou marquer manuellement la portion ;
5. voir immédiatement le résultat tokenisé dans l’éditeur ;
6. sauvegarder cette modification comme une nouvelle version du dossier ;
7. interroger ensuite uniquement la version tokenisée via Claude/Codex/RAG ;
8. consulter l’original dans un viewer humain séparé, lorsque le format est supporté.

Une modification humaine est une décision de confidentialité, pas une simple correction
visuelle. Elle doit donc être versionnée, attribuée à l’utilisateur, auditée et soumise à la
même politique de non-egress que la redaction automatique.

## Deux profils d’installation

### Local Full

Tauri embarque Hacienda/Xberg, LanceDB Embedded et un coffre local chiffré. L’original et la
capacité de viewer restent sur le poste ; Claude/Codex passent par un gateway local qui ne
reçoit que la projection tokenisée.

### Team On-Premise

Tauri devient un client de session et de viewer. Hacienda Gateway/Engine exécute Xberg,
résout le tenant, applique les ACL et écrit l’audit. LanceDB est le data plane RAG token-only
derrière `RagStore`, tandis que Postgres/object store portent le control plane, les versions,
les jobs, l’audit et les originaux chiffrés. Aucun client ne se connecte directement à LanceDB.

Le contrat complet de ces profils, y compris la séparation entre pseudonymes Hacienda stables
et `xberg` `RehydrationMap` document-local, est détaillé dans
[`l’enquête Xberg/LanceDB/réhydratation #131`](../../research/2026-11-14-wayfinder-131-xberg-lancedb-rehydration.md).

## Modèle de contenu

Tiptap est utilisé comme modèle éditable de la **représentation tokenisée**. L’état canonique
de l’éditeur est le JSON Tiptap/ProseMirror, pas du HTML libre ni un texte Markdown aplati.
Tiptap documente `editor.getJSON()` comme format de persistance recommandé ; HTML et Markdown
sont des projections d’affichage/export.

Sources :

- [Tiptap — Concepts et JSON](https://tiptap.dev/docs/editor/core-concepts/introduction)
- [Tiptap — Persistence](https://tiptap.dev/docs/editor/core-concepts/persistence)
- [Tiptap — JSON, HTML et Markdown](https://tiptap.dev/docs/guides/output-json-html)

## Enveloppe d’ingestion et projection Tiptap

Xberg retourne un `ExtractedDocument` riche : `content`, `mime_type`, `metadata`, `tables`,
ainsi que selon le format `pages`, `chunks`, `images`, `elements`, `summary`, `uris`,
`form_fields`, `extracted_keywords` et d’autres champs optionnels. Lorsque la redaction Xberg
est activée, elle réécrit les surfaces textuelles autorisées et retourne un
`redaction_report` dont les offsets se rapportent au contenu original.

Hacienda ne doit pas exposer directement cet objet à Tiptap. L’API actuelle filtre déjà cette
sortie dans `DocumentResult` et `StructuredExtractionDto` ; le client doit ajouter une
projection d’éditeur stable :

```json
{
  "schema_version": "hacienda-editor-v1",
  "document_id": "doc_opaque",
  "version_id": "ver_opaque",
  "source": {
    "mime_type": "application/pdf",
    "original_ref": "orig_opaque",
    "viewer_ref": "viewer_opaque"
  },
  "metadata_token_only": {
    "title": "[TITLE:k1:opaque]",
    "filename": "[FILENAME:k1:opaque].pdf",
    "authors": ["[PERSON:k1:opaque]"],
    "subject": "[MATTER:k1:opaque]",
    "keywords": ["contrat", "[PROJECT:k1:opaque]"]
  },
  "editor": {
    "type": "doc",
    "content": []
  },
  "search_projection": {
    "text_token_only": "...",
    "chunk_ids": ["chunk_0001"],
    "embedding_state": "pending"
  },
  "provenance": {
    "xberg_version": "1.3.10",
    "ner_model_id": "fastino/GLiNER2-...",
    "ner_model_digest": "opaque-digest",
    "redaction_policy_version": "pii-policy-vN",
    "audit_event_id": "audit_opaque"
  }
}
```

Le champ `editor` est le seul état canonique de Tiptap. `metadata_token_only` alimente le
header et le panneau de métadonnées ; `search_projection` alimente le chunking et LanceDB ;
`original_ref`/`viewer_ref` ne sont jamais des chemins ou des données lisibles par l’agent.

### Exemple de JSON Tiptap affiché

```json
{
  "type": "doc",
  "content": [
    {
      "type": "haciendaDocumentHeader",
      "attrs": {
        "titleToken": "[TITLE:k1:t_01]",
        "filenameToken": "[FILENAME:k1:f_01].pdf",
        "mimeType": "application/pdf",
        "viewerRef": "viewer_opaque"
      }
    },
    {
      "type": "haciendaPage",
      "attrs": {"pageNumber": 1, "sourcePageRef": "page_opaque"},
      "content": [
        {
          "type": "heading",
          "attrs": {"level": 1},
          "content": [
            {
              "type": "text",
              "text": "[TITLE:k1:t_01]",
              "marks": [
                {
                  "type": "haciendaSensitive",
                  "attrs": {
                    "tokenClass": "title",
                    "tokenId": "t_01",
                    "mode": "pseudonymize",
                    "source": "xberg_metadata",
                    "disclosure": "viewer-only"
                  }
                }
              ]
            }
          ]
        },
        {
          "type": "paragraph",
          "content": [
            {"type": "text", "text": "Le client "},
            {
              "type": "text",
              "text": "[PERSON:k1:p_01]",
              "marks": [
                {
                  "type": "haciendaSensitive",
                  "attrs": {
                    "tokenClass": "person",
                    "tokenId": "p_01",
                    "mode": "pseudonymize",
                    "source": "gliner",
                    "disclosure": "viewer-only"
                  }
                }
              ]
            },
            {"type": "text", "text": " signe le contrat."}
          ]
        }
      ]
    }
  ]
}
```

La couleur, le soulignement et l’icône de cadenas sont une décoration Tiptap. La sécurité
repose sur le token, les attributs non secrets, le coffre, les ACL et le gateway.

### Correspondance des champs Xberg

| Xberg redacted | Projection Tiptap | RAG/LanceDB |
|---|---|---|
| `content`/Markdown | paragraphes, titres, listes, citations | `search_text_token_only` |
| `pages` | `haciendaPage` + page metadata | `page_number`, `page_ref` |
| `tables` | table/row/cell Tiptap | texte de cellule token-only |
| `metadata.title` | `haciendaDocumentHeader` + heading | `title_token_only` |
| auteurs/sujet/mots-clés | panneau metadata token-only | colonnes token-only/tagues |
| `images` | image avec `viewer_ref` opaque | référence, jamais octets originaux |
| `uris` | lien contrôlé avec label redacted | URL/label allowlistés |
| `redaction_report` | provenance/audit, non affiché intégralement | `audit_event_id`, pas les offsets originaux |

Les offsets du `redaction_report` Xberg sont relatifs au texte original. Ils ne doivent donc
pas être utilisés directement comme positions Tiptap après redaction. Hacienda doit construire
les marks à partir des tokens émis et conserver les offsets originaux uniquement dans l’audit.

### Mark Tiptap sensible

Les portions sensibles utilisent un mark ou node dédié, par exemple `haciendaSensitive`,
avec uniquement des attributs non secrets :

```json
{
  "type": "text",
  "text": "[PERSON:k1:ABC...]",
  "marks": [
    {
      "type": "haciendaSensitive",
      "attrs": {
        "tokenClass": "person",
        "tokenId": "opaque-local-id",
        "mode": "pseudonymize",
        "source": "manual",
        "disclosure": "viewer-only"
      }
    }
  ]
}
```

Le mark ne contient jamais la valeur brute. Une entrée créée sans valeur de réhydratation
utilise `mode: manual-mask` ou `mode: manual-sensitive` et est définitivement non
réhydratable.

### Actions utilisateur

- **Anonymiser une sélection détectée :** remplace la sélection par un token Hacienda,
  conserve la relation avec le coffre si la politique l’autorise, et crée un audit.
- **Ajouter une PII non détectée :** l’utilisateur fournit la sélection ou la valeur dans
  un contrôle local ; la valeur brute va directement au coffre, jamais au texte tokenisé ni
  au modèle.
- **Masquer/supprimer :** crée une transformation irréversible ou un token de masque sans
  entrée de coffre.
- **Modifier un token :** interdit de modifier directement le token stable ; l’utilisateur
  doit créer une nouvelle décision de redaction ou une nouvelle version.
- **Annuler :** crée une nouvelle version avec une décision inverse autorisée ; ne réécrit
  pas silencieusement l’historique d’audit.

## Persistance LanceDB

LanceDB est le candidat recommandé pour le magasin de recherche local et le data plane des
versions du **contenu tokenisé**, pas le coffre de PII brute. Cette recommandation ne vaut pas
encore décision de remplacer `PgVectorStore` pour le service d’équipe : l’adaptateur doit
rester derrière le contrat [`RagStore`](../../../crates/hacienda-rag/src/store.rs) et passer
les tests de parité, d’ACL et de non-egress décrits dans
[`l’enquête LanceDB #130`](../../research/2026-11-14-wayfinder-130-lancedb-full-rag.md).

L’état éditorial est enregistré en batch et upserté avec une clé composite :

```text
(tenant_id, dossier_id, document_id, version_id, chunk_id)
```

LanceDB documente `merge_insert` comme une opération transactionnelle d’upsert par clé,
capable d’insérer les lignes nouvelles et de mettre à jour les lignes existantes. Le client
doit utiliser cette opération plutôt qu’une boucle de mises à jour ligne par ligne.

Source : [LanceDB — merge_insert et opérations de table](https://lancedb.github.io/lancedb/python/python/).

### Colonnes minimales de la table tokenisée

```text
tenant_id                 string
dossier_id                string
document_id               string
version_id                string
parent_version_id         string nullable
chunk_id                  string
content_tiptap_json       json/string token-only
content_markdown          string token-only, projection
search_text               string token-only, normalisé
title_token_only          string token-only, projection
filename_token_only       string token-only, projection
authors_token_only        json/string token-only
subject_token_only        string token-only, projection
keywords_token_only       json/string token-only
attachment_names_token_only json/string token-only
structured_fields_token_only json/string token-only
embedding                 vector nullable
token_spans               json token-only
classification            json non-secret
source_hash               string
redaction_policy_version  string
editor_schema_version     string
created_by                string opaque/auditable
created_at                timestamp
audit_event_id            string
index_state               string
```

Le titre, le nom de fichier, les auteurs, le sujet, les mots-clés, les noms de pièces jointes,
les URLs/labels et les métadonnées PDF/Office sont des surfaces documentaires et doivent être
redacted par Hacienda/Xberg avant toute persistance ou indexation. Le texte des colonnes
`content_markdown` et `search_text`, les chunks, les embeddings et toutes les projections
`*_token_only` ne doivent dériver que de la représentation tokenisée. Une mise à jour humaine invalide les
chunks et embeddings de la version précédente, puis déclenche un re-chunking et un
re-embedding de la nouvelle version. L’ancienne version reste consultable uniquement selon
les droits d’historique du dossier.

LanceDB couvre la recherche dense, FTS/BM25, hybride, les filtres et le reranking, mais ne
remplace ni l’extraction/chunking Hacienda, ni l’ACL par dossier/chunk, ni l’audit, ni la
politique d’egress. Les modes `Sparse` et `LateInteraction` du contrat RAG restent
optionnels tant qu’un adaptateur LanceDB ne les démontre pas.

### Ce qui n’est pas dans la table de recherche

Le binaire original, le texte original et la table de correspondance token → valeur brute
ne doivent pas être placés dans les colonnes indexées, FTS ou vectorielles. Ils résident dans
un coffre de documents chiffré :

```text
LanceDB token-only row
  ├── original_ref opaque
  ├── source_hash
  └── viewer_capability_ref

Encrypted document store / local secure vault
  └── original bytes + restricted metadata + key reference
```

Si le déploiement exige physiquement LanceDB pour tous les artefacts, l’original chiffré peut
être stocké dans une table/blob séparée sans FTS, sans embedding, sans retour RAG et sans
accès direct depuis MCP. La PII en clair ne doit jamais partager la table de recherche.

## Synchronisation et versions

Chaque sauvegarde Tiptap crée une nouvelle `version_id` ou une révision append-only. Le client
doit :

1. valider le JSON Tiptap contre le schema d’éditeur ;
2. extraire la projection token-only ;
3. recalculer `source_hash` et un hash du contenu tokenisé ;
4. écrire l’événement d’audit ;
5. effectuer l’upsert LanceDB ;
6. marquer l’ancienne version comme superseded ;
7. planifier le re-chunking/re-embedding ;
8. rendre la nouvelle version interrogeable seulement après validation de l’index.

En mode local hors ligne, ces opérations sont placées dans une file transactionnelle locale,
avec reprise idempotente. En mode équipe/on-premise, le tenant, le dossier et les ACL sont
vérifiés côté serveur ; l’éditeur ne peut pas décider seul qu’une version est partageable.

## Viewer original et formats

Le viewer humain doit afficher l’original dans une fenêtre distincte du contexte agent. Il
doit être en lecture seule par défaut, afficher le tenant/dossier/version consultés et
interdire la copie automatique vers le prompt ou le presse-papiers.

### Formats prioritaires

| Format | Présentation prévue | Fallback |
|---|---|---|
| PDF | viewer PDF intégré Tauri, lecture seule, pages et recherche locale | Markdown tokenisé + ouverture originale autorisée |
| DOCX/ODT | aperçu document structuré ou conversion contrôlée en aperçu local | Markdown tokenisé + `Ouvrir l’original` |
| XLSX/ODS | grille en lecture seule, formules non exécutées | Markdown/tableau tokenisé + ouverture originale |
| PPTX/ODP | aperçu de diapositives rendu localement | Markdown tokenisé + ouverture originale |
| EML/MSG | viewer MIME local, pièces jointes isolées | Markdown tokenisé + ouverture originale |
| Images | viewer image local avec restrictions de copie | Markdown/métadonnées + ouverture originale |
| Autres formats | représentation Markdown tokenisée | lien original si une application autorisée existe |

Le Markdown fallback est une projection du contenu tokenisé, jamais une conversion de
l’original qui réintroduit la PII. Les pièces jointes non inspectables sont signalées et
ne sont pas envoyées au LLM.

### Ouvrir l’original sur le bureau

Le lien `Ouvrir l’original` n’est disponible que si :

- l’utilisateur possède une capacité viewer valide pour le document et la version ;
- l’OS dispose d’une application associée au format ;
- l’ouverture est confirmée si le format est sensible ou si l’application est externe ;
- l’action est auditée ;
- le chemin réel n’est pas exposé au modèle, à MCP ou à un lien Markdown distant.

Tauri fournit un plugin opener permettant d’ouvrir un chemin dans l’application par défaut,
mais les permissions et scopes de chemin doivent être explicitement limités. Sources :
[Tauri Opener](https://tauri.app/plugin/opener/) et [permissions du plugin Opener](https://tauri.app/reference/javascript/opener/).

## Invariants de confidentialité

- L’original reste hors du prompt, du transcript, des résultats MCP, de l’index RAG et des
  logs applicatifs.
- La sélection brute ne transite que vers le coffre local/serveur autorisé et le viewer
  humain.
- Les fournisseurs voient uniquement la version tokenisée validée par le gateway.
- Le viewer n’a pas de commande qui réinjecte automatiquement le texte révélé dans Tiptap,
  Claude ou Codex.
- Toute exportation d’une version tokenisée est distincte d’un export original et nécessite
  une capacité différente.
- Les couleurs et decorations Tiptap sont de la présentation ; la sécurité repose sur les
  marks token-only, les ACL, le coffre, le gateway et l’audit.

## Décisions encore à valider dans le ticket #129

- Les formats exacts du premier lot et les bibliothèques de rendu acceptées.
- La politique d’ouverture externe par OS et le niveau de confirmation requis.
- Le choix entre table/blob chiffré LanceDB et stockage objet local/on-premise pour les
  originaux.
- Le comportement de conflit pour deux éditeurs simultanés et l’usage éventuel de Yjs.
- La durée de conservation des versions et la politique de purge.

## Addendum — état livré au 2026-11-15

Le flux LocalFull de base est implémenté : sélection native, ingestion Xberg/Hacienda,
projection Tiptap token-only, persistance LanceDB et append de révisions avec validation
par pipeline (`6e6f9eb`). L’ouverture de l’original passe par une commande native avec
références opaques et contrôle côté processus.

### Évolution du pipeline embeddings — 2026-11-15

L’implémentation en cours active `hacienda-rag/embeddings` et le chunking Xberg dans le client
Tauri. Elle choisit le preset `multilingual` (E5, 768 dimensions), crée des chunks de 1 024
caractères avec chevauchement de 100, et passe les documents et les révisions de l’éditeur à
`Backend::persist_to_with_embedder` / `append_editor_revision_with_embedder`. L’activation
du modèle est explicite depuis l’interface ; aucun contenu documentaire n’est utilisé lors
du chargement/téléchargement du modèle. La collection est versionnée
`documents-multilingual-v1` pour isoler son schéma vectoriel.

La compilation Tauri Intel avec `ort-dynamic`, Clippy ciblé, le format Rust et le contrôle
TypeScript/bridge passent. Cette machine ne possède pas de bibliothèque ONNX Runtime exploitable
(le chemin Homebrew présent est vide), donc le chargement réel du modèle n’a pas encore été
vérifié. La distribution doit fournir une bibliothèque compatible et définir le chemin de
chargement pour chaque plateforme ; le bouton échoue fermé si le modèle ne peut pas démarrer.
Le chemin de requête RAG (embedding de la question, retrieval hybride dense + FTS filtré, puis
injection de chunks token-only dans Claude/Codex) et son expérience UI restent à implémenter.
La recherche dense/hybride n’est donc pas encore livrée de bout en bout.

Cette tranche ne livre pas encore les viewers intégrés PDF/DOCX/XLSX/PPTX du tableau, ni
la restauration durable des références de l’original après redémarrage. Le service garde
encore la liaison document/version/chemin dans l’état de session en mémoire. Les décisions
sur stockage durable chiffré, conflits concurrents, rétention, confirmation par OS et
formats de viewer restent ouvertes ; ne pas annoncer le contrat complet comme achevé.

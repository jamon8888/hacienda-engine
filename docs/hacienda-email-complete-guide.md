# Guide complet `hacienda-email` — fonctionnement actuel et perspectives

> Ce document décrit le plugin `hacienda-email` tel qu'il existe réellement dans ce dépôt
> (branche `fix/email-plugin-completion`), ainsi que les évolutions prévues mais non
> implémentées. Il est volontairement plus précis que le `crates/hacienda-email/README.md`
> historique : celui-ci contient un certain nombre d'affirmations « aspirantes » qui ne
> correspondent pas au code compilé (voir § 9, *Écarts avec le README*).
>
> Sources de vérité : `crates/hacienda-email/src/*`, `crates/hacienda-email/Cargo.toml`,
> `hacienda-api/src/handlers/email.rs`, `hacienda-cli/src/{cli.rs,commands.rs}` et
> `hacienda-mcp/src/{lib.rs,params.rs}`.

---

## 1. Vision et périmètre

`hacienda-email` est un plugin Rust qui traite des boîtes mail **sans faire sortir le
texte clair de la machine**, dans la même logique de secret professionnel que le reste
de hacienda. Il réutilise les composants existants plutôt que d'en réinventer :

- `hacienda-core` pour le moteur PII, l'audit et la pseudonymisation ;
- `hacienda-rag` pour le découpage en chunks (`chunk_full_text`) et l'indexation ;
- `xberg` pour l'extraction réelle des pièces jointes et du texte ;
- `io-email` 0.1 (Pimalaya) comme façade de protocole ;
- `mail-parser` pour décoder les octets RFC 5322 en modèle simplifié.

Ce qu'il fait aujourd'hui : lire une boîte IMAP ou Maildir, archiver, extraire les
pièces jointes, détecter des entités, **rediger avant toute indexation RAG**, et exposer
ces traitements via la CLI, l'API HTTP et le serveur MCP.

Ce qu'il ne fait pas encore : parler Gmail/Graph en direct, gérer JMAP/M2Dir/SMTP
(interface déclarée mais non branchée), rafraîchir les tokens OAuth ni les chiffrer au
repos. Voir § 8.

---

## 2. Architecture et les six phases

Le travail a été mené en six phases, visibles dans l'historique git :

| Phase | Apport principal | Fichiers clés |
|---|---|---|
| 1 | Crate `hacienda-email` : archives, NER, RAG, redaction, modèle de messages | `crates/hacienda-email/src/{archive,ner,rag,redact,models}.rs` |
| 2 | Vrai moteur PII (`hacienda-core`), design des pièces jointes, correction des tests | `src/pii.rs`, `src/attachments.rs`, `tests/integration_test.rs` |
| 3 / 3bis | Redaction RAG **fail-closed**, modes de confidentialité par locataire | `src/rag.rs`, `src/config.rs` (`PrivacyMode`) |
| 4 / 4bis | Extraction réelle via `xberg`, backend OAuth/XOAUTH2 Gmail & Outlook | `src/attachments.rs`, `src/backend.rs` (`build_oauth_client`) |
| 5 | Câblage CLI, HTTP et MCP | `hacienda-cli`, `hacienda-api`, `hacienda-mcp` |
| 6 | Remise en état des gates CI stricts et du formatage | `.github/workflows/*`, crate email |

Les décisions de conception sont consignées dans `crates/hacienda-email/DESIGN.md` :

- **D1** — deux modes au choix par locataire : `redact` (irréversible, par défaut,
  fail-closed) et `pseudonymize` (jetons réversibles sous clé, révélation auditée).
  Le choix est fait **à l'ingestion**, pas à la requête, pour qu'un texte clair ne
  puisse jamais être indexé sous le mauvais mode.
- **D2** — redaction obligatoire avant chunking/embedding. Si le moteur PII échoue,
  l'indexation est refusée.
- **D3** — uniquement de vraies API (`HaciendaFacade`, `chunk_full_text`,
  `ExtractInput::from_bytes()` / `extract()` de xberg).
- **D4** — OAuth/XOAUTH2 via `SaslXoauth2` de `pimalaya-stream 0.0.1`, sans client Graph.
- **D5** — la CLI est la surface principale ; HTTP et MCP construisent un `EmailProcessor`
  **par requête**, à partir de la configuration du locataire authentifié, jamais une
  instance globale de confiance.

---

## 3. Activation (features Cargo)

Déclarées dans `crates/hacienda-email/Cargo.toml` :

| Feature | Effet réel |
|---|---|
| `imap` | Active le backend IMAP et le backend OAuth (tous deux via `io-email/imap`). |
| `maildir` | Active `build_maildir_client` (`io-email/maildir`). |
| `jmap` | Active `io-email/jmap` — **le type est déclaré mais non branché** dans `build_client`. |
| `m2dir` | Active `io-email/m2dir` — idem, non branché. |
| `smtp` | Active `io-email/smtp` — idem, non branché (`send_message` renvoie `InvalidOperation`). |
| `gmail` / `msgraph` | Features déclarées pour faire round-tripper l'énumération de config ; **`build_client` les rejette explicitement**. |
| `rustls-ring` | Fournisseur TLS par défaut, via `pimalaya-stream/rustls-ring`. |
| `rustls-aws` / `native-tls` | Autres fournisseurs TLS (`pimalaya-stream/*`). |
| `ner` | Active `hacienda-core/ner-candle`. |
| `rag` | Active `hacienda-rag/chunking` et la dépendance `xberg`. |
| `attachments` | Active `dep:xberg` pour l'extraction de pièces jointes. |
| `redact` | Active le chemin de redaction. |
| `mock` | Feature de test (serveur IMAP factice prévu, non matérialisé dans le code actuel). |

**Features par défaut :** `imap`, `maildir`, `rustls-ring`.

```toml
# Exemple d'activation minimale
hacienda-email = { path = "../crates/hacienda-email", features = ["imap", "maildir", "ner", "rag", "redact"] }
```

---

## 4. Configuration complète (le cœur)

### 4.1 Structure `EmailConfig` (`src/config.rs`)

| Champ | Type | Défaut | Remarques |
|---|---|---|---|
| `backend` | `EmailBackend` | `Imap { server: "imaps://imap.example.com", username: "user@example.com" }` | Voir § 4.2. |
| `account` | `String` | `"default"` | Sert de clé pour les variables d'environnement `HACIENDA_EMAIL_<COMPTE>_*`. |
| `archive_path` | `Option<String>` | `None` | Chemin d'archive local (Maildir / stockage local). |
| `ner` | `NerConfig` | voir § 4.3 | Modèle et seuils NER. |
| `rag` | `RagConfig` | voir § 4.4 | Découpage et index. |
| `redaction` | `RedactionConfig` | voir § 4.5 | Règles et mode de confidentialité. |

Le chargement se fait avec `config::load_config(path)` (TOML → `EmailConfig`) ou
`config::load_config_with_secrets(path)`, qui surcharge certains champs sensibles depuis
l'environnement (§ 4.7). **Important :** `load_config` désérialise directement un
`EmailConfig` — le TOML est donc à la racine, sans préfixe `[email]` (§ 9).

### 4.2 Backends (`EmailBackend`, `#[serde(tag = "type")]`)

| `type` | Champs | Statut réel dans `build_client` |
|---|---|---|
| `imap` | `server`, `username` | ✅ implémenté (`build_imap_client`, SASL PLAIN) |
| `maildir` | `path` | ✅ implémenté (`build_maildir_client`) |
| `oauth` | `server`, `username`, `provider` | ✅ implémenté (`build_oauth_client`, SASL XOAUTH2) |
| `jmap` | `server`, `token` | ❌ déclaré, rejeté : `backend Jmap { .. } is not enabled or not implemented` |
| `gmail` | `client_id`, `client_secret` | ❌ idem |
| `msgraph` | `tenant_id`, `client_id` | ❌ idem |

`OAuthProvider` accepte `gmail` et `outlook`, avec les points d'accès implicites :

- `gmail` → `imaps://imap.gmail.com:993`
- `outlook` → `imaps://outlook.office365.com:993`

Un `server` explicite dans la configuration `oauth` l'emporte sur cette valeur par défaut.

### 4.3 `NerConfig`

| Champ | Défaut |
|---|---|
| `model` | `"gliner-multi-v2.1"` |
| `confidence_threshold` | `0.5` |
| `entity_types` | `PERSON`, `ORG`, `EMAIL`, `PHONE`, `LOC`, `DATE`, `MONEY` |
| `batch_size` | `32` |

### 4.4 `RagConfig`

| Champ | Défaut |
|---|---|
| `chunk_size` | `512` |
| `chunk_overlap` | `50` |
| `embedding_model` | `"all-MiniLM-L6-v2"` |
| `index_name` | `"email-corpus"` |

### 4.5 `RedactionConfig` et `PrivacyMode`

- `rules` : par défaut `["email", "phone", "ssn", "credit_card", "iban"]`.
  Types disponibles : `email`, `phone`, `ssn`, `credit_card`, `iban`, `ip`, et
  `custom { pattern, label }`.
- `replacement` : `"[REDACTED]"`.
- `preserve_format` : `true`.
- `mode` : `redact` (défaut) ou `pseudonymize`.

`PrivacyMode::Redact` masque définitivement les PII avant persistance/indexation.
`PrivacyMode::Pseudonymize` les remplace par des jetons réversibles par clé de locataire ;
la révélation est une opération distincte et auditée via `HaciendaFacade`.

### 4.6 Exemples TOML complets, par boîte

Les exemples suivants sont des fichiers `email.toml` passés à `hacienda email
--email-config <chemin>` (ou lus par `config::load_config[_with_secrets]`). Les mots de
passe et tokens **ne sont jamais écrits dans le fichier**.

#### a) IMAP avec mot de passe (Gmail, serveur IMAP + mot de passe d'application)

```toml
account = "work"

[backend]
type = "imap"
server = "imaps://imap.gmail.com:993"
username = "user@gmail.com"
# HACIENDA_EMAIL_WORK_PASSWORD est lu au moment de build_client()

[ner]
model = "gliner-multi-v2.1"
confidence_threshold = 0.5
entity_types = ["PERSON", "ORG", "EMAIL", "PHONE", "LOC", "DATE", "MONEY"]
batch_size = 32

[rag]
chunk_size = 512
chunk_overlap = 50
embedding_model = "all-MiniLM-L6-v2"
index_name = "email-corpus"

[redaction]
rules = ["email", "phone", "ssn", "credit_card", "iban"]
replacement = "[REDACTED]"
preserve_format = true
mode = "redact"
```

```bash
export HACIENDA_EMAIL_WORK_PASSWORD='mot-de-passe-application'
```

#### b) IMAP + XOAUTH2 (Gmail)

```toml
account = "work"

[backend]
type = "oauth"
server = "imaps://imap.gmail.com:993"   # vide => valeur par défaut du provider
username = "user@gmail.com"
provider = "gmail"

[ner]
model = "gliner-multi-v2.1"
confidence_threshold = 0.5
entity_types = ["PERSON", "ORG", "EMAIL", "PHONE", "LOC", "DATE", "MONEY"]
batch_size = 32

[rag]
chunk_size = 512
chunk_overlap = 50
embedding_model = "all-MiniLM-L6-v2"
index_name = "email-corpus"

[redaction]
rules = ["email", "phone", "ssn", "credit_card", "iban"]
replacement = "[REDACTED]"
preserve_format = true
mode = "redact"
```

```bash
export HACIENDA_EMAIL_WORK_ACCESS_TOKEN='ya29.…'   # token d'accès OAuth
```

Le token est transformé en `secrecy::SecretString` et injecté dans un
`pimalaya_stream::sasl::SaslXoauth2 { username, token }`. Il n'est **jamais** accepté
dans le corps d'une requête HTTP (§ 6.2).

#### c) IMAP + XOAUTH2 (Outlook / Microsoft 365)

```toml
account = "pro"

[backend]
type = "oauth"
server = "imaps://outlook.office365.com:993"
username = "prenom.nom@entreprise.fr"
provider = "outlook"

[redaction]
rules = ["email", "phone", "iban", "custom"]
replacement = "[REDACTED]"
preserve_format = true
mode = "pseudonymize"
```

```bash
export HACIENDA_EMAIL_PRO_ACCESS_TOKEN='eyJ0eXAi…'
```

> Le mode `pseudonymize` nécessite un résolveur de clé de pseudonymisation ; la
> configuration du résolveur lui-même n'est pas portée par `EmailConfig` (à confirmer /
> évolution future).

#### d) Maildir local (cas par défaut de la CLI)

```toml
account = "local"

[backend]
type = "maildir"
path = "~/.local/share/himalaya/mail"

[ner]
model = "gliner-multi-v2.1"
confidence_threshold = 0.5
entity_types = ["PERSON", "ORG", "EMAIL", "PHONE", "LOC", "DATE", "MONEY"]
batch_size = 32

[rag]
chunk_size = 512
chunk_overlap = 50
embedding_model = "all-MiniLM-L6-v2"
index_name = "email-corpus"

[redaction]
rules = ["email", "phone", "ssn", "credit_card", "iban"]
replacement = "[REDACTED]"
preserve_format = true
mode = "redact"
```

Sans `--email-config`, la CLI construit elle-même ce profil : `HACIENDA_EMAIL_MAILDIR`
(sinon `~/.local/share/himalaya/mail`, sinon `./email-archive`) et
`HACIENDA_EMAIL_ACCOUNT` (sinon `"default"`).

#### e) JMAP / Gmail REST / Microsoft Graph — configuration déclarée, non exécutable

Ces trois blocs sont acceptés par le désérialiseur (ils font round-tripper), mais
`build_client` renvoie une erreur explicite. Ils documentent la cible future, pas un
fonctionnement actuel :

```toml
# JMAP — NON IMPLÉMENTÉ DANS build_client
[backend]
type = "jmap"
server = "https://jmap.example.com/.well-known/jmap"
# token = ... (ou HACIENDA_EMAIL_<COMPTE>_TOKEN via load_config_with_secrets)

# Gmail REST — NON IMPLÉMENTÉ
[backend]
type = "gmail"
client_id = "xxx.apps.googleusercontent.com"
# client_secret = ... (ou HACIENDA_EMAIL_<COMPTE>_CLIENT_SECRET)

# Microsoft Graph — NON IMPLÉMENTÉ
[backend]
type = "msgraph"
tenant_id = "00000000-0000-0000-0000-000000000000"
# client_id = ... (ou HACIENDA_EMAIL_<COMPTE>_CLIENT_ID)
```

### 4.7 Secrets et variables d'environnement

| Variable | Utilisée pour |
|---|---|
| `HACIENDA_EMAIL_<COMPTE>_PASSWORD` | Mot de passe IMAP (SASL PLAIN). Obligatoire pour `type = "imap"`. |
| `HACIENDA_EMAIL_<COMPTE>_ACCESS_TOKEN` | Token OAuth/XOAUTH2. Obligatoire pour `type = "oauth"`. |
| `HACIENDA_EMAIL_<COMPTE>_TOKEN` | Surcharge du token JMAP (`load_config_with_secrets`). |
| `HACIENDA_EMAIL_<COMPTE>_CLIENT_SECRET` | Surcharge du secret client Gmail. |
| `HACIENDA_EMAIL_<COMPTE>_CLIENT_ID` | Surcharge du client ID Microsoft Graph. |
| `HACIENDA_EMAIL_MAILDIR` | Racine Maildir par défaut de la CLI. |
| `HACIENDA_EMAIL_ACCOUNT` | Nom de compte par défaut de la CLI. |

`<COMPTE>` est `config.account` en majuscules. Les secrets sont encapsulés dans
`secrecy::SecretString` dès la construction du client.

### 4.8 TLS

Tous les clients réseau utilisent `pimalaya_stream::tls::Tls::default()` et passent par
`EmailClientStd::connect_imap(&url, &tls, false, Some(sasl), None)`. Le fournisseur TLS
est choisi à la compilation par les features `rustls-ring` (défaut), `rustls-aws` ou
`native-tls`. Les URL IMAP attendues sont de la forme `imaps://hôte:993` ; une URL invalide
produit `EmailError::Config`.

---

## 5. Le pipeline de bout en bout

### 5.1 Modèle d'appel

`EmailProcessor::process(EmailRequest)` distribue vers une méthode selon
`EmailOperation` (`src/lib.rs`) :

| `EmailOperation` | Méthode interne |
|---|---|
| `Archive { … }` | `archive()` |
| `ExtractNER { … }` | `extract_ner()` |
| `IndexForRag { … }` | `index_for_rag()` |
| `Redact { … }` | `redact()` |
| `Pipeline { … }` | `pipeline()` |

Les sources (`EmailSource`) et destinations (`EmailDestination`) couvrent Maildir,
fichiers et stdin ; l'entrée `Stdin` et `Files` sont explicitement refusées pour
l'indexation RAG (`InvalidOperation`).

### 5.2 Étapes du pipeline

`PipelineStep` (utilisé par l'API HTTP) énumère, dans l'ordre :

1. `Archive` — lecture et archivage via l'adaptateur `EmailClient` ;
2. `ExtractAttachments` — extraction réelle avec xberg (`attachments.rs`) ;
3. `Ner` — détection d'entités via le moteur PII de `hacienda-core` ;
4. `RagIndex` — découpage avec `hacienda-rag::chunk_full_text` puis indexation ;
5. `Redact` — redaction du corps **et des en-têtes** (`redact.rs`).

### 5.3 Le garde-fou fail-closed (D2)

`src/rag.rs` applique la redaction **avant** le chunking et l'embedding, pour chaque
message. Si `redact()` renvoie une erreur, le message est ajouté à la liste `errors` avec
le libellé :

```text
Redaction failed for <id> — refusing to index (fail-closed): <erreur>
```

et **aucun chunk de ce message n'est indexé**. Le résultat porte `success =
errors.is_empty()`. C'est la fermeture du gap E4 identifié dans le workspace : il n'existait
auparavant aucun garde-fou de redaction à l'ingestion du chemin d'embedding.

### 5.4 Adaptateur de backend

`io-email` 0.1 expose un `EmailClientStd` bloquant, `!Sync` (le flux IMAP est `Send` mais
pas `Sync`). Le plugin l'enveloppe dans `BlockingClient { inner: Arc<Mutex<EmailClientStd>> }`
et exécute chaque appel dans `tokio::task::spawn_blocking`, ce qui donne un trait
`EmailClient: Send + Sync` utilisable derrière un `Arc` sans bloquer l'exécuteur async.
Les octets RFC 5322 renvoyés par `get_message` sont décodés avec `mail-parser` vers
`EmailMessage` (subject, from/to/cc, date UTC, corps texte/HTML, pièces jointes).

Le trait `EmailClient` expose : `list_mailboxes`, `list_envelopes`, `search_envelopes`,
`get_message`, `add_message`, `copy_messages`, `move_messages`, `delete_messages`,
`store_flags` et `send_message`. Sur les backends locaux/IMAP, `send_message` renvoie
`InvalidOperation` (« sending is only available on the SMTP/JMAP backends »).

### 5.5 Erreurs

`EmailError` (`src/error.rs`) : `Config`, `Backend`, `Connection`, `Auth`,
`MailboxNotFound`, `MessageNotFound`, `Io`, `Serde`, `UrlParse`, `Ner`, `Rag`,
`Redaction`, `Attachment`, `Plugin`, `InvalidOperation`.

---

## 6. Surfaces d'appel réelles

### 6.1 CLI — `hacienda email`

Cinq sous-commandes (`hacienda-cli/src/cli.rs`) :

```bash
hacienda email archive   --source <maildir> --output <dir> [--since "30 days ago"] [--email-config <toml>]
hacienda email ner       --source <maildir> --output <entities.json> [--email-config <toml>]
hacienda email redact    --source <maildir> --output <dir> [--mode redact|pseudonymize] [--email-config <toml>]
hacienda email index     --source <maildir> [--index <name>] [--email-config <toml>]
hacienda email pipeline  --source <maildir> --output <dir> [--email-config <toml>]
```

Le mode par défaut est `redact` ; `--mode pseudonymize` bascule
`config.redaction.mode` avant la construction du processeur. La CLI est un processus
local et est donc traitée comme `Caller::Trusted` (D5).

### 6.2 HTTP — `POST /v1/email/process`

Une seule route est enregistrée dans `hacienda-api/src/routes.rs` (et dans le document
OpenAPI via `crate::handlers::email::process_email`). Elle est protégée par la capacité
`DocumentsProcess`.

Corps (`EmailProcessRequest`) :

```json
{
  "account": "work",
  "backend": { "type": "oauth", "server": "", "username": "user@gmail.com", "provider": "gmail" },
  "privacy_mode": { "mode": "redact" },
  "operation": { "operation": "pipeline", "source": "…", "output": "…" }
}
```

Formes de `backend` acceptées par le DTO : `maildir { path }` et
`oauth { server?, username, provider }`. Les identifiants ne sont **jamais** dans le
corps : le serveur les résout depuis `HACIENDA_EMAIL_<ACCOUNT>_*`. Un processeur neuf est
construit pour chaque requête (D5), ce qui empêche qu'un locataire lise la boîte d'un
autre ou déclenche une redaction sous le mauvais mode.

Opérations disponibles dans le DTO : `archive`, `extract_ner`, `redact`, `index_for_rag`,
`pipeline`.

### 6.3 MCP — outil `email_redact`

`hacienda-mcp/src/lib.rs` n'expose qu'un seul outil email, `email_redact`, décrit dans
`hacienda-mcp/src/params.rs` par `EmailRedactParams { source: String, output: String }`.
Il construit un `EmailConfig` Maildir à partir du chemin fourni, exécute
`EmailOperation::Redact` et renvoie le nombre de messages traités. Comme pour HTTP,
aucun `EmailProcessor` n'est partagé entre appels.

L'inventaire des outils est vérifié par le test anti-dérive
`registers_exactly_the_documented_tools`, qui inclut `"email_redact"` dans la liste
attendue (14 outils au total sur cette branche).

### 6.4 Configuration du serveur MCP dans Claude Desktop

Le serveur MCP de hacienda utilise **exclusivement le transport stdio** (M1a) : il ne
monte pas de port HTTP/SSE. C'est le client (Claude Desktop) qui le lance comme processus
enfant et qui lui parle via stdin/stdout. La commande est :

```bash
hacienda mcp serve
```

Elle accepte les options globales de la CLI :

- `--config <PATH>` : chemin vers un `hacienda.toml` (sinon découverte automatique) ;
- `--config-json <JSON>` : configuration inline, prioritaire sur `--config`.

> **Frontière de confiance.** Le serveur appelle la facade avec `Caller::Trusted` : un
> agent n'est pas un réviseur humain. C'est pourquoi l'inventaire MCP exclut
> volontairement l'outil de révélation `pii_reveal` — la révélation d'un jeton reste dans
> le viewer Tauri authentifié et audité. Le serveur tourne jusqu'à la déconnexion du
> client ; il n'existe pas encore d'arrêt propre du facade store (le processus s'appuie
> sur le teardown du système d'exploitation).

#### Fichier de configuration Claude Desktop

Éditez `claude_desktop_config.json` :

| Système | Emplacement |
|---|---|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

Exemple minimal :

```json
{
  "mcpServers": {
    "hacienda": {
      "command": "/Users/moi/.cargo/bin/hacienda",
      "args": ["mcp", "serve", "--config", "/Users/moi/.config/hacienda/hacienda.toml"]
    }
  }
}
```

Avec des secrets d'environnement (utile si la configuration du facade en a besoin ;
l'outil email n'en réclame pas, voir ci-dessous) :

```json
{
  "mcpServers": {
    "hacienda": {
      "command": "/Users/moi/.cargo/bin/hacienda",
      "args": ["mcp", "serve", "--config", "/Users/moi/.config/hacienda/hacienda.toml"],
      "env": {
        "HACIENDA_EMAIL_WORK_ACCESS_TOKEN": "ya29.…",
        "HACIENDA_EMAIL_WORK_PASSWORD": "…"
      }
    }
  }
}
```

Points importants :

- utilisez un **chemin absolu** vers le binaire `hacienda` : Claude Desktop ne compte
  pas sur le `PATH` du shell interactif ;
- `mcp` puis `serve` sont deux niveaux de sous-commandes distincts (`hacienda mcp serve`) ;
- `--config` est un argument **global** de la CLI : il peut aussi être placé avant
  `mcp serve` ;
- après modification du fichier, **redémarrez Claude Desktop** pour que le serveur soit
  relancé ; les 14 outils, dont `email_redact`, doivent alors apparaître dans la liste des
  outils.

#### Appel de l'outil depuis Claude Desktop

`email_redact` ne prend que deux paramètres, définis dans
`hacienda-mcp/src/params.rs` (`EmailRedactParams`) :

```json
{ "source": "/Users/moi/Mail/INBOX", "output": "/Users/moi/Mail/redacted" }
```

Une formulation naturelle suffit, par exemple : *« Redact the PII in the Maildir at
`/Users/moi/Mail/INBOX` and write the result to `/Users/moi/Mail/redacted` »*. Le serveur
construit alors un `EmailConfig` avec `EmailBackend::Maildir { path: params.source }`,
le compte `"mcp"` et les valeurs par défaut (mode `redact`, règles standards), puis
exécute `EmailOperation::Redact` sur les corps **et les en-têtes**.

**Limite à connaître :** l'outil MCP email est **câblé en dur sur Maildir local**. Il ne
lit ni le fichier `email.toml` décrit au § 4, ni les variables `HACIENDA_EMAIL_*`, et ne
sait donc pas se connecter à IMAP ni utiliser XOAUTH2. Pour une boîte distante (Gmail,
Outlook), passez par la CLI `hacienda email --email-config …` ou par
`POST /v1/email/process` (§ 6.2) ; un futur outil MCP « process » générique comblerait
ce gap.

#### Dépannage

- Le serveur ne démarre pas : lancez `hacienda mcp serve` dans un terminal — il doit
  rester en attente sur stdin sans afficher d'erreur ;
- `email_redact` renvoie `email processor: …` : vérifiez que `source` est un **chemin
  Maildir valide** et que le répertoire `output` est accessible en écriture ;
- aucun outil n'apparaît dans Claude Desktop : vérifiez le chemin absolu du binaire, la
  syntaxe JSON du fichier et le redémarrage de l'application ;
- les logs du serveur vont sur stderr ; avec stdio, **n'écrivez rien sur stdout** (le
  protocole JSON-RPC l'utilise).

---

## 7. Vérification et tests

Commandes de référence (target privé pour rester sous la limite de RAM) :

```bash
CARGO_TARGET_DIR=target-email cargo check -j 2 -p hacienda-email
CARGO_TARGET_DIR=target-email cargo check -j 2 -p hacienda-mcp -p hacienda-api
CARGO_TARGET_DIR=target-email cargo test  -j 2 -p hacienda-email
cargo fmt --all -- --check
typos . --config _typos.toml
```

La suite `crates/hacienda-email/tests/integration_test.rs` contient **12 tests**, tous
vertus : construction du processeur Maildir, structure des opérations archive/NER/redact/
pipeline, valeurs par défaut et sérialisation de la config, parsing de dates (`7 days ago`,
ISO, date simple), conversion HTML→texte, valeurs par défaut OAuth, défaut du mode de
confidentialité (`redact`) et formatage des adresses. Aucun test n'a besoin d'un serveur
IMAP externe.

Le répertoire de build `target-email/` est ajouté à `.gitignore` : il ne doit jamais
apparaître dans `git status`.

---

## 8. Limites connues et feuille de route (« future »)

1. **Gmail / Microsoft Graph REST** — les features `gmail` et `msgraph` existent dans
   `Cargo.toml` et les variantes `EmailBackend::Gmail` / `Msgraph` dans `config.rs`, mais
   `build_client` les rejette avec `backend … is not enabled or not implemented`. La voie
   retenue (D4) est IMAP + XOAUTH2, déjà fonctionnelle ; un client Graph dédié reste à
   écrire si l'on veut les dossiers et les labels non-IMAP.
2. **JMAP, M2Dir, SMTP** — même situation : types de config et features déclarés, non
   branchés. `send_message` est donc inopérant sur les backends actuellement supportés.
3. **Cycle de vie des tokens OAuth** — `DESIGN.md` mentionne le rafraîchissement des
   tokens et leur stockage chiffré comme faisant partie du travail ; le code actuel lit
   `HACIENDA_EMAIL_<COMPTE>_ACCESS_TOKEN` mais n'implémente ni refresh ni stockage chiffré
   (à confirmer / évolution future).
4. **Résolveur de clé de pseudonymisation** — le mode `pseudonymize` est porté par
   `PrivacyMode`, mais la configuration du résolveur de clé par locataire n'est pas dans
   `EmailConfig` (à confirmer / évolution future).
5. **Dépendance `io-email` 0.1** — elle tire `io-m2dir 0.1.0`, désormais *yanké* sur
   crates.io. Le `Cargo.lock` doit conserver cette entrée épinglée pour que la résolution
   fonctionne. Une montée de version de la pile Pimalaya est nécessaire avant de pouvoir
   régénérer le lock « à blanc ».
6. **Surface HTTP réduite** — seul `POST /v1/email/process` existe ; les endpoints
   granulaires du README ne sont pas enregistrés.
7. **`mock`** — feature déclarée pour un serveur IMAP de test qui n'est pas matérialisé
   dans les 12 tests actuels.

---

## 9. Écarts avec le `README.md` historique

Le README de la crate décrit une cible plus large que le code compilé. En particulier :

- il montre un fichier `config.toml` avec un préfixe `[email]` ; or `load_config`
  désérialise directement `EmailConfig` : les sections doivent être à la racine
  (`[backend]`, `[ner]`, `[rag]`, `[redaction]`) avec `type = "imap"` dans `[backend]` ;
- il annonce cinq endpoints HTTP (`/v1/email/archive`, `/ner`, `/index`, `/redact`,
  `/pipeline`) et quatre outils MCP (`email_archive`, `email_ner`, `email_rag_index`,
  `email_redact`) : **seuls** `POST /v1/email/process` et `email_redact` existent ;
- sa matrice de support marque JMAP/Gmail/Graph/M2Dir/SMTP comme opérationnels : ils sont
  déclarés mais rejetés par `build_client` ;
- sa table de features cite des dépendances directes `io-imap`, `io-gmail`, `io-msgraph`
  qui ne sont pas celles du `Cargo.toml` actuel (tout passe par `io-email` 0.1 et
  `xberg` est la dépendance d'extraction).

Ce guide fait foi pour l'état présent du code ; le README doit être aligné lors d'une
prochaine passe de documentation.

---

*Document établi à partir du code de la branche `fix/email-plugin-completion` (PR #166),
rebasée sur `main`. Les items marqués « à confirmer / évolution future » ne sont pas
implémentés et ne doivent pas être considérés comme disponibles en production.*

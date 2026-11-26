# Installer le harnais d'agent VPS (pour un autre utilisateur)

Ce harnais est **user-level** : il s'installe dans le `HOME` de la personne qui
exécute l'installateur, sans sudo, et ne touche jamais les comptes des autres
utilisateurs. Chaque utilisateur reproduit donc l'installation ci-dessous avec
**son propre compte**. L'état d'installation peut différer selon le profil ;
utilisez `doctor` pour vérifier l'environnement courant.

## 1. Principe

```
<HOME>/.local/share/agent-harness/        # paquet versionné (données de l'utilisateur)
  versions/0.2.0/agent_harness/           # code Python
  versions/0.2.0/bin/agent-harness        # lanceur copié
  versions/0.2.0/bin/gatekeeper           # verify_changes() gate
  versions/0.2.0/MANIFEST.json            # empreintes SHA-256
  current -> versions/0.2.0               # pointeur vers la version active
<HOME>/.local/bin/agent-harness -> .../current/bin/agent-harness   # commande dans PATH
```

L'installation est **idempotente** : relancer la commande ne réécrit rien si le
contenu est identique ; une nouvelle version est installée à côté et `current`
est repointé. Aucun fichier système (`/usr/local`, `/etc`) n'est modifié.

## 2. Prérequis

À vérifier avec le compte de l'utilisateur :

```sh
basemind --version     # doit afficher 0.34.0 (version épinglée)
orca --version         # doit être >= 1.4.263
python3 --version      # >= 3.11 (nécessaire pour tomllib du stdlib)
```

Les six CLI d'agent sont **optionnels** pour le `doctor` : `claude`,
`claude-teams` (même binaire que `claude`, avec
`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`), `codex`, `opencode`, `opencode2`,
`pi`. Une CLI absente est rapportée `not_installed` et ne fait pas échouer le
`doctor`. Vérifier aussi que `~/.local/bin` est dans le `PATH` :

```sh
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) export PATH="$HOME/.local/bin:$PATH"
     echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc ;;
esac
```

## 3. Installation

Depuis un checkout contenant le dossier `vps-harness/` (le worktree de
`jamin` peut servir de source, mais chacun installe dans **son** `HOME`) :

```sh
cd /chemin/vers/le/checkout
./vps-harness/bin/agent-harness install
```

Résultat attendu (JSON si `--json`) :

```sh
./vps-harness/bin/agent-harness install --json
# status: "installed" la 1re fois, "already_installed" ensuite
```

Pour tester sans toucher au vrai `HOME`, surcharger les chemins :

```sh
export AGENT_HARNESS_DATA_HOME=/tmp/mon-data
export AGENT_HARNESS_BIN_HOME=/tmp/mon-bin
./vps-harness/bin/agent-harness install --json
```

Variables reconnues : `AGENT_HARNESS_DATA_HOME`, `AGENT_HARNESS_BIN_HOME`,
`AGENT_HARNESS_SOURCE_ROOT` (source à copier) et `AGENT_HARNESS_SELFTEST_PROVER`.

## 4. Vérification de la machine

```sh
agent-harness doctor --json
```

Points de contrôle :

| Élément | Attendu |
|---|---|
| `basemind` | `0.34.0` (toute autre version → `version_mismatch` / `degraded`) |
| `orca` | `>= 1.4.263` |
| `python` | `>= 3.11` |
| CLI absente | `not_installed`, **sans** faire échouer le doctor |
| statut global | `ok` ou `degraded` (code 0), `fail` seulement si outil requis manquant (code 1) |

Le `doctor` n'écrit jamais rien.

## 5. Câbler les agents (bootstrap)

D'abord un contrôle **en lecture seule** :

```sh
agent-harness bootstrap --check --json    # code 1 s'il reste des changements à faire
```

Puis, si le résultat est satisfaisant, appliquer :

```sh
agent-harness bootstrap --apply --json    # code 0
```

Garanties :

- `--check` ne modifie aucun fichier ;
- `--apply` ne touche que la clé `basemind`, fait une **sauvegarde horodatée**
  (`<config>.agent-harness.bak.<AAAAMMJJHHMMSS>`) avant chaque écriture, puis
  revérifie le résultat ;
- relancer `--apply` ne change rien (idempotent) ;
- les configs JSONC sont resérialisées lorsqu'une modification est nécessaire
  (les commentaires sont normalisés), mais l'original est toujours dans la
  sauvegarde et une config déjà correcte n'est jamais réécrite.

| Fichier modifié (user-global) | Clé |
|---|---|
| `~/.claude.json` (Claude + Claude Teams) | `mcpServers.basemind` |
| `~/.codex/config.toml` | `[mcp_servers.basemind]` |
| `$XDG_CONFIG_HOME/opencode/opencode.json(c)` | `mcp.basemind` |
| `$XDG_CONFIG_HOME/opencode2/opencode.json(c)` | `mcp.basemind` |
| Pi | aucun fichier, fallback CLI `basemind` depuis `PATH` |

## 6. Valider un dépôt

```sh
agent-harness setup --root /chemin/du/repo --json
```

Échoue **fermé** (code 1) si : le chemin n'est pas un worktree git, basemind
n'expose pas les commandes CLI `admin, code, docs, git, memory, overview,
status`, ou l'index du dépôt est vide (`file_count = 0`). Ce compteur décrit
les fichiers de code indexés, pas tous les fichiers du dépôt ; les documents
sont consultables dans leur recherche séparée. Cette commande est également
en lecture seule.

## 7. Self-test (preuve live)

```sh
agent-harness self-test --agent claude --json
```

Sans prouveur live configuré, le résultat est `unsupported` avec **code 3** et
des étapes manuelles actionnables. C'est volontaire : la détection statique
(binaire présent, config présente) est explicitement marquée
`static_detection_is_proof: false` et n'est **jamais** présentée comme une
preuve.

Pour brancher une preuve réelle, fournir un exécutable qui reçoit l'id de
l'agent et imprime un JSON `{"proven": true, "evidence": "..."}` :

```sh
export AGENT_HARNESS_SELFTEST_PROVER=/chemin/vers/mon-proveur
agent-harness self-test --agent codex --json
```

## 8. Revenir en arrière (désinstallation manuelle)

L'installateur n'a pas de commande `uninstall` ; la suppression est simple et
réversible :

```sh
rm -f "$HOME/.local/bin/agent-harness"
rm -rf "$HOME/.local/share/agent-harness"
# pour annuler bootstrap : restaurer les fichiers .agent-harness.bak.* à la main
```

## 9. Dépannage

- **`command not found: agent-harness`** — `~/.local/bin` n'est pas dans le
  `PATH`, ou l'installation n'a pas été faite avec ce compte.
- **`doctor: fail`** — regarder `required.basemind` / `required.orca` dans le
  JSON ; seules ces dépendances peuvent faire échouer le doctor.
- **`bootstrap --check` renvoie 1** — ce n'est pas une erreur : il reste des
  entrées à ajouter. Passer à `--apply`.
- **`setup: fail`** — l'index basemind du dépôt est vide ; lancer
  `basemind --root <repo> scan` puis relancer `setup`.
- **`self-test: unsupported`** — comportement attendu sans prouveur live ;
  suivre les `manual_steps` du JSON.

## 10. Tests (pour reproduire le développement)

```sh
cd vps-harness
python3 -m unittest discover -s tests -t . -v
```

Les 54 tests sont **offline** : les outils externes sont remplacés par un
runner en mémoire et tous les chemins pointent vers un `HOME`/XDG temporaire.
Ils ne touchent jamais les configs globales réelles.

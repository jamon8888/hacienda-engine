# Mise en place du harnais d'agents IA (Claude Code / OpenCode)

Guide pour un développeur qui rejoint le projet et doit configurer les assistants IA
(Claude Code, Codex CLI, OpenCode) sur `hacienda-engine`. Le gouvernail unique du projet
est [ai-rulez](https://github.com/Goldziher/ai-rulez) : toute la configuration source vit
dans `.ai-rulez/`, et les fichiers consommés par les harnais (`CLAUDE.md`, `AGENTS.md`,
`.claude/`, `.codex/`) sont **générés**, jamais édités à la main.

## 1. Vue d'ensemble

```
.ai-rulez/config.toml   ← source de vérité (règles, agents, skills, MCP, presets)
.ai-rulez/rules/        ← règles locales (overrides du projet)
.ai-rulez/agents/       ← agents locaux
.ai-rulez/skills/       ← skills locaux
        │
        │  ai-rulez generate
        ▼
CLAUDE.md, .claude/      ← consommé par Claude Code
AGENTS.md, .codex/       ← consommé par Codex CLI et par OpenCode (norme agents.md)
```

`config.toml` déclare `presets = ["claude", "codex"]` : ai-rulez matérialise donc deux
harnais à partir de la même source :

- **`claude`** → `CLAUDE.md` + `.claude/agents/*.md`, `.claude/skills/*/SKILL.md`,
  `.claude/settings.json`, `.claude/plugins.json`.
- **`codex`** → `AGENTS.md` + `.codex/agents/*.toml`, `.codex/skills/*/SKILL.md`,
  `.codex/commands/*.md`, `.codex/config.toml`, `.codex/plugins.json`.

`AGENTS.md` suit la spécification [agents.md](https://agents.md), un format ouvert lu par
plusieurs harnais — Codex CLI, **OpenCode**, Aider, etc. Il n'y a pas de préset `opencode`
distinct dans `ai-rulez` : OpenCode consomme directement `AGENTS.md`, donc le préset
`codex` couvre les deux outils sans configuration supplémentaire.

La config `.ai-rulez/config.toml` inclut aussi cinq modules distants partagés à l'échelle
du polyrepo `xberg-io` (`xberg-core`, `xberg-languages`, `xberg-cicd`,
`xberg-infrastructure`, `xberg-e2e-generator`, dépôt `xberg-io/agent-conventions`), fusionnés
avec les règles locales via `merge_strategy = 'local-override'` — en cas de conflit, la
règle locale du projet gagne.

## 2. Prérequis

- Accès en lecture au dépôt privé `github.com/xberg-io/agent-conventions` (les includes
  distants échouent silencieusement — `continue-on-error` — sans token valide).
- Un token Git si le dépôt est privé pour vous : `AI_RULEZ_GIT_TOKEN` en variable
  d'environnement, ou `--token`/`-T` en argument CLI.
- `npx` disponible (le serveur MCP Playwright déclaré dans `config.toml` est lancé via
  `npx -y @playwright/mcp@latest`).

## 3. Installer l'outil `ai-rulez`

```bash
# Méthode utilisée sur ce poste (via uv, gestionnaire Python)
uv tool install ai-rulez

# Vérifier
ai-rulez version
```

D'autres méthodes d'installation existent (pip, brew, binaire précompilé) — se référer à
la documentation officielle du projet si `uv` n'est pas disponible sur votre poste. Il n'y
a pas de dépendance ai-rulez dans `task setup` : elle est volontairement hors du toolchain
Rust/binding, à installer une fois par poste de développement.

## 4. Générer les fichiers du harnais

Depuis la racine du dépôt :

```bash
# Aperçu sans écrire sur disque (recommandé la première fois)
ai-rulez generate --dry-run

# Génération réelle (écrit CLAUDE.md, AGENTS.md, .claude/, .codex/)
ai-rulez generate
```

Points de vigilance :

- La première génération télécharge et met en cache les includes distants dans
  `~/.cache/ai-rulez/includes/`. Utiliser `--no-fetch` (`-f`) pour régénérer à partir du
  cache uniquement (utile hors ligne ou en CI restreinte).
- Des avertissements `Duplicate rule collapsed` sont normaux et attendus : ils indiquent
  qu'une règle locale (`.ai-rulez/rules/`) a correctement écrasé l'équivalent builtin
  d'ai-rulez (`atomic-commits`, `branch-hygiene`, `commit-messages`, `safe-git-operations`,
  `tdd-workflow`, `test-alongside-code`, `verify-before-acting`). Ce n'est pas une erreur
  de configuration.
- Ne jamais éditer `CLAUDE.md` ou `AGENTS.md` directement : les deux portent un bandeau
  `AI-RULEZ :: GENERATED FILE — DO NOT EDIT DIRECTLY`. Toute modification doit passer par
  `.ai-rulez/rules/`, `.ai-rulez/agents/`, `.ai-rulez/skills/`, ou par le module distant
  `agent-conventions` (hors du scope de ce dépôt).
- Valider la configuration avant de générer si vous avez touché `.ai-rulez/config.toml` :
  `ai-rulez validate`.

## 5. Configurer Claude Code

Une fois `ai-rulez generate` exécuté :

1. Ouvrir le dépôt avec Claude Code (`claude` en CLI, ou l'extension IDE).
2. `CLAUDE.md` est chargé automatiquement comme contexte projet — il contient les 74
   règles du projet et référence les 32 agents spécialisés définis dans `.claude/agents/`.
3. Les MCP servers déclarés dans `.ai-rulez/config.toml` (`playwright` pour l'automatisation
   navigateur E2E) sont configurés automatiquement dans `.claude/settings.json` — pas de
   configuration manuelle de `.mcp.json` nécessaire pour ceux-là. `.mcp.json` à la racine
   ne contient que ce même serveur, généré.
4. Les skills (`.claude/skills/*/SKILL.md`) sont invocables via l'outil Skill — vérifier
   leur présence avec `/help` ou en listant `.claude/skills/`.
5. Les subagents spécialisés (`hacienda-engineer`, `rust-core-engineer`,
   `wasm-specialist`, etc.) sont disponibles via l'outil Agent — se référer à la section
   "Agents" de `CLAUDE.md` pour savoir quand déléguer.

## 6. Installer les plugins Claude Code — basemind et les agents xberg

`.ai-rulez/config.toml` déclare un marketplace de plugins Claude Code :

```toml
[[marketplaces]]
name = "basemind"
source = "Goldziher/basemind"
type = "github"

[[plugins]]
marketplace = "basemind"
name = "basemind"
scope = "project"
enabled = true
```

`ai-rulez generate` matérialise cette déclaration dans `.claude/plugins.json` (et son
équivalent `.codex/plugins.json`), mais **ce fichier n'est pas lu nativement par Claude
Code** — le mécanisme natif de marketplace/plugin de Claude Code vit dans
`.claude/settings.json` (clés `extraKnownMarketplaces` / `enabledPlugins`), pas dans un
`plugins.json` séparé. `.claude/plugins.json` reste donc pour l'instant une déclaration
d'intention côté ai-rulez : **chaque développeur doit installer le plugin une fois,
manuellement**, à l'intérieur d'une session Claude Code interactive.

### 6.1 basemind — indexation et mémoire du code

[basemind](https://github.com/Goldziher/basemind) fournit des outils MCP/CLI d'indexation
structurelle et historique du code (symboles, appelants, graphe d'appels, historique git
par symbole) — voir `.claude/skills/basemind-tools/SKILL.md` pour l'usage détaillé une
fois installé. Il doit être préféré à `grep`/lecture de fichiers/`git log` bruts pour les
questions structurelles ou historiques sur le code : moins de tokens consommés, réponses
avec chemins et numéros de ligne directement exploitables.

Installation (une fois, dans une session Claude Code ouverte à la racine du dépôt) :

```shell
/plugin marketplace add Goldziher/basemind
/plugin install basemind@basemind
```

Au moment de l'installation, choisir le **scope `project`** pour rester cohérent avec
`scope = "project"` déclaré dans `.ai-rulez/config.toml` (partagé avec l'équipe, pas
seulement pour vous). Puis activer :

```shell
/reload-plugins
```

Vérifier ensuite que les outils basemind apparaissent (`/plugin list --enabled`) et que
le skill `basemind-tools` est bien chargé.

Alternative non interactive (scriptable, par ex. dans un script d'onboarding) :

```bash
claude plugin marketplace add Goldziher/basemind
claude plugin install basemind@basemind --scope project
```

### 6.2 Les agents et skills spécifiques à xberg

Il n'y a pas de marketplace de plugins Claude Code séparé pour xberg : les extensions
spécifiques au projet (32 subagents comme `hacienda-engineer`, `rust-core-engineer`,
`wasm-specialist`, etc., et l'ensemble des skills sous `.claude/skills/`) sont générées
directement par `ai-rulez` à partir de `.ai-rulez/agents/`, `.ai-rulez/skills/`, et des
modules distants `xberg-*` (voir §1) — pas installées via `/plugin install`. Elles sont
donc déjà actives dès que `ai-rulez generate` a été exécuté (§4) ; il n'y a pas d'étape
d'installation supplémentaire à faire pour elles. Ne pas chercher un marketplace
`xberg` à ajouter avec `/plugin marketplace add` — cela n'existe pas, c'est le rôle que
joue `ai-rulez` lui-même pour ce dépôt.

### 6.3 Point de vigilance sécurité

`/plugin marketplace add` et `/plugin install` exécutent du code potentiellement
arbitraire sur votre machine avec vos privilèges utilisateur. N'ajouter que des
marketplaces de confiance — `Goldziher/basemind` est explicitement approuvé pour ce
projet via `.ai-rulez/config.toml`, ne pas ajouter d'autres marketplaces sans validation
de l'équipe.

## 7. Configurer Codex CLI / OpenCode

1. Le fichier `AGENTS.md` à la racine est le point d'entrée standard lu par Codex CLI et
   par OpenCode au démarrage de session dans ce répertoire.
2. `.codex/config.toml` porte la configuration spécifique à Codex (agents, MCP).
3. `.codex/agents/*.toml` et `.codex/skills/*/SKILL.md` sont les équivalents Codex des
   agents/skills Claude — même contenu source, format différent.
4. Pour OpenCode spécifiquement : vérifier dans sa documentation qu'il lit bien
   `AGENTS.md` à la racine du projet ouvert (comportement par défaut de la norme
   agents.md) ; aucune configuration additionnelle n'est fournie par ce dépôt au-delà de
   ce fichier généré.

## 8. Mettre à jour le harnais après un changement de règles

Toute modification de convention (nouvelle règle, nouvel agent, nouveau skill) passe par
`.ai-rulez/` puis régénération :

```bash
# Après avoir édité .ai-rulez/rules/, .ai-rulez/agents/, ou .ai-rulez/skills/
ai-rulez validate
ai-rulez generate --dry-run   # relire le diff attendu
ai-rulez generate
git status                    # CLAUDE.md, AGENTS.md, .claude/, .codex/ doivent apparaître modifiés
```

Committer les fichiers générés avec le changement de source dans le même commit — ils
doivent toujours être synchronisés avec `.ai-rulez/config.toml` dans l'historique Git.

## 9. Skills additionnels (`.agents/`, `skills-lock.json`)

Indépendamment d'ai-rulez, ce dépôt embarque aussi un second registre de skills sous
`.agents/skills/` (verrouillé par `skills-lock.json`, sourcé depuis `mattpocock/skills` sur
GitHub — `code-review`, `tdd`, `research`, `diagnosing-bugs`, `wayfinder`, etc.). C'est un
système distinct d'ai-rulez, à ne pas confondre : `.ai-rulez/` gère les règles et agents
propres au projet xberg/hacienda, `.agents/skills/` est une bibliothèque de skills
génériques d'ingénierie logicielle installée via un gestionnaire de skills tiers. Ne pas
régénérer l'un en pensant régénérer l'autre.

## 10. Vérification rapide

```bash
ai-rulez version                 # outil installé
ai-rulez validate                # config.toml valide
ai-rulez generate --dry-run      # prévisualise sans écrire
head -5 CLAUDE.md AGENTS.md      # bandeau "GENERATED FILE" présent = setup correct
```

Dans une session Claude Code, vérifier aussi que le plugin basemind est bien installé et
actif (§6.1) :

```shell
/plugin list --enabled
```

`basemind` doit apparaître dans la liste, sans quoi les outils MCP/CLI décrits dans
`.claude/skills/basemind-tools/SKILL.md` ne sont pas disponibles pour l'agent.

Si `CLAUDE.md`/`AGENTS.md` n'existent pas encore ou semblent désynchronisés du
`Generated:` timestamp dans leur bandeau, relancer `ai-rulez generate` avant de commencer
à travailler avec l'agent.

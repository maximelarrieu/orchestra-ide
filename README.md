# Orchestra IDE

Un **outil de dev piloté par IA**, et rien d'autre. Un unique agent — l'**Orchestrateur** —
déploie sa propre équipe de sous-agents à la volée pour faire avancer ton code, pendant que
tu gardes une vue claire sur **cinq piliers** : les **Fichiers**, l'état **Git**, l'état
**Docker** du projet, le **Plan/les Tâches** en cours, et **qui fait quoi** parmi les agents.

**Deux interfaces, un seul cœur.** Un TUI terminal (`ratatui`) et une appli de bureau
(`orchestra-desktop`, Dioxus, tout-Rust) consomment **exactement la même logique**
(`orchestra-core`, zéro dépendance d'affichage) — même comportement des deux côtés, voir
`CLAUDE.md`.

## Les 5 piliers

| Pilier | Ce que tu vois | Où |
|---|---|---|
| **Fichiers** | Arborescence du workspace, annotée en direct par ce que les agents lisent/écrivent | TUI `[6]` · Desktop (explorateur, gauche) |
| **Git** | Branche, ahead/behind, staged/unstaged/untracked, diff au clic | TUI `[8]` · Desktop (rail Tâches → GIT) |
| **Docker** | Conteneurs du projet (image, état, ports) si `Dockerfile`/`docker-compose.yml` présent — lecture seule | TUI `[9]` · Desktop (rail Tâches → DOCKER) |
| **Plan / Tâches** | Plan établi par l'Orchestrateur, avancement tâche par tâche | TUI (panneau Plan) · Desktop (rail Tâches) |
| **Agents** | Statut live de l'Orchestrateur et des sous-agents déployés (repos/réflexion/actif/terminé) | TUI (sidebar) · Desktop (`SquadPanel`) |

Complètent le tableau : **Terminal** (commandes réellement exécutées par les agents),
**Mémoire** (`.orchestra/memory.md`, notes partagées entre agents et sessions), **Contexte**
(fichiers touchés cette session) et **Docs** (persona, ADRs, mémoire, `.md` du workspace —
consultables et éditables).

## Démarrer

```bash
# TUI — ouvre un dossier de projet existant (ou lance sans argument dans le dossier courant)
cargo run -p orchestra-tui -- /chemin/vers/mon-projet

# Crée un nouvel Espace de Contexte (.orchestra/{config.json, persona.md, adr/})
cargo run -p orchestra-tui -- init ./mon-espace

# GUI de bureau (Dioxus) — prérequis webview : WebView2 (Windows) / webkit2gtk (Linux),
# voir crates/orchestra-desktop/README.md
cargo run -p orchestra-desktop
```

Sans espace fourni : ouvre un dossier de code existant (l'outil l'« adopte » comme Espace de
Contexte) ou crée-en un nouveau. L'Orchestrateur déploie ensuite sa propre équipe à la volée
— rien à pré-configurer côté agents/skills.

### Activer le LLM — Claude, Gemini, **ou un modèle local (Ollama)**, au choix

```bash
export ANTHROPIC_API_KEY="sk-ant-..."   # Claude (préféré si les deux clés cloud sont présentes)
export GEMINI_API_KEY="..."             # Gemini (repli automatique si Claude est indisponible)
# Optionnel :
export ORCHESTRA_PROVIDER=gemini        # anthropic | gemini | ollama — force un fournisseur unique
export ORCHESTRA_MODEL=gemini-2.5-flash
```

**Aucune clé API ? Utilise un modèle local via [Ollama](https://ollama.com)** — `ollama serve`
tournant en local, aucun compte ni clé requis :

```bash
export ORCHESTRA_PROVIDER=ollama              # modèle qwen2.5-coder par défaut
# Optionnel :
export ORCHESTRA_OLLAMA_MODEL=mistral         # un autre modèle déjà `ollama pull`é (mistral, gpt-oss…)
export ORCHESTRA_OLLAMA_HOST=http://localhost:11434   # défaut, à changer si Ollama tourne ailleurs

cargo run -p orchestra-tui -- /chemin/vers/mon-projet
```

Sans `ORCHESTRA_PROVIDER=ollama` explicite, Ollama ne rejoint **jamais** la chaîne de repli
automatique par défaut — sauf si tu définis `ORCHESTRA_OLLAMA_MODEL` sans forcer de
fournisseur : il devient alors un repli local après Claude/Gemini. Sans clé ni Ollama
configuré (ou si tout échoue), l'outil bascule en **mode simulé** — pleinement utilisable
hors-ligne, sans appel réseau.

### Activer Git / GitHub

Dans `.orchestra/config.json` :

```json
"integrations": {
  "git": { "auto_branching": true, "main_branch": "main" },
  "github": { "repo": "owner/repo", "token_env_var": "GITHUB_TOKEN" }
}
```

Le panneau **Git** (branche/diff) fonctionne dès que le dossier est un dépôt Git, sans aucune
configuration — c'est de la lecture seule côté UI. La configuration `integrations.git`
n'active que les **outils Git pour l'agent** (`Git_Commit`, `Git_Create_Branch`…). GitHub
requiert en plus un token lu depuis la variable d'environnement déclarée (jamais en dur).

### Docker

Le panneau **Docker** détecte automatiquement un `Dockerfile`/`docker-compose.yml` à la
racine du projet et affiche l'état des conteneurs (`docker compose ps`) si le démon Docker
est joignable — sans configuration, en **lecture seule** (pas d'actions start/stop/logs dans
cette version).

## Documentation

| Doc | Contenu |
|---|---|
| [`docs/FONCTIONNEL.md`](docs/FONCTIONNEL.md) | Vision produit, Espace de Contexte, parcours utilisateur, état des fonctionnalités |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Crates, modèle de données, contrat d'événements, décisions techniques (diagrammes Mermaid) |
| [`docs/JOURNAL.md`](docs/JOURNAL.md) | Avancement par phase |
| [`CLAUDE.md`](CLAUDE.md) | Règle de parité TUI ⇄ Desktop pour les agents qui contribuent au projet |

Doc technique générée du code : `cargo doc -p orchestra-core --open`.

## Structure (workspace Cargo)

```
crates/
├─ orchestra-core/     # domaine pur — AUCUNE dépendance UI
│  └─ src/{error,events,runtime,llm,skills,markdown_skill,memory,
│           orchestration,integrations,git,docker,explorer,browser,
│           registry,session,scaffold,diff,model/{config,space}}.rs
├─ orchestra-tui/      # frontend ratatui + CLI — consomme orchestra-core
│  └─ src/{main,app,dashboard,editor,markdown,wizard}.rs
└─ orchestra-desktop/  # GUI bureau Dioxus, tout-Rust — consomme orchestra-core
   └─ src/{main,state,components,styles}.rs
```

## Raccourcis clavier (TUI)

| Touche | Action |
|---|---|
| `[1]` | Lancer une intention (objectif rapide, one-shot via l'Orchestrateur) |
| `[5]` | Assistant — conversation persistante avec l'Orchestrateur |
| `[6]` | Fichiers |
| `[7]` | Modifications (diffs des fichiers touchés par les agents cette session) |
| `[8]` | Git (`[r]` rafraîchir, `Entrée` diff du fichier sélectionné) |
| `[9]` | Docker (`[r]` rafraîchir) |
| `[0]` | Terminal (commandes exécutées par les agents) |
| `[m]` | Mémoire (`.orchestra/memory.md`) |
| `[c]` | Contexte (fichiers touchés cette session) |
| `[2]` | Docs (persona/ADRs/mémoire/`.md`) + visualiseur Markdown |
| `[3]` | Changer d'Espace (sélecteur des espaces connus, navigateur de dossiers, création) |
| `[4]` | Éditer le persona |
| `Tab` / `Maj+Tab` | Session suivante / précédente (onglets) |
| `Ctrl+W` | Fermer la session active |
| `q` / `Échap` | Quitter (ou revenir à l'écran précédent selon le contexte) |

Le Desktop offre les mêmes capacités en clic-souris (voir `crates/orchestra-desktop/README.md`).

## Contribuer

Voir [`CLAUDE.md`](CLAUDE.md) : toute logique nouvelle vit dans `orchestra-core` (testée,
zéro dépendance UI), puis est exposée à l'identique dans `orchestra-tui` **et**
`orchestra-desktop` — jamais une seule des deux. `cargo test -p orchestra-core
-p orchestra-tui` et `cargo clippy --workspace --all-targets` doivent rester au vert.

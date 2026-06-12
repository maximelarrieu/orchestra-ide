# Orchestra IDE — Prototype CLI (Rust)

Moteur + interface d'un « IDE pour l'ère agentique ». TUI aujourd'hui (`ratatui`),
portage Tauri + React prévu — d'où le **découplage strict** logique métier / affichage.

> Plan d'architecture complet (5 phases) : voir `../ORCHESTRA_PLAN.md`.
> Ce dossier sera recopié à terme dans le repo dédié `maximelarrieu/orchestra-ide`.

## Structure (workspace Cargo)

```
crates/
├─ orchestra-core/   # domaine pur — AUCUNE dépendance UI
│  └─ src/{error,events,scaffold,model/{project_type,config,space,skill_id}}.rs
└─ orchestra-tui/    # frontend ratatui + CLI — consomme orchestra-core
   └─ src/{main,dashboard,wizard}.rs
```

## Phase 1 — Modèle + dashboard ✅

Modèle des Espaces de Contexte agnostiques + coquille ASCII du dashboard (3 zones :
en-tête / écran radar / menu). **Sans LLM, sans agent** : le radar est vide.

```bash
# Ouvre l'espace exemple fourni
cargo run -p orchestra-tui -- examples/recherche-immo-aix
# Quitter : q ou Échap
```

Sans argument, l'outil tente de charger un `.orchestra/config.json` du répertoire courant ;
s'il n'en trouve pas, le dashboard s'affiche en état « Aucun espace chargé ».

## Phase 2 — `orchestra init` ✅

Assistant interactif qui génère un Espace de Contexte selon le type de projet
(Dev / Nutrition / Langue / Immobilier), en pré-remplissant la matrice de Skills et
d'agents par défaut.

```bash
# Crée .orchestra/{config.json, persona.md, adr/} dans le dossier cible (défaut : .)
cargo run -p orchestra-tui -- init ./mon-espace
```

La logique d'écriture vit dans `orchestra-core::scaffold` (pure, testée) ; les prompts
terminal restent dans `orchestra-tui::wizard` — découplage métier/affichage respecté.
Un espace déjà initialisé n'est jamais écrasé (`SpaceAlreadyExists`).

## Phases suivantes

3. Runtime d'agents + flux temps réel (`tokio::sync::mpsc`) → radar vivant.
4. Intégration LLM + Skills écosystème Dev (Git / Jira / GitHub).
5. Agent Documentaliste (Doc_Auto_Update, Mermaid) + finitions.

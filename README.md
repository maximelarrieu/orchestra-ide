# Orchestra IDE — Prototype CLI (Rust)

Moteur + interface d'un « IDE pour l'ère agentique ». TUI aujourd'hui (`ratatui`),
portage Tauri + React prévu — d'où le **découplage strict** logique métier / affichage.

> Plan d'architecture complet (5 phases) : voir `../ORCHESTRA_PLAN.md`.
> Ce dossier sera recopié à terme dans le repo dédié `maximelarrieu/orchestra-ide`.

## Structure (workspace Cargo)

```
crates/
├─ orchestra-core/   # domaine pur — AUCUNE dépendance UI
│  └─ src/{error,events,model/{project_type,config,space,skill_id}}.rs
└─ orchestra-tui/    # frontend ratatui — consomme orchestra-core
   └─ src/{main,dashboard}.rs
```

## Phase 1 (actuelle)

Modèle des Espaces de Contexte agnostiques + coquille ASCII du dashboard (3 zones :
en-tête / écran radar / menu). **Sans LLM, sans agent** : le radar est vide.

### Lancer

```bash
# Ouvre l'espace exemple fourni
cargo run -p orchestra-tui -- examples/recherche-immo-aix
# Quitter : q ou Échap
```

Sans argument, l'outil tente de charger un `.orchestra/config.json` du répertoire courant ;
s'il n'en trouve pas, le dashboard s'affiche en état « Aucun espace chargé ».

## Phases suivantes

2. Commande `orchestra init` (scaffolding interactif).
3. Runtime d'agents + flux temps réel (`tokio::sync::mpsc`) → radar vivant.
4. Intégration LLM + Skills écosystème Dev (Git / Jira / GitHub).
5. Agent Documentaliste (Doc_Auto_Update, Mermaid) + finitions.

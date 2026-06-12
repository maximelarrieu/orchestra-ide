# Orchestra IDE — Prototype CLI (Rust)

Moteur + interface d'un « IDE pour l'ère agentique ». TUI aujourd'hui (`ratatui`),
portage Tauri + React prévu — d'où le **découplage strict** logique métier / affichage.

## 📚 Documentation

| Doc | Contenu |
|---|---|
| [`docs/FONCTIONNEL.md`](docs/FONCTIONNEL.md) | Vision produit, Espaces de Contexte, parcours utilisateur, état des fonctionnalités |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Crates, modèle de données, flux d'événements `mpsc`, décisions techniques (diagrammes Mermaid) |
| [`docs/JOURNAL.md`](docs/JOURNAL.md) | Avancement par phase : livré / limites connues / à venir |

Doc technique générée du code : `cargo doc -p orchestra-core --open`.

## Structure (workspace Cargo)

```
crates/
├─ orchestra-core/   # domaine pur — AUCUNE dépendance UI
│  └─ src/{error,events,runtime,scaffold,model/{project_type,config,space,skill_id}}.rs
└─ orchestra-tui/    # frontend ratatui + CLI — consomme orchestra-core
   └─ src/{main,app,dashboard,wizard}.rs
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

## Phase 3 — Runtime d'agents, radar vivant ✅

Le radar n'est plus une coquille. `orchestra-core::runtime::spawn` lance chaque agent de
l'espace comme une tâche `tokio` qui publie des `AgentEvent` sur un canal
`tokio::sync::mpsc` ; le TUI les consomme en direct.

```bash
cargo run -p orchestra-tui -- examples/recherche-immo-aix
# Dans le dashboard : [1] lance l'orchestre → le radar défile en temps réel.  [q] quitte.
```

Boucle async multiplexée par `tokio::select!` (clavier via `EventStream` + flux d'agents +
tick de rafraîchissement). L'agrégation du flux (`orchestra-tui::app::App`) est isolée du
rendu et testée.

## Phase 4a — LLM (Claude **ou** Gemini) + Skills Dev exécutables ✅

Les agents deviennent réellement intelligents. `orchestra-core::llm` appelle l'API d'un
fournisseur d'IA **au choix** en HTTP brut (pas de SDK Rust officiel) ; une représentation
neutre découple la boucle agentique du format de chaque provider. Chaque agent mène une
**boucle agentique** : le modèle raisonne, demande un *outil*, on l'exécute, on lui renvoie
le résultat. Les trois Skills Dev sont branchés sur le système (`orchestra-core::skills`) :
`Read_File`, `Write_File_Validated`, `Execute_Terminal_Command`, confinés au workspace.

```bash
# Claude (défaut claude-opus-4-8) :
export ANTHROPIC_API_KEY="sk-ant-..."
# …ou Gemini (défaut gemini-2.0-flash) :
export GEMINI_API_KEY="..."
# Forcer le fournisseur / le modèle si besoin :
export ORCHESTRA_PROVIDER=gemini      # anthropic | gemini
export ORCHESTRA_MODEL=gemini-2.0-flash

cargo run -p orchestra-tui -- examples/recherche-immo-aix
# [1] lance l'orchestre — le radar affiche les actions réelles des agents.
```

Sélection automatique selon la clé présente (`ANTHROPIC_API_KEY` puis `GEMINI_API_KEY`),
`ORCHESTRA_PROVIDER` ayant priorité. Clés **jamais en dur**, lues depuis l'environnement.

**Repli automatique** : sans aucune clé (ou si l'API est injoignable), on retombe sur les
agents *simulés* de la Phase 3 — l'appli reste pleinement fonctionnelle hors-ligne, et la
compilation/les tests n'exigent aucune clé. L'en-tête indique le mode (`🤖 <modèle>` ou
`simulé · clé API absente`), et le radar rappelle alors quelles variables définir. La
signature de `runtime::spawn` n'a pas changé.

> ⚠️ `Execute_Terminal_Command` exécute des commandes shell dans le workspace (capacité
> assumée pour un IDE de dev) : sortie plafonnée, délai max 30 s, chemins confinés.

## Phases suivantes

4b. Intégrations écosystème : Git / GitHub / Jira.
5. Agent Documentaliste (Doc_Auto_Update, Mermaid) + finitions.

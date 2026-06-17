# orchestra-desktop — interface graphique (Dioxus)

Port **bureau** d'Orchestra IDE, en **Rust pur** (Dioxus). L'UI consomme directement
`orchestra-core` (aucune frontière IPC, aucun toolchain Node) : elle appelle `runtime`,
`ContextSpace`, etc. et fait un `match` natif sur les `AgentEvent` — la même couture que le TUI.

> État : **première tranche verticale** — charge l'espace exemple, liste les agents, lance
> l'orchestration et affiche en direct le radar + le panneau Plan + l'approbation du plan.

## Prérequis de build (webview système)

Dioxus desktop s'appuie sur la webview de l'OS :

- **Windows** : **WebView2** (préinstallé sur Windows 10/11). Rien à faire en général.
- **macOS** : WebKit (système).
- **Linux** : paquets de dev `webkit2gtk` (ex. Debian/Ubuntu :
  `sudo apt install libwebkit2gtk-4.1-dev libxdo-dev libappindicator3-dev`).

## Lancer

```bash
# Depuis la racine du dépôt
cargo run -p orchestra-desktop
```

(La fenêtre charge `examples/recherche-immo-aix`. Définis `GEMINI_API_KEY` ou
`ANTHROPIC_API_KEY` pour une orchestration réelle, sinon le mode simulé s'affiche.)

## Structure (`src/`)

| Fichier | Rôle |
|---|---|
| `main.rs` | Point d'entrée (`launch`) + composant racine (composition + signaux). |
| `state.rs` | État + **pont vers le cœur** (`drive_orchestration`, `PlanRow`) — isole « parler à `orchestra-core` » du rendu. |
| `components.rs` | Composants de présentation (`header`, `plan_panel`, `radar`). |
| `styles.rs` | CSS de la fenêtre. |

Les composants sont de simples `fn -> Element` ; ils deviendront des `#[component]` avec props
quand ils grossiront (Dioxus, c'est du Rust normal — on découpe librement).

## Prochaines étapes

- Sélecteur de dossier d'espace (au lieu du chemin codé en dur).
- Saisie de l'objectif + chat coordinateur (`[5]`).
- Sélecteur de skills et éditeur de persona (reprise des écrans du TUI).

# orchestra-desktop — interface graphique (Dioxus)

Port **bureau** d'Orchestra IDE, en **Rust pur** (Dioxus). L'UI consomme directement
`orchestra-core` (aucune frontière IPC, aucun toolchain Node) : elle appelle `runtime`,
`ContextSpace`, etc. et fait un `match` natif sur les `AgentEvent` — la même couture que le TUI.

> État : montée en parité avec le TUI. Présent : **navigation par vues**, **sélecteur d'espace**
> (chemin), **saisie d'objectif** + orchestration (radar + panneau Plan + approbation),
> **Documents** (liste + visualiseur), **Agents & skills** (catalogue à cocher, persisté).
> À venir : **chat coordinateur** et **édition du persona**.

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

- **Chat coordinateur** (`[5]`) : conversation persistante + orchestration inline.
- **Édition du persona** (`[4]`) : zone de texte + sauvegarde via le cœur.
- Création/édition de fiches de skill depuis la vue Agents (`[n]`/`[e]`).
- Sélecteur de dossier natif (au lieu de la saisie de chemin).
- Sidebar « orchestre live » + bandeau d'état (fournisseur LLM actif).

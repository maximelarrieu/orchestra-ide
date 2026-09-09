# orchestra-desktop — interface graphique (Dioxus)

Port **bureau** d'Orchestra IDE, en **Rust pur** (Dioxus). L'UI consomme directement
`orchestra-core` (aucune frontière IPC, aucun toolchain Node) : elle appelle `runtime`,
`ContextSpace`, etc. et fait un `match` natif sur les `AgentEvent` — la même couture que le TUI.

> État : parité avec le TUI sur les 5 piliers dev. Présent : **onglets de session**, **barre
> d'espaces** (ouvrir/créer/parcourir/reprendre un projet existant), **conversation avec
> l'Orchestrateur** (squad live, plan + approbation, mémoire, contexte), **explorateur de
> fichiers** annoté en temps réel (« orchestre en verre »), **visualiseur/éditeur** de fichier
> (Markdown rendu avec diagrammes Mermaid, diff des changements d'agent, édition/sauvegarde),
> **Terminal** (commandes exécutées par les agents), **panneau Git** (branche, ahead/behind,
> staged/unstaged/untracked, diff au clic), **panneau Docker** (conteneurs du projet, lecture
> seule), et **Docs** (persona/ADR/mémoire, éditables au même endroit que les fichiers).

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

(La fenêtre démarre sans session : ouvre ou crée un espace via la barre d'espaces. Définis
`GEMINI_API_KEY` ou `ANTHROPIC_API_KEY` pour une orchestration réelle, sinon le mode simulé s'affiche.)

## Structure (`src/`)

| Fichier | Rôle |
|---|---|
| `main.rs` | Point d'entrée (`launch`) + composant racine (composition des signaux et du shell 5 zones). |
| `state.rs` | État (`DesktopSession`) + **pont vers le cœur** (`apply_event`, `refresh_dev_status`, `save_space_document`…) — isole « parler à `orchestra-core` » du rendu. |
| `components.rs` | Composants de présentation (`FileExplorer`, `CenterPane`, `TaskRail`, `SquadPanel`, `chat_view`…). |
| `styles.rs` | CSS de la fenêtre. |

Certains composants sont de simples `fn -> Element`, d'autres des `#[component]` avec props
(Dioxus, c'est du Rust normal — on découpe librement selon la taille).

## Prochaines étapes

- Actions Docker (start/stop/logs) — le panneau Docker actuel est volontairement en lecture seule.
- Sélecteur de dossier natif (au lieu du navigateur maison).

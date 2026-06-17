# Orchestra IDE — consignes pour les agents

## Règle de parité TUI ⇄ GUI (NON négociable)

Toute nouvelle fonctionnalité **ou** amélioration d'une fonctionnalité existante doit être
livrée **simultanément dans les deux interfaces** :

- `orchestra-tui` — l'application CLI / terminal (ratatui) ;
- `orchestra-desktop` — l'application graphique de bureau (Dioxus).

Les deux applications doivent **toujours offrir le même fonctionnement** (mêmes capacités,
même comportement). On ne livre jamais une feature dans une seule des deux.

### Comment respecter cette règle sans dupliquer

Le projet impose un **découplage strict** : toute la logique vit dans `orchestra-core`, qui ne
dépend d'**aucune** bibliothèque d'affichage. Donc :

1. Mettre la logique (modèle, règles, accès disque, catalogue, édition…) dans `orchestra-core`.
2. Les deux UIs se contentent d'appeler ces fonctions et de consommer le type `AgentEvent`.

Ainsi une feature ajoutée au cœur est immédiatement branchable des deux côtés, à l'identique.

### Checklist avant de considérer une feature « terminée »

- [ ] Logique ajoutée/centralisée dans `orchestra-core` (testée).
- [ ] Exposée dans `orchestra-tui`.
- [ ] Exposée dans `orchestra-desktop`.
- [ ] `cargo test -p orchestra-core -p orchestra-tui` au vert, `clippy` sans warning.
- [ ] Docs mises à jour (`docs/ARCHITECTURE.md`, `docs/JOURNAL.md`, READMEs concernés).

> Note : `orchestra-desktop` (Dioxus) ne compile pas dans l'environnement cloud (webview
> Linux absente) ; il se compile sur poste (WebView2 sur Windows). On itère sur ses éventuelles
> erreurs d'API au build local.

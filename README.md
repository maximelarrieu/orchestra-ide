# Orchestra IDE

Un tableau de bord de développement qui pilote une **ferme d'agents Claude Code**.

Ce n'est ni un clone de VS Code ni un clone de Codex. Tu écris un brief de feature,
un orchestrateur analyse et compose l'équipe d'agents nécessaire, et tu suis depuis
ton terminal ce que chaque agent fait en direct, ticket par ticket, projet par projet,
token par token.

## État

**Phase 0 livrée** : squelette, daemon, base SQLite, socket, tableau de bord.
Le suivi des coûts (phase 1), les tickets et l'orchestrateur (phase 2) et l'exécution
des agents (phase 3) arrivent ensuite. Le plan complet est dans
`~/.claude/plans/jai-un-projet-orchestra-ide-wise-salamander.md`.

Ce qui marche aujourd'hui :

```sh
cargo build --workspace
./target/debug/orchestra daemon      # démarre le daemon (ou laisse le client le faire)
./target/debug/orchestra project add ~/mon-projet
./target/debug/orchestra tui         # tableau de bord
```

`orchestra ping`, `orchestra status`, `orchestra project list` et `orchestra usage`
complètent la ligne de commande.

## Principes

- **Zéro code LLM.** Les agents sont des processus `claude` (Claude Code) ; Orchestra
  parse leur flux `stream-json` et ne parle jamais à une API de modèle.
- **Une seule interface**, en terminal (ratatui), pensée pour vivre dans un pane zellij.
  Le daemon détient tout l'état ; toute autre interface serait un simple client du socket.
- **Isolation par Git.** Chaque ticket travaille dans son propre worktree, sur sa branche.
- **Tokens bruts = vérité.** Le montant en dollars est indicatif, calculé à l'affichage
  depuis une grille tarifaire que tu édites.

## Architecture

Cinq crates, détaillées dans `docs/ARCHITECTURE.md` :

| Crate | Rôle |
|---|---|
| `orchestra-core` | Types, règles, protocole, parsing. Aucune I/O. |
| `orchestra-daemon` | Base SQLite, bus d'événements, serveur socket, superviseur d'agents. |
| `orchestra-tui` | Tableau de bord ratatui, client du socket. |
| `orchestra` | Le binaire : `daemon`, `tui`, `project`, `usage`, `ping`, `status`. |
| `orchestra-hook` | Shim minuscule appelé par les hooks de Claude Code. |

## Licence

MIT.

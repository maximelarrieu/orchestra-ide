# Orchestra IDE

Un tableau de bord de développement qui pilote une **ferme d'agents Claude Code**.

Ce n'est ni un clone de VS Code ni un clone de Codex. Tu écris un brief de feature,
un orchestrateur analyse et compose l'équipe d'agents nécessaire, et tu suis depuis
ton terminal ce que chaque agent fait en direct, ticket par ticket, projet par projet,
token par token.

## État

**Phases 0 et 1 livrées** : le daemon, la base, le tableau de bord, et le suivi des
coûts. Les tickets et l'orchestrateur (phase 2) puis l'exécution des agents (phase 3)
arrivent ensuite. Le plan complet est dans
`~/.claude/plans/jai-un-projet-orchestra-ide-wise-salamander.md`.

Orchestra compte déjà **toutes** tes sessions Claude Code, y compris celles que tu
lances toi-même, groupées par dépôt :

```sh
cargo install --path crates/orchestra
orchestra tui                    # tableau de bord ; touche 4 pour les coûts
orchestra usage --by model --since 7d
orchestra usage --by project
orchestra project add ~/mon-projet
```

`orchestra ping` et `orchestra status` complètent la ligne de commande. Le daemon
démarre tout seul au premier appel.

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
| `orchestra-daemon` | Base SQLite, bus d'événements, serveur socket, suivi des transcripts, superviseur d'agents. |
| `orchestra-tui` | Tableau de bord ratatui, client du socket. |
| `orchestra` | Le binaire : `daemon`, `tui`, `project`, `usage`, `ping`, `status`. |
| `orchestra-hook` | Shim minuscule appelé par les hooks de Claude Code. |

## Licence

MIT.

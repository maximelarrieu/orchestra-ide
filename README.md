# Orchestra IDE

Un tableau de bord de développement qui pilote une **ferme d'agents Claude Code**.

Ce n'est ni un clone de VS Code ni un clone de Codex. Tu écris un brief de feature,
un orchestrateur analyse et compose l'équipe d'agents nécessaire, et tu suis depuis
ton terminal ce que chaque agent fait en direct, ticket par ticket, projet par projet,
token par token.

## État

**Phases 0 à 3 livrées** : le daemon, le suivi des coûts, les tickets,
l'orchestrateur, et les agents qui exécutent vraiment. L'intégration zellij
(phase 4) arrive ensuite. Le plan complet est dans
`~/.claude/plans/jai-un-projet-orchestra-ide-wise-salamander.md`.

```sh
cargo install --path crates/orchestra
cargo install --path crates/orchestra-hook   # le garde-fou, indispensable
orchestra init                               # installe le catalogue de rôles
orchestra project add ~/mon-projet
orchestra tui                                # tableau de bord
```

Les statuts sont colorés et portent chacun un symbole, de sorte que la couleur ne
soit jamais la seule information : `●` en cours, `✓` terminé, `✗` échoué, `◆` à
relire, `⚠` bloqué par le garde-fou.

Dans le tableau de bord : `n` crée un ticket, `Entrée` l'ouvre, `p` demande une
équipe à l'orchestrateur, `a` la relit et l'ajuste, `y` l'accepte, `L` lance les
agents. Depuis un ticket, `Entrée` sur un agent ouvre son flux en direct, où `s`
lui envoie une consigne, `S` l'interrompt pour le réorienter, `x` l'arrête. La
touche `4` ouvre les coûts, `?` l'aide.

En ligne de commande :

```sh
orchestra ticket new --project mon-projet --title "…" --brief-file brief.md --plan
orchestra ticket accept 12
orchestra ticket launch 12 --follow
orchestra tail backend --ticket 12          # un agent en plein écran
orchestra agent steer backend "ajoute aussi un test"
orchestra usage --by role --since 7d
orchestra project prune --dry-run           # oublie les dépôts disparus
```

Orchestra compte **toutes** tes sessions Claude Code, y compris celles que tu lances
toi-même, groupées par dépôt. Le daemon démarre tout seul au premier appel.

## Ce qui protège ton dépôt

Les agents tournent avec les permissions contournées : sans garde-fou, une commande
malheureuse toucherait n'importe quoi. C'est exactement ce qui est arrivé à la v1 de
ce projet, où un agent a exécuté `mv *.md docs/` sur le dépôt lui-même.

`orchestra-hook` est appelé par Claude Code avant chaque appel d'outil. Il décide
seul, sans parler au daemon, à partir de son environnement : toute écriture hors du
worktree du ticket est refusée, avec un motif que l'agent lit et comprend. Les règles
sont dans `orchestra-core/src/guard.rs`, partagées par le daemon et le garde pour
qu'ils ne puissent pas diverger.

Si `orchestra-hook` n'est pas installé, les agents tournent **sans garde-fou**.

## Les rôles

`orchestra init` installe six rôles dans `~/.config/orchestra/roles` : architecte,
backend, frontend, tests, relecteur, documentation. Ce sont de simples fichiers
Markdown avec une entête, dans l'esprit des sous-agents de Claude Code :

```markdown
---
name: backend
description: Implémente la logique serveur et les migrations.
model: sonnet
effort: high
git: confined   # « full » ouvre push, merge et rebase, dans son worktree
---
Tu es l'ingénieur backend de l'équipe…
```

Édite-les : ce sont les consignes que suivront tes agents. Un projet peut redéfinir
n'importe quel rôle dans `<projet>/.orchestra/roles/`. L'écran 8 du tableau,
« Rôles & règles », les liste avec leurs droits : `e` pour éditer, `p` pour ouvrir
ou fermer git, `d` pour supprimer, `:role add <nom>` pour en créer un.

## Principes

- **Zéro code LLM.** Les agents sont des processus `claude` (Claude Code) ; Orchestra
  parse leur flux `stream-json` et ne parle jamais à une API de modèle.
- **Une seule interface**, en terminal (ratatui), pensée pour vivre dans un pane zellij.
  Le daemon détient tout l'état ; toute autre interface serait un simple client du socket.
- **Isolation par Git.** Chaque ticket travaille dans son propre worktree, sur sa
  branche. Un garde-fou refuse toute écriture qui en sortirait, et `git push` et
  `git checkout` sont interdits : ton dépôt principal n'est jamais touché.
- **L'orchestrateur propose, tu décides.** Aucune équipe ne démarre sans que tu aies
  relu ses rôles, ses objectifs et son ordre d'exécution.
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

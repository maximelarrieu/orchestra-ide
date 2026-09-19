# Orchestra IDE v2 — consignes pour les agents

Dashboard de dev pilotant une ferme d'agents **Claude Code**. Plan de référence :
`~/.claude/plans/jai-un-projet-orchestra-ide-wise-salamander.md` ; architecture : `docs/ARCHITECTURE.md`.

## Règles non négociables

1. **Une seule UI** (ratatui). Le daemon possède tout l'état ; toute UI est un client du socket.
2. **`orchestra-core` n'a aucune dépendance d'I/O** : pas de tokio, rusqlite, ratatui. Types, règles, parsing seulement.
3. **Zéro code LLM.** Les agents sont des processus `claude`. On parse leur stream, on ne parle jamais à une API de modèle.
4. **Parsing tolérant** du stream-json et des transcripts : `#[serde(default)]`, `#[serde(flatten)] rest`, `#[serde(other)]`.
   Toute évolution du parseur s'accompagne d'une fixture réelle dans `crates/orchestra-core/tests/fixtures/`.
5. **Les agents ne travaillent que dans un worktree git** créé par le daemon. Jamais dans le dépôt principal.
   L'intégrateur (`config.integration`) est le seul à qui git est ouvert, et il reste
   lui aussi dans son worktree : la fusion dans la branche par défaut est faite par le
   daemon en `--ff-only`, pas par un agent.
6. **Tokens bruts = vérité.** Le `$` est indicatif, calculé à la requête depuis la grille de `config.toml`.
7. Un seul type `Event` (`orchestra-core/src/events.rs`) : persisté dans SQLite puis diffusé. Rien n'est affiché qui ne soit un événement ou une lecture de la base.
8. **Le coût se tarifie depuis les modèles réellement utilisés**, jamais depuis
   l'alias configuré : `haiku` n'est pas dans la grille, `claude-haiku-4-5-…` l'est.
9. **L'orchestrateur propose, l'utilisateur décide.** Aucune équipe ne démarre sans
   passage par l'écran de relecture.
10. **Les règles du garde-fou vivent dans `orchestra-core/src/guard.rs`**, jamais
    dupliquées : le daemon et `orchestra-hook` doivent en avoir la même lecture.
11. **La couleur ne porte jamais une information seule** : toujours un symbole avec,
    et pas de distinction qui repose sur l'opposition rouge / vert (`tui/src/theme.rs`).
12. **Toute équipe se termine par une relecture** ajoutée d'office à la proposition
    (`orchestra-core/src/review.rs`). Son verdict est lu par la machine : il peut
    relancer des tours de correction, bornés par `review.max_rounds`. Un verdict
    illisible arrête la boucle — le silence ne vaut pas accord.
13. **Le superviseur décide du statut d'un agent, pas le flux.** Une interruption y
    ressemble à un échec ; seul celui qui a envoyé le signal sait ce qui s'est passé.

## Checklist avant de considérer une tâche terminée

- [ ] `cargo build --workspace` et `cargo test --workspace` au vert, `cargo clippy --workspace -- -D warnings` propre.
- [ ] Nouveau comportement du daemon → commande dans `protocol.rs` + gestion dans `daemon.rs` + affichage TUI si pertinent.
- [ ] Nouvelle table / colonne → nouvelle migration `NNNN_*.sql`, jamais d'édition d'une migration livrée.
- [ ] Faits appris sur le CLI `claude` (flags, formats) consignés dans `docs/CLAUDE_CLI_NOTES.md`.

## Environnement

`cat` est un alias de `bat` dans le shell de l'utilisateur : écrire un fichier avec `cat > f <<EOF`
y injecte la décoration de bat. Utiliser `tee f >/dev/null <<EOF` ou `command cat`.

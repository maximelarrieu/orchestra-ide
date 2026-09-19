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
6. **Tokens bruts = vérité.** Le `$` est indicatif, calculé à la requête depuis la grille de `config.toml`.
7. Un seul type `Event` (`orchestra-core/src/events.rs`) : persisté dans SQLite puis diffusé. Rien n'est affiché qui ne soit un événement ou une lecture de la base.

## Checklist avant de considérer une tâche terminée

- [ ] `cargo build --workspace` et `cargo test --workspace` au vert, `cargo clippy --workspace -- -D warnings` propre.
- [ ] Nouveau comportement du daemon → commande dans `protocol.rs` + gestion dans `daemon.rs` + affichage TUI si pertinent.
- [ ] Nouvelle table / colonne → nouvelle migration `NNNN_*.sql`, jamais d'édition d'une migration livrée.
- [ ] Faits appris sur le CLI `claude` (flags, formats) consignés dans `docs/CLAUDE_CLI_NOTES.md`.

## Environnement

`cat` est un alias de `bat` dans le shell de l'utilisateur : écrire un fichier avec `cat > f <<EOF`
y injecte la décoration de bat. Utiliser `tee f >/dev/null <<EOF` ou `command cat`.

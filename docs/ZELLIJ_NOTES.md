# Ce qu'on sait du CLI zellij

Vérifié sur **zellij 0.45.1**. Comme pour `claude`, ce fichier existe pour que le
prochain qui touche `crates/orchestra-daemon/src/zellij.rs` n'ait pas à
redécouvrir ce qui suit.

## Les panes ont un identifiant, et `new-pane` le rend

```
$ zellij action new-pane --help
Open a new pane in the specified direction [right|down] …
Returns: Created pane ID (format: terminal_<id> or plugin_<id>)
```

C'est ce qui rend tout le reste possible : sans cet identifiant sur la sortie
standard, un pane ouvert ne serait plus jamais adressable. On le stocke dans
`agents.pane_id`.

Les trois commandes qui le prennent :

| Commande | Effet |
|---|---|
| `zellij action focus-pane-id <id>` | met le pane au premier plan |
| `zellij action rename-pane --pane-id <id> <nom>` | renomme **un** pane (sans `--pane-id`, c'est celui qui a le focus) |
| `zellij action close-pane --pane-id <id>` | le ferme |

Options utiles de `new-pane` : `--name`, `--cwd`, `--close-on-exit`,
`--floating`, `-b/--blocking` (attend la fin de la commande — à ne jamais
utiliser depuis le daemon).

## Un client qui ne joint pas son serveur attend, il n'échoue pas

Observé depuis un sous-processus qui héritait de `ZELLIJ` et
`ZELLIJ_SESSION_NAME` sans pouvoir joindre le socket du serveur :
`zellij action dump-layout` ne rend jamais la main (tué à 5 s, code 124).

C'est la raison d'être du délai dans `zellij.rs`. Sans lui, un pane qui
n'arrive pas gèlerait la tâche qui déroule les étapes d'un ticket — donc le
ticket. Toute commande zellij lancée par le daemon a une échéance de 3 s et son
échec ne coûte qu'une ligne de journal.

## `ZELLIJ` dit si on est dedans

Zellij exporte `ZELLIJ` (valeur `0`) et `ZELLIJ_SESSION_NAME` dans chaque pane
qu'il ouvre. On teste la présence de `ZELLIJ` avant de lancer quoi que ce soit :
sur une machine sans zellij, le module ne crée aucun processus.

Conséquence pratique : **le daemon doit être lancé depuis zellij** pour pouvoir
ouvrir des panes. Un daemon démarré par systemd ou depuis un autre terminal
n'héritera pas de ces variables, et `orchestra open` répondra qu'il n'y a pas de
session zellij.

## Dispositions

`~/.config/zellij/layouts/<nom>.kdl` (ou `$ZELLIJ_CONFIG_DIR/layouts`), chargée
par `zellij -l <nom>`. `orchestra init --zellij` y écrit la nôtre, nommée
d'après `zellij.layout` dans `config.toml` pour que les deux ne divergent pas.

# Protocole daemon ⇄ clients

Socket Unix, **un objet JSON par ligne**, UTF-8, ligne limitée à 4 Mio.

- Chemin : `$ORCHESTRA_SOCK`, sinon `$XDG_RUNTIME_DIR/orchestra.sock`, sinon
  `/tmp/orchestra-<uid>.sock`. Permissions `0600`.
- Un `flock` sur le fichier `.lock` voisin garantit un seul daemon.
- Version du protocole : `1`. Un client qui annonce une autre version est refusé.

## Séquence

```
client                              daemon
  │  ── connexion ─────────────────►│
  │◄── {"type":"hello", ...} ───────│   première ligne, toujours
  │  ── {"id":1,"cmd":"ping"} ─────►│
  │◄── {"id":1,"outcome":"ok","reply":"pong"}
```

## Requête

```json
{"id": 7, "cmd": "create_ticket", "project_id": "…", "title": "…", "brief": "…"}
```

`id` est choisi par le client et renvoyé tel quel. Le nom de la commande est aplati
dans l'objet, sous la clé `cmd`.

## Réponse

```json
{"id": 7, "outcome": "ok",  "reply": "tickets", "tickets": [...]}
{"id": 7, "outcome": "err", "error": {"code": "not_found", "message": "ticket introuvable"}}
```

Le discriminant de succès est `outcome`, pas `status` : `Reply` est aplati dans le
même objet et possède déjà un champ `status`. Un test parcourt toutes les variantes
pour empêcher ce genre de collision de revenir.

Codes d'erreur : `not_found`, `invalid`, `conflict`, `internal`, `unsupported`.

## Abonnement aux événements

```json
{"id": 2, "cmd": "subscribe", "filter": {"exclude_verbose": true}, "backlog": 50}
```

Le daemon renvoie `{"reply":"subscribed","current_seq":N}` puis pousse des trames
`{"type":"event", …}` sur la même connexion, mêlées aux réponses.

- `backlog` : nombre d'événements passés rejoués avant le direct.
- `since_seq` : reprise exacte après une coupure ; le client redonne le dernier `seq` vu.
- Si un client prend trop de retard, le daemon rejoue depuis la base ce qu'il a manqué
  plutôt que de laisser un trou.

Le filtre accepte `project_id`, `ticket_id`, `agent_id`, `todo_id`, une liste de `tags`, et
`exclude_verbose` qui écarte les événements bavards (texte d'agent, réflexion, outils).

## Événements

Un seul type, discriminé par `kind` : `daemon_started`, `project_added`,
`ticket_created`, `ticket_status_changed`, `proposal_ready`, `worktree_created`,
`agent_spawned`, `agent_status_changed`, `agent_text`, `agent_thinking`,
`tool_started`, `tool_finished`, `usage`, `agent_steered`, `agent_result`,
`hook_blocked`, `unmanaged_session_seen`, `warning`, `todo_added`, `todo_updated`,
`todo_status_changed`, `todo_deleted`, `todo_promoted`, `rule_proposed`,
`rule_created`, `rule_status_changed`, `rule_deleted`, `rules_checked`,
`merge_blocked`, et quelques autres.

Les rôles se lisent avec `list_roles` (`{"reply":"roles","roles":[…],"errors":[…]}`,
où `git` est toujours résolu : ce qu'un agent du rôle recevra). `create_role` répond
`{"reply":"role_file","path":"…"}`, `set_role_git` (`git`: `confined` ou `full`),
`delete_role` et `promote_role` réécrivent le fichier ; chacun publie `role_created`,
`role_updated` ou `role_deleted`.

`merge_blocked` (`branch`, `head`, `reason`) dit qu'une branche prête n'a pas pu
être fusionnée. Tant qu'il reste le dernier mot du ticket, `list_tickets` et
`get_ticket` renvoient sa raison dans `merge_blocked`, à côté de `pull_request`.

`launch_ticket` et `integrate_ticket` répondent `ack` dès les vérifications
rapides faites (équipe acceptée, transition permise, ticket pas déjà en route). Le
worktree se prépare ensuite, hors de la boucle du daemon : la suite arrive en
événements (`worktree_created`, `ticket_status_changed`, ou un `warning` si le
worktree n'a pas pu être préparé, le ticket restant alors prêt à lancer).

`get_diff` (`ticket_id`) répond `{"reply":"diff","diff":{branch, base, files:[{path,
added, removed}], patch, truncated}}` : ce que la branche du ticket apporte depuis
qu'elle a quitté la branche par défaut (`base...branche`), lu dans le dépôt
principal. `added` et `removed` valent `null` pour un fichier binaire ; le patch
est coupé en fin de ligne à 200 Kio (`truncated`).

**Épopées.** `create_epic` (`project_id`, `title`, `brief`) répond
`{"reply":"epic","detail":{epic, tickets}}` ; `plan_epic` (`epic_id`) répond `ack`,
et le découpage arrive en `epic_split_ready` (`epic_id`, `tickets`) ou
`epic_split_failed` (`epic_id`, `error`). `list_epics` (`project_id` facultatif)
répond `{"reply":"epics","epics":[…]}`, `get_epic` (`epic_id`) un `epic`.
`accept_epic` (`epic_id`, `proposal` : `{summary, tickets:[{title, brief,
depends_on:[positions], acceptance}]}`) crée les tickets dans une seule transaction,
publie `ticket_created` pour chacun puis `epic_accepted` (`epic_id`, `tickets`), et
répond l'épopée. `epic_finished` (`epic_id`) suit la fusion du dernier ticket.
Chaque carte de `list_tickets` porte `epic` (`epic_id`, `epic_title`, `waiting_on`
: les numéros des tickets pas encore fusionnés dont elle dépend), et
`launch_ticket` refuse un ticket dont une dépendance n'est pas fusionnée.

`agent_stalled` (`silent_secs`) signale un agent muet depuis `daemon.stall_secs`
secondes. Il n'est pas arrêté ; il redevient normal au premier mot.

Chaque carte de `list_tickets` porte aussi `attention` : ce que le ticket attend de
l'utilisateur, ou `null`. Valeurs, de la plus urgente à la moins urgente :
`merge_blocked`, `checks_failed`, `review_blocked`, `agent_stalled`,
`proposal_ready`, `ready_to_integrate`, `ready_to_launch`, `pull_request_open`. La
règle vit dans `orchestra-core/src/attention.rs` ; le daemon l'applique, les
clients se contentent de l'afficher.

Les règles (conventions et ADR) sont des fichiers, pas des lignes de la base :
`list_rules` les relit à chaque appel (`{"reply":"rules","rules":[…],"errors":[…]}`),
`create_rule` répond `{"reply":"rule_file","path":"…"}` pour qu'un client l'ouvre
dans un éditeur, et `set_rule_status`, `delete_rule`, `promote_rule` modifient le
fichier. Les événements, eux, sont persistés comme les autres. Dans ces commandes
comme dans les événements, la sorte s'appelle `rule_kind`, `kind` étant déjà le
discriminant des événements.

Chaque événement porte un `seq` monotone attribué par la base, un horodatage RFC 3339,
et la portée qui le concerne (`project_id`, `ticket_id`, `agent_id`, `todo_id`).

## Hooks

`orchestra-hook` envoie la charge utile du hook telle quelle :

```json
{"id": 1, "cmd": "hook", "agent_id": "…", "payload": { … JSON de Claude Code … }}
```

et lit la réponse `{"reply":"hook","allow":false,"reason":"…"}`. Seul un refus
explicite fait sortir le shim en code 2 ; toute autre situation, y compris un daemon
injoignable, laisse passer l'appel d'outil.

# Architecture

## Vue d'ensemble

```
        ┌──────────────┐        socket Unix, JSON par ligne
        │ orchestra tui│◄──────────────────────────────┐
        └──────────────┘                               │
        ┌──────────────┐                        ┌──────┴───────┐
        │ orchestra CLI│◄──────────────────────►│   daemon     │
        └──────────────┘                        │              │
        ┌──────────────┐   requête « hook »     │  ┌─────────┐ │
        │orchestra-hook│───────────────────────►│  │ SQLite  │ │
        └──────▲───────┘                        │  └─────────┘ │
               │ hooks                          └──────┬───────┘
        ┌──────┴───────┐   stdout stream-json          │ spawn
        │ claude -p    │◄──────────────────────────────┘
        │ (un par rôle)│
        └──────────────┘
               │ écrit
        ┌──────▼──────────────────┐   surveillé par le watcher
        │ ~/.claude/projects/*.jsonl│
        └─────────────────────────┘
```

Le daemon est **propriétaire unique** de l'état. Les connexions ne touchent jamais
la base : elles envoient une commande dans un canal mpsc avec un canal oneshot pour
la réponse. Il y a donc un seul écrivain, et pas de `Arc<Mutex<Daemon>>`.

## Les crates

### `orchestra-core`

Types du domaine, événements, protocole, grille tarifaire, configuration. **Aucune
dépendance d'I/O** : ni tokio, ni rusqlite, ni ratatui. Tout y est testable sans
toucher au disque ni au réseau.

- `model.rs` — `Project`, `Ticket`, `Team`, `Agent`, `Tokens`, `RoleDefinition`, et
  les règles qui les gardent : cycle de vie d'un ticket (`can_transition`), validation
  d'une proposition d'équipe et tri topologique en étapes (`Team::from_proposal`).
- `events.rs` — le type `Event` unique, son `EventTag` (la colonne indexée) et
  `EventFilter`.
- `protocol.rs` — `Command`, `Reply`, `Frame`, plus les vues (`TicketSummary`,
  `UsageRow`…). Une trame tient sur une ligne.
- `pricing.rs` — tokens vers dollars indicatifs, par plus long préfixe de modèle.
- `config.rs` — `config.toml` et tous les chemins XDG.

### `orchestra-daemon`

- `store/` — SQLite. Une connexion derrière un mutex, atteinte depuis l'async via
  `spawn_blocking`. `migrate.rs` applique les migrations numérotées ; `rows.rs` fait
  la correspondance SQL vers types.
- `bus.rs` — persiste **puis** diffuse. C'est cet ordre qui permet à un client de
  reprendre sans trou : tout événement reçu porte déjà son `seq`.
- `watcher.rs` — suit les transcripts de Claude Code pour compter **toutes** les
  sessions de la machine, pas seulement celles qu'Orchestra lance. Détection par
  sondage à la seconde plutôt que par inotify : avec une poignée de fichiers le coût
  est négligeable, et cela évite les limites de surveillance sur un dossier qui grossit
  à chaque session, ainsi qu'une dépendance encore en préversion. Pour un agent piloté,
  le chemin rapide reste sa propre sortie standard ; ce surveillant est le filet.
- `ledger.rs` — agrège et tarifie. L'agrégat SQL est toujours découpé par modèle,
  même quand l'affichage ne l'est pas : une ligne couvrant deux modèles coûte la somme
  de ses parties, jamais la moyenne de leurs tarifs. C'est ce qui fait que les lignes
  totalisent le total.
- `server.rs` — socket Unix, une tâche par connexion. Un `flock` garantit un seul
  daemon. Un client en retard est resynchronisé depuis la base.
- `daemon.rs` — la boucle qui traite les commandes.
- `orchestrator.rs` — la composition d'équipe. C'est elle-même un `claude -p` :
  outils en lecture seule, schéma JSON construit depuis le catalogue, budget borné.
  Elle propose ; l'utilisateur décide.
- `worker/` — le lancement et la lecture d'un processus `claude`. `claude.rs` est le
  seul endroit qui connaît la ligne de commande ; `translate.rs` transforme le flux en
  événements et masque au passage ce qui ressemble à un secret.
- `init.rs` — installe le catalogue de rôles et la configuration d'exemple, sans
  jamais écraser ce que l'utilisateur a modifié.
- `supervisor.rs` — la machine à états d'un ticket : un worktree, puis un processus
  `claude` par rôle, dans l'ordre des étapes, avec passage de relais entre eux.
  C'est lui qui décide du statut d'un agent : le flux dit ce que le processus a fait,
  pas ce que cela signifie. Une interruption ressemble à un échec dans le flux ; seul
  le superviseur sait qu'il a envoyé le signal.
- `worktree.rs` — création et adoption des worktrees git. Relancer un ticket
  retrouve le sien plutôt que d'échouer, et supprimer un worktree garde sa branche :
  c'est là qu'est le travail.
- `hooks.rs` — les réglages passés par session à un agent, et la reconnaissance
  d'un refus dans le flux.

### `orchestra-tui`

État `App` pur, façon Elm : `update(Msg) -> Vec<Command>`. Aucun type ratatui dans
l'état, donc toute la machine se teste sans terminal ; le rendu se teste avec
`TestBackend` en 80×24 et en pane étroit.

Les raccourcis évitent `Alt` et les accords `Ctrl+b` / `Ctrl+g`, réservés à zellij
par la configuration de l'utilisateur.

### `orchestra-hook`

Le garde-fou, appelé par Claude Code avant chaque appel d'outil d'un agent. Sortie 0
autorise, sortie 2 avec un motif sur stderr refuse et l'agent lit ce motif.

**Il décide seul.** Une première version l'aurait fait interroger le daemon par le
socket : cela aurait mis un aller-retour sur le chemin critique de chaque appel
d'outil et fait dépendre la sûreté d'un agent de la santé du daemon. Tout ce qu'il
faut tient dans son environnement et dans la charge utile du hook.

Il ne dépend que d'`orchestra-core`, pour les règles elles-mêmes : le daemon et lui
ne peuvent pas être en désaccord sur ce qui est permis. Trois propriétés priment sur
toute fonctionnalité : il démarre immédiatement, il ne bloque jamais, et un problème
d'Orchestra ne fait jamais échouer l'agent — tout imprévu autorise l'appel.

## Décisions

**Une seule interface.** La v1 imposait la parité entre un TUI et une application
Dioxus : chaque feature coûtait deux implémentations. Ici le daemon détient l'état,
et n'importe quelle interface future n'est qu'un client de plus sur le socket.

**Claude Code comme exécutant.** Pas de boucle LLM maison : on gagne les outils, les
permissions, les hooks, la reprise de session et le comptage de tokens sans les écrire.
Il n'existe pas de SDK Rust officiel, donc on pilote le CLI en sous-processus.

**SQLite dès le premier jour.** La v1 stockait en fichiers plats et n'a jamais pu
répondre à « combien m'a coûté cette feature ». Le coût est une table de première
classe, alimentée par deux sources réconciliées sur l'identifiant de message de l'API.

**Fusionner, pas ignorer.** Le plan prévoyait un `INSERT OR IGNORE` sur cet
identifiant. La lecture des vrais transcripts a montré que les copies d'une même
réponse ne sont pas identiques : les premières annoncent un `output_tokens` partiel.
Garder la première aurait sous-compté d'un facteur cent. Chaque champ garde donc le
maximum vu, ce qui est commutatif et idempotent. Détails et mesures dans
`docs/CLAUDE_CLI_NOTES.md`.

**Le schéma est construit, pas écrit.** Le champ `role` de la proposition porte une
énumération des rôles qui existent vraiment. Sans elle, le modèle invente des
intitulés plausibles au lieu de choisir dans le catalogue, ce qui s'est produit au
premier essai. La réponse est revalidée à l'arrivée : un schéma est une indication
forte, pas une garantie.

**Le superviseur tranche, pas le flux.** Une interruption produit une ligne `result`
en erreur, exactement comme un vrai échec. Le flux ne peut donc pas dire si un agent
a été annulé ou s'il a échoué ; seul celui qui a envoyé le signal le sait.

**Un appel unique ferme son entrée.** Avec `--input-format stream-json`, `claude`
attend d'autres tours de conversation ; sans fermeture de stdin, la planification
n'aurait jamais rendu la main. Les agents de la phase 3, eux, gardent cette entrée
ouverte pour recevoir des consignes.

**Le dépôt git prime sur le chemin.** Pour rattacher une session à un projet, la
racine du dépôt est consultée avant tout préfixe de chemin. Sans cela, une seule
session lancée dans le dossier personnel y crée un projet, et toutes les sessions
suivantes s'y rattachent par préfixe : tous les dépôts disparaissent dans une ligne.
Un dossier trop large (la racine, `/tmp`, le dossier personnel) ne devient jamais un
projet ; ses sessions sont comptées « hors projet ».

**Un worktree par ticket.** Un agent de la v1 a exécuté `mv *.md docs/` sur le dépôt
lui-même. Les agents ne voient plus que leur worktree, et un garde `PreToolUse` refuse
ce qui en sort.

## Base de données

Sept tables, décrites dans `crates/orchestra-daemon/src/store/migrations/0001_init.sql` :
`projects`, `tickets`, `agents`, `events`, `sessions`, `usage_samples`, `transcript_files`.

`events` est append-only et sa clé primaire auto-incrémentée sert de curseur aux clients.
`usage_samples` a pour clé primaire `message_id`, l'identifiant de la réponse API : le
flux d'un agent piloté et le transcript sur disque rapportent la même réponse, et le
transcript la répète une fois par bloc de contenu, avec des totaux partiels sur les
premiers blocs. L'écriture fusionne donc par maximum au lieu d'ignorer les doublons.

Les agrégats de coût sont de simples `GROUP BY` : rien n'est matérialisé, et
l'attribution (projet, ticket, agent) est dénormalisée à l'insertion pour que le
regroupement reste une requête sur une seule table.

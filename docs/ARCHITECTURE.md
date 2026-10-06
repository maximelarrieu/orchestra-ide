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
- `epic.rs` — les épopées : une demande découpée en tickets ordonnés, le découpage
  proposé (`EpicProposal`, validé en vagues par `waves`), son schéma pour
  `--json-schema`, et le lien d'un ticket à son épopée (`EpicLink`).
- `attention.rs` — ce qu'un ticket attend de l'utilisateur (`attention::of`),
  d'où la file « à toi » du tableau.
- `pricing.rs` — tokens vers dollars indicatifs, par plus long préfixe de modèle.
- `config.rs` — `config.toml` et tous les chemins XDG.
- `guard.rs` — les règles du garde-fou, dont `GitPolicy` : `Confined` par défaut,
  `Full` pour un rôle dont l'entête dit `git: full` — et, faute de mot, pour
  l'intégrateur seul (`GitPolicy::for_role`). La politique ne lève que la liste des
  sous-commandes interdites ; le confinement des chemins vaut pour tous.
  Les règles sont pures ; la seule question qu'elles ne peuvent pas trancher —
  « ce chemin est-il écrivable ? » — est posée par `Boundary::reach`, fournie par
  l'appelant. Sans réponse, tout est réputé écrivable (lecture stricte).
- `review.rs` — la relecture : ajout du relecteur en fin de proposition, et lecture
  du verdict qu'il écrit (`VERDICT: prêt` / `VERDICT: corrections` suivi des points
  bloquants). Le silence n'y vaut jamais accord.

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
  **Il ne crée jamais de projet** : une session est rattachée à un projet que
  l'utilisateur a ajouté (par la racine git d'abord, puis par le chemin), et sinon
  comptée sans projet.
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
- `checks.rs` — les vérifications du dépôt, lancées par le daemon dans le
  worktree : quelles commandes, et ce qu'elles ont rendu.
- Les notes de ticket (`.orchestra-ticket.md`, `worktree::NOTES_FILE`) sont la
  mémoire de l'équipe sur un ticket : écrites au lancement (brief, équipe, critères,
  points d'attention), lues et complétées par chaque agent, exclues de git par
  `info/exclude` du dépôt — jamais commitées. Les passages de relais en restent le
  résumé ; le plan et les décisions vivent là, en entier.
- `daemon/epics.rs` — les commandes des épopées ; `epic_flow.rs` les fait avancer :
  à la fusion d'un ticket, ceux qu'il libère sont planifiés (`epic.auto_plan`, une
  proposition seulement), et l'épopée se termine avec son dernier ticket. Le lien
  ticket ↔ épopée vit dans `epic_tickets` (migration 0003), pas sur `tickets`.
- `attention.rs` — ce qu'un ticket attend de l'utilisateur, lu depuis la base (la
  règle est dans `orchestra-core`), et le veilleur qui notifie le bureau quand un
  ticket se met à attendre (`notify.attention`). `notify.rs` envoie, sans jamais
  échouer.
- `zellij.rs` — les panes. Tout y est best-effort et sous échéance ; rien n'y
  peut faire échouer un ticket. Faits sur le CLI dans `docs/ZELLIJ_NOTES.md`.

### `orchestra-tui`

État `App` pur, façon Elm : `update(Msg) -> Vec<Command>`. Aucun type ratatui dans
l'état, donc toute la machine se teste sans terminal ; le rendu se teste avec
`TestBackend` (`screens::text_of`) en 80×24 et en pane étroit, et les écrans
principaux sont figés en snapshots `insta` (`tests/snapshots/`).

`app/` répartit les méthodes d'`App` par préoccupation : `input` (la cascade des
touches), `nav` (écrans, colonnes, lignes, file « à toi »), un fichier par écran
pour ses touches, `palette`, `replies` (réponses et événements du daemon),
`describe` (les événements en mots). `screens/layout.rs` dessine le cadre commun ;
chaque `screens/<écran>.rs` ne dessine que son contenu.

**Une grammaire de touches** : `y` accepte, `x` rejette ou arrête, `e` édite, `o`
ouvre, `d` supprime ; une confirmation ne se donne qu'avec `y` ou Entrée. Un test
essaie toutes les lettres sur plusieurs écrans : une touche qui agit sans être
écrite sur la barre ni dans l'aide le fait échouer.

Les raccourcis évitent `Alt` et les accords `Ctrl+b` / `Ctrl+g`, réservés à zellij
par la configuration de l'utilisateur.

**Ce que font les touches est écrit une seule fois** (`keys.rs`). `keymap.rs` dit
quelle touche produit quelle action ; `keys.rs` dit ce que l'écran courant sait
faire, et le bandeau du bas comme l'aide lisent cette même liste : une commande que
l'écran n'offre pas ne s'affiche jamais, et une nouvelle commande ne peut pas être
oubliée d'un des deux endroits. Deux rangs seulement — ce que cet écran est seul à
savoir faire, en avant ; ce qui marche partout, estompé derrière une barre — et la
touche est entre crochets pour qu'on voie où elle finit. Quand la place manque, ce
sont les touches communes qui sautent d'abord, puisque l'aide les redit ; « ? » est
la dernière à partir, et le rendu lui garde sa place. Un champ de saisie ouvert
n'annonce que ses propres touches : `?` et `q` s'y écriraient dans le texte.

**La couleur ne porte jamais une information seule** (`theme.rs`). Chaque statut a
son symbole en plus de sa teinte, et la palette n'oppose pas le rouge au vert, qui
est la paire que beaucoup de gens ne distinguent pas : le bleu, le cyan et le jaune
font le travail, le rouge n'apparaît qu'avec une croix et le vert qu'avec une coche.
Seules les seize couleurs du terminal sont utilisées, pour suivre le thème déjà en
place plutôt que le combattre.

**L'écran Agent se synchronise tout seul.** Ouvert depuis la barre d'onglets sans
ticket, il cherche l'agent qui tourne où qu'il soit ; ouvert depuis un ticket, il
prend celui qui travaille plutôt que la première ligne, et suit l'équipe quand un
rôle passe la main. Le journal arrive par le flux, mais le statut et le coût
viennent du ticket, relu à chaque seconde : sans cela l'en-tête restait sur
« démarrage » pendant que l'agent travaillait visiblement.

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

**Il demande au système de fichiers ce qui est écrivable.** Une commande shell est
pleine de choses qui ressemblent à un chemin sans en être : la route `/habitudes`
passée à un outil de capture, la regex `/_next/static/[a-z]+\.js`, le script sed
`/motif/d`. Chacune a bloqué du vrai travail. Ce qui les distingue de
`/home/u/projet/.env`, ce n'est pas leur forme mais `access(2)` : un utilisateur
ordinaire ne peut rien écrire sous `/`, donc rien là n'est une sortie du worktree.
Le hook remonte jusqu'au premier ancêtre existant et regarde s'il est écrivable ;
sous `sudo`, la réponse ne veut plus rien dire et la lecture stricte reprend. Les
outils `Write`/`Edit` et `git -C` gardent la lecture stricte : là, le chemin n'est
pas ambigu. Le répertoire `~/.cache` est de l'espace de brouillon, comme `/tmp`.

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

**Ce qui se mesure ne se demande pas.** La relecture est un agent : ce qu'elle
dit des tests est un rapport, et rien ne distingue un rapport honnête d'un
rapport pressé. Un code de sortie, si. Le daemon lance donc lui-même les
vérifications du dépôt dans le worktree du ticket (`daemon/src/checks.rs`, règles
dans `core/src/checks.rs`), **juste avant chaque passage du relecteur** — jamais
après : lire une branche qui ne compile pas coûte une heure d'agent à
`effort: high` pour ce que `cargo build` dit en vingt secondes. L'invariant tient
en une phrase : avant tout verdict, une mesure. Donc un ticket qui arrive en
« à relire » est un ticket dont la dernière passe était verte.

Un rouge renvoie au travail le dernier rôle qui a écrit du code, avec la sortie
de la commande plutôt qu'un résumé — un compilateur dit mieux que nous ce qu'il
refuse. Toujours rouge une fois `checks.max_rounds` épuisé, le ticket **échoue**,
ce qui est exactement ce qui s'est passé : l'équipe n'a pas livré quelque chose
qui tient. Il se relance, et la relance garde le travail déjà dans la branche.

La commande vient de `checks.commands` ou, à défaut, du **dépôt principal** —
jamais de la branche : une branche qui choisit son examinateur n'est pas
examinée. Ce qui s'exécute ensuite est bien le code de la branche, et il ne peut
pas en être autrement puisque des tests sont du code ; c'était déjà vrai quand le
relecteur les lançait depuis son shell. Ce que la porte retire, c'est la
possibilité d'annoncer un vert sans l'avoir obtenu. Rien ne passe par un shell :
la commande est découpée à l'avance, donc un `;` dans la configuration reste un
argument. Le résultat est un événement comme le reste — `CheckStarted`,
`CheckFinished` — et c'est de ces événements que se relit la dernière passe :
pas de colonne de plus à tenir en phase. L'intégration le vérifie côté daemon,
pas seulement dans l'écran, et le relecteur reçoit le résultat en annexe de son
prompt pour qu'il n'ait ni à le refaire ni à nous croire sur parole.

**Toute équipe finit par une relecture.** Le relecteur n'est pas laissé au jugement
de l'orchestrateur : le daemon l'ajoute à la fin de chaque proposition, où
l'utilisateur peut encore l'enlever avant d'accepter. Son verdict est lu par la
machine : bloquant, il renvoie au travail les rôles qu'il nomme — un point bloquant
qui ne nomme personne va au dernier rôle qui a écrit du code, parce qu'il a le
contexte le plus frais — puis il relit. La boucle est bornée par
`review.max_rounds` et par le verdict lui-même : une relecture qui ne termine pas
par un verdict lisible arrête tout et rend le ticket en « à relire ». Un tour de
correction est un nouvel agent, pas une reprise de session : il apparaît sur le
ticket sous le même rôle, suffixé « reprise N ».

**Le tableau est un kanban.** Une ligne par ticket disait le statut ; une colonne par
étape dit *où en est le travail* — à faire, en cours, à relire, PR à valider, fusionné.
« À relire » et « PR à valider » sont le même statut vu de deux endroits : ce qui
attend tes yeux ici, ce qui attend ton clic sur GitHub. Les deux colonnes de bord —
la PR et les tickets arrêtés — n'apparaissent que si elles portent quelque chose, pour
qu'un projet qui n'ouvre jamais de requête ne traîne pas une colonne vide. `h`/`l`
changent de colonne, `j`/`k` descendent dedans, et la colonne du curseur est celle qui
porte la marque de focus. Le volet des projets est un sélecteur, pas une vue : il a la
largeur d'un nom, le reste va au tableau.

**Un ticket fermé se rouvre.** Fusionné en local alors qu'on le voulait en pull
request, annulé trop vite : `Done | Cancelled → Review` existe, parce que le travail
est dans la branche et que le verdict est dans les événements — seule la décision est
reprise. Le worktree, lui, est recréé depuis la branche : un worktree est une copie de
travail, pas le travail, et le supprimer à la fermeture ne doit pas fermer la porte.
Un ticket en cours ne se rouvre pas, et un ticket échoué se relance.

**Une pull request a une forme, et c'est l'agent qui l'écrit.** Le squelette est une
convention livrée (`assets/conventions/pr-template.md`, `applies_to: [integrator]`,
`mode: pr`) : il n'arrive qu'à l'intégrateur, et seulement quand une requête est ce
qui sortira du run. L'agent écrit `PR.md` dans son worktree — cinq sections, dans
cet ordre, sans en ajouter, dont trois que le daemon vérifie — et le daemon y
accroche le pied de page qu'il est seul à connaître : ticket, branche, verdict de la
relecture, tokens et coût. Ce partage n'est pas cosmétique : la substance vient de
celui qui a lu la branche, les faits de celui qui les a mesurés, et personne n'a à
croire un agent sur les chiffres. Sans `PR.md`, une description est assemblée depuis
le brief et le dernier passage de relais, avec les mêmes titres de section : moins
bonne, jamais absente, et toujours à la même forme.

**Ou bien la branche part en pull request.** `integration.mode = "pr"` remplace la
fusion locale : le daemon pousse la branche et ouvre une requête avec `gh` — déjà
installé, déjà authentifié, aucun jeton à garder ici. Le ticket reste alors « à
relire » avec le lien, parce que le travail n'est pas dans la branche par défaut tant
que personne n'a cliqué. Une tâche interroge les requêtes ouvertes une fois par
minute : fusionnée, le ticket se termine, le worktree est nettoyé et la branche par
défaut locale rattrape le remote ; fermée sans fusion, le ticket est annulé et la
branche reste. Un `gh` muet — hors ligne, quota — ne décide de rien : la requête
reste ouverte jusqu'à ce qu'on puisse lire son état. Un projet sans remote retombe
sur la fusion locale plutôt que de s'arrêter.

**Pousser reste une décision.** `integration.push` est à faux par défaut et couvre les
deux côtés : l'intégrateur envoie sa branche, et le daemon envoie la branche par
défaut une fois la fusion faite — sans quoi le travail s'arrêtait sur la machine, la
branche principale en avance sur son remote sans que rien ne le dise. Un dépôt sans
remote n'est pas une erreur, et un push raté ne défait pas la fusion : il est signalé,
la branche par défaut est à jour en local.

**Les agents ont des habitudes, écrites et vérifiées.** Un rôle dit à quoi sert un
agent ; une *règle* dit comment on fait ici. Deux sortes, un seul format — Markdown
à entête YAML, comme les rôles (`orchestra-core/src/conventions.rs`) :

- une **convention** est une habitude attachée à des rôles (`applies_to`, vide pour
  tous) : le squelette de PR, la forme d'un commit, « un défaut corrigé arrive avec
  son test ». Les globales vivent dans `~/.config/orchestra/conventions`, un projet
  les remplace par nom dans `.orchestra/conventions` — y compris pour en rejeter
  une chez lui ;
- un **ADR** est une décision d'architecture du projet (`.orchestra/adr/NNNN-*.md`),
  donnée à tous ses agents et résumée à l'orchestrateur quand il compose l'équipe.

Chaque agent reçoit, sous son rôle et le pied de page commun, les conventions
acceptées qui le visent et tous les ADR acceptés. Le fichier est relu à chaque
agent : une règle acceptée pendant un ticket vaut dès l'agent suivant. Tant que
`~/.config/orchestra/conventions` n'existe pas, le daemon lit les conventions livrées
dans le binaire — une installation antérieure ne perd pas le squelette de PR ; un
dossier présent, même vide, est la volonté de l'utilisateur.

Ce qui se mesure est vérifié : une convention peut porter des `checks`
(`commit_message`, une expression régulière sur la première ligne de chaque commit
de la branche, merges exclus ; `pr_sections`, les titres que `PR.md` doit avoir). Le
daemon les évalue après l'intégrateur et avant la fusion ou la pull request
(`RulesChecked`). Un écart renvoie l'intégrateur — le rôle dont c'est le métier de
reformuler un historique — au plus `checks.max_rounds` fois ;
au-delà, ni fusion ni requête.

Les agents proposent, l'humain décide. Un bloc `PROPOSITION: convention|adr` à la
fin d'un message est lu par la machine, comme le verdict, et écrit **par le daemon**
dans `.orchestra/` avec `status: proposed` (`RuleProposed`). Une règle proposée n'est
ni donnée ni vérifiée ; elle attend sur l'écran 8, où `a` l'accepte, `r` la rejette
et `e` l'ouvre dans `$EDITOR`. Une règle écrite à la main commence elle aussi
`proposed` : son squelette ne doit atteindre personne avant d'avoir été rédigé.

**Un ticket interrompu se reprend, il ne se refait pas.** Les agents meurent avec le
daemon qui les a lancés, et la tâche qui déroulait les étapes aussi. Au démarrage,
les agents restés « en cours » sont marqués plantés et **le ticket qui les portait
passe en « échoué »** : laissé « en cours », il attendait un exécutant qui ne
reviendrait jamais, et `launch` le refusait puisqu'il tournait déjà. Relancé, il
reprend : un rôle qui a un agent terminé avec un passage de relais est sauté, son
travail étant déjà dans la branche, et un tour de correction déjà effectué avant la
coupure n'est pas rejoué — on enchaîne sur la relecture qui manquait. Les tours de
correction déjà dépensés sont recomptés depuis les verdicts enregistrés : une reprise
ne rend pas son budget.

**L'écran interroge, il n'attend pas.** La boucle reçoit un tic par seconde ; elle ne
faisait que redessiner avec lui, sans le passer à l'application. Donc rien n'était
interrogé : le tableau, le ticket ouvert et les compteurs ne bougeaient que si un
événement passait par là, et un statut changé ailleurs ne se voyait qu'en quittant
l'écran pour y revenir. Le tic va maintenant à l'application, qui interroge ce que
l'écran montre chaque seconde et l'agrégat de coût toutes les deux. Et une connexion
perdue est reprise toute seule au tic suivant, sans relancer de daemon.

**Une attente doit se voir.** La planification est une minute de silence : le run lit
le dépôt avant de répondre. L'écran Ticket s'abonne donc aux événements du ticket,
verbeux compris, et montre ce que l'orchestrateur lit, cherche et pèse, avec le temps
écoulé et les tokens dépensés. Et ce n'est plus un rafraîchissement qui décide que la
réflexion est finie — il l'effaçait une seconde après la touche — mais l'agent
orchestrateur lui-même : actif, il pense ; terminé après notre demande, c'est fini.

**Git suit le rôle, et jamais dans le dépôt principal.** L'intégrateur est
lancé à la main depuis l'écran Ticket, et seulement si la relecture n'a rien bloqué.
Son garde-fou ouvre les sous-commandes git (`GitPolicy::Full`) mais **pas** les
chemins : `git -C <dépôt principal>` reste refusé comme n'importe quelle sortie de
boîte. Il travaille donc dans son worktree — il rapatrie la branche par défaut chez
lui, règle les conflits, rejoue les vérifications — et c'est le daemon qui fait
ensuite la fusion, en `--ff-only`, dans un dépôt principal qu'il exige propre et sur
sa branche par défaut. Une intégration ratée laisse l'historique principal intact.
La règle 5 tient donc sans exception : aucun agent n'a jamais de shell dans le dépôt.
La politique suit le rôle, pas l'étape : elle est lue dans l'entête du rôle (`git:
confined | full`) à chaque agent lancé (`Supervisor::git_for`). Un intégrateur que
l'orchestrateur a placé dans une équipe garde donc `Full` — confiné, il se voyait
refuser le push que son objectif lui demandait —, et l'utilisateur peut ouvrir git à
un autre rôle, ou le fermer à l'intégrateur. Sans `git:` écrit, seul le rôle nommé
par `config.integration.role` l'a : un catalogue installé avant ce champ garde son
comportement.

**Rôles et règles, un seul écran.** Les rôles sont des fichiers comme les
conventions, et répondent à la même question : comment l'équipe travaille ici.
L'écran 8 les liste en tête, avant conventions et ADR, avec leurs droits git en
toutes lettres (et `⎇` pour un rôle ouvert). `e` ouvre le fichier dans `$EDITOR` —
nom, description, modèle, outils, consignes —, `p` ouvre ou referme git (demandé
avant d'ouvrir : un push sort de la machine), `g` rend global un rôle de projet,
`d` le supprime, et `:role add <nom>` en crée un depuis un squelette confiné. Un rôle
n'a pas d'état « proposé » : seul l'humain en écrit. Le daemon refuse de supprimer
le dernier fichier d'un rôle dont il a besoin (relecture, intégration), et chaque
écriture publie `RoleCreated`, `RoleUpdated` ou `RoleDeleted` pour que les autres
clients relisent le catalogue.

**Une fusion refusée attend l'utilisateur, et le dit.** Quand la branche est prête
mais que le daemon ne peut pas la fusionner (dépôt principal modifié, autre branche
sortie, branche par défaut qui a bougé), il publie `MergeBlocked` avec la raison et
le commit prêt — pas un simple avertissement perdu dans le flux. Tant qu'aucun
événement plus récent ne fait avancer le ticket, sa carte porte « ⏸ fusion en
attente » et l'écran Ticket en donne la raison (`daemon/src/integration.rs`).
Réessayer (`f`) ne relance pas d'intégrateur si la branche pointe toujours sur ce
commit et que l'avance rapide reste possible : seule la porte est rejouée. Si la
branche par défaut a bougé, il faut quelqu'un pour la ramener, et l'intégrateur
repart.

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

**Un pane par agent, et une porte de sortie.** Le multiplexeur est déjà là ; ce
qui manquait, c'est une fenêtre par agent. `zellij action new-pane` rend
l'identifiant du pane qu'il vient d'ouvrir (`terminal_<n>`) : on le garde dans
`agents.pane_id`, et c'est ce qui permet ensuite de le remettre au premier plan
plutôt que d'en ouvrir un deuxième sur le même agent. Le pane fait tourner
`orchestra tail <id>` — **pas** un `claude` interactif : un `claude` lancé à la
main n'a pas de sortie structurée, donc ni steering, ni annulation, ni comptage
par le daemon. Le pane est une vue ; le daemon garde la main. À la fin d'un
agent, son pane est renommé avec un symbole (`✓`, `✗`, `⊘`) : un mur de panes se
lit à un mètre, et c'est exactement la distance où la couleur cesse d'être
lisible.

La porte de sortie est `T`. La session Claude de l'agent existe toujours, donc
`claude --resume <session_id>` la rouvre entière dans un pane, dans le worktree
du ticket : l'agent passe en « manuel », le daemon cesse de le piloter, et le
watcher continue de compter ses tokens puisque l'identifiant de session n'a pas
bougé. Réservée à un agent arrêté : deux mains sur une même session se défont
l'une l'autre.

Tout cela est sous échéance. Un `zellij action` dont le client ne joint pas son
serveur **n'échoue pas, il attend** — mesuré : `dump-layout` tué à 5 s. Sans
délai, un pane qui n'arrive pas gèlerait la tâche qui déroule les étapes du
ticket, donc le ticket. Chaque appel a trois secondes et son échec ne coûte
qu'une ligne de journal. Et rien n'est tenté hors d'une session zellij, ce qui
se lit dans l'environnement (`ZELLIJ`) plutôt qu'en essayant : sur une machine
sans zellij, le module ne lance aucun processus.

**Le premier commit d'un projet neuf est le nôtre.** `git init` laisse HEAD sur une
branche qui n'existe pas encore : rien ne peut en partir, et le tout premier ticket
d'un projet — celui qui l'initialise — mourait sur `fatal: invalid reference: main`.
Le daemon crée donc un commit vide « init » quand le dépôt n'en a aucun, et le dit
dans le flux. C'est le seul commit qu'Orchestra écrit hors d'un worktree ; tout le
reste s'y passe.

**Un worktree par ticket.** Un agent de la v1 a exécuté `mv *.md docs/` sur le dépôt
lui-même. Les agents ne voient plus que leur worktree, et un garde `PreToolUse` refuse
ce qui en sort.

## Base de données

Huit tables. Sept viennent de `crates/orchestra-daemon/src/store/migrations/0001_init.sql` :
`projects`, `tickets`, `agents`, `events`, `sessions`, `usage_samples`, `transcript_files`.
`0002_todos.sql` ajoute `todos` — idées et tâches personnelles, rattachées à aucun
projet tant qu'elles ne sont pas promues en ticket — et une colonne de portée
`todo_id` sur `events`.

`events` est append-only et sa clé primaire auto-incrémentée sert de curseur aux clients.
`usage_samples` a pour clé primaire `message_id`, l'identifiant de la réponse API : le
flux d'un agent piloté et le transcript sur disque rapportent la même réponse, et le
transcript la répète une fois par bloc de contenu, avec des totaux partiels sur les
premiers blocs. L'écriture fusionne donc par maximum au lieu d'ignorer les doublons.

Les agrégats de coût sont de simples `GROUP BY` : rien n'est matérialisé, et
l'attribution (projet, ticket, agent) est dénormalisée à l'insertion pour que le
regroupement reste une requête sur une seule table.

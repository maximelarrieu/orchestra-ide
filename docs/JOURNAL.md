# Journal de bord — Orchestra IDE

> Avancement par phase : ce qui a été livré, les limites connues, ce qui suit. Mis à jour
> à chaque phase. Détails techniques dans [`ARCHITECTURE.md`](./ARCHITECTURE.md), vision
> produit dans [`FONCTIONNEL.md`](./FONCTIONNEL.md).

Plan global en 5 phases (+ évolutions post-Phase 5 ci-dessous) :

1. Modèle des Espaces de Contexte + coquille du dashboard ✅
2. Commande `orchestra init` (scaffolding interactif) ✅
3. Runtime d'agents + flux temps réel → radar vivant ✅
4a. Intégration LLM (Claude **ou** Gemini) + Skills Dev exécutables (tool use) ✅
4b. Intégrations Git (local) + GitHub (REST) ✅
4c. Intégration Jira ⏳ (optionnel)
5. Agent Documentaliste (Mermaid) + finitions ✅

---

## Phase 1 — Modèle + dashboard ✅

**Livré**
- Workspace Cargo à deux crates, posant le découplage strict cœur (`orchestra-core`) / UI
  (`orchestra-tui`).
- Modèle agnostique : `ContextSpace`, `ProjectConfig`, `ProjectType`, `Integrations`,
  `Adr`. Chargement depuis `.orchestra/config.json`.
- Contrat d'événements `AgentEvent` figé d'emblée (consommé plus tard par l'UI).
- Coquille du tableau de bord en 3 zones (en-tête / radar / menu).

**Limites connues**
- Radar vide (aucun agent), menu non interactif hormis `q`.

---

## Phase 2 — `orchestra init` ✅

**Livré**
- `orchestra-core::scaffold` : `InitOptions` + `scaffold_space` (logique pure) qui génère
  `.orchestra/{config.json, persona.md, adr/}`, avec garde anti-écrasement
  (`SpaceAlreadyExists`).
- Matrice `default_agents()` (en complément de `default_skills()`).
- `orchestra-tui::wizard` : assistant interactif stdin (nom, type, workspace pour Dev,
  documentaliste).
- Dispatch CLI : `init` | dashboard | `--help`.
- Gabarits de `persona.md` propres à chaque type de projet.

**Limites connues**
- L'assistant ne configure pas encore les intégrations (Git/GitHub/Jira).

---

## Phase 3 — Runtime d'agents, radar vivant ✅

**Livré**
- `orchestra-core::runtime::spawn` : un agent = une tâche `tokio`, événements publiés sur
  un canal `tokio::sync::mpsc` ; fermeture du canal = orchestre au repos.
- `AgentEvent::Started` ajouté ; helper `AgentEvent::agent()`.
- `orchestra-tui::app::App` : agrégation du flux (compteurs, historique borné, phases
  `Idle`/`Running`/`Finished`), isolée du rendu et testée.
- Boucle async `tokio::select!` (clavier via `EventStream` + flux agents + tick) ;
  touche `[1]` lance l'orchestre, radar défilant stylé par type d'événement.
- Test de rendu **headless** via `ratatui::backend::TestBackend`.

**Limites connues**
- ⚠️ **Agents simulés, aucun LLM** : le flux est scripté pour valider la chaîne temps
  réel. Aucun travail réel n'est effectué.
- Touches `[2]` (ADRs) et `[3]` (changer d'Espace) encore inactives.

**Tests** : 9 verts (`cargo test --workspace`), `clippy` sans warning.

---

## Phase 4a — LLM Claude + Skills Dev exécutables ✅

**Livré**
- `orchestra-core::llm` : client **multi-fournisseurs** en HTTP brut (`reqwest`, rustls) —
  **Claude** (`claude-opus-4-8`) ou **Gemini** (`gemini-2.5-flash`) au choix, via une
  représentation neutre (`Msg`/`Block`/`ToolSpec`). Sélection par `ORCHESTRA_PROVIDER` ou
  auto-détection de la clé (`ANTHROPIC_API_KEY` / `GEMINI_API_KEY`) ; modèle surchargé par
  `ORCHESTRA_MODEL`.
- `orchestra-core::skills` : trois Skills Dev exécutables via tool use — `Read_File`,
  `Write_File_Validated`, `Execute_Terminal_Command` — confinés au workspace (chemins
  absolus/`..` refusés ; commande shell avec délai 30 s et sortie plafonnée).
- `runtime` : boucle agentique réelle (Claude ↔ outils, max 6 tours) **sans changer la
  signature de `spawn`** ; **repli automatique** sur le flux simulé sans clé ou si l'API
  échoue.
- TUI : indicateur de mode dans l'en-tête (`🤖 <modèle>` / `simulé · clé API absente`) et
  rappel sur le radar des variables d'environnement à définir pour activer un vrai LLM.

**Limites connues**
- Pas d'intention saisie par l'utilisateur : chaque agent part d'un objectif générique
  dérivé du persona (saisie interactive prévue plus tard).
- Skills exécutables limités au triptyque Dev ; les autres types restent « parlants ».
- Intégrations Git / GitHub / Jira non encore implémentées (Phase 4b).

**Tests** : 14 verts (`cargo test --workspace`) — dont skills (round-trip fichier, garde
anti-évasion, exécution shell) et runtime hors-ligne ; `clippy` sans warning. La vraie
boucle LLM nécessite une clé API (testée en local).

## Phase 4b — Intégrations Git + GitHub ✅

**Livré**
- `orchestra-core::integrations` : Skills d'intégration exposés au LLM **uniquement si
  configurés** dans `config.integrations`.
  - Git local : `Git_Status`, `Git_Diff`, `Git_Create_Branch`, `Git_Commit` (binaire `git`,
    workspace, délai/plafond ; nom de branche validé).
  - GitHub REST : `GitHub_List_Issues`, `GitHub_Create_Issue_Comment`,
    `GitHub_Create_Pull_Request` (token via `token_env_var`, jamais en dur ; exposés
    seulement si le token est présent).
- `runtime` : fusion des outils Dev + intégrations, dispatch par `integrations::handles`.

**Limites connues**
- Actions sortantes (PR/commentaire) et modifiantes (commit/branche) sans confirmation
  interactive (human-in-the-loop prévu plus tard) — autorisation par la config de l'espace.
- L'assistant `init` ne configure pas encore les intégrations (édition manuelle de
  `config.json`).

**Tests** : 21 verts (`cargo test --workspace`) — dont Git réel sur dépôt temporaire
(`status`, création de branche), exposition conditionnelle des Skills, validation de nom de
branche ; `clippy` sans warning. GitHub REST testé en local (token requis).

## Phase 5 — Agent Documentaliste + finitions ✅

**Livré**
- Agent Documentaliste : activé par `documentalist_enabled`, rejoint l'orchestre avec un
  prompt et un jeu d'outils dédiés (`Read_File`, `Write_File_Validated`,
  `Write_Mermaid_Diagram`), indépendants de la liste de Skills du projet.
- `skills::Write_Mermaid_Diagram` : écrit un `.md` avec un bloc ` ```mermaid ` ; type de
  diagramme validé (graph/sequenceDiagram/classDiagram…), confiné au workspace.
- Finitions dashboard : `[2]` bascule radar ↔ liste des ADRs ; `[3]` change d'Espace via
  une saisie de chemin (chargement à l'`Entrée`, annulation à `Échap`, message de
  succès/erreur).

**Limites connues**
- Pas de human-in-the-loop sur les actions des agents (autorisation par la config).
- L'assistant `init` ne propose toujours pas les intégrations ni le Documentaliste.

**Tests** : 27 verts (`cargo test --workspace`) — dont Skill Mermaid (validation + écriture),
Documentaliste ajouté quand activé, bascule de vue ADRs, édition/consommation de la saisie,
rendus headless (ADRs + mode saisie). `clippy` sans warning.

## Conversation avec un coordinateur (post-Phase 5) ✅

- **Mode conversationnel** (`[5]`) en plus de l'exécution autonome (`[1]`) :
  `runtime::start_conversation` ouvre une `ChatHandle { user, events }` (canal mpsc
  **bidirectionnel**) ; une tâche tokio tient la boucle et conserve l'historique entre les
  messages.
- **Pattern « agent-outil » / coordinateur** : chaque agent du roster est exposé au chef
  d'orchestre comme un outil (`delegation_tool`) ; il délègue via `run_subagent` (qui
  réutilise `run_agent_turn`, mutualisé avec le mode autonome), voit l'activité du sous-agent
  défiler sur le radar, puis synthétise. Le coordinateur peut aussi poser des questions à
  l'utilisateur.
- TUI : ligne de saisie de chat (`›`), indicateur `💬 conversation`, `Entrée` envoie, `Échap`
  ferme le canal et termine la conversation.
- Refactor : la boucle agentique d'un tour est extraite (`run_agent_turn`) et partagée entre
  le mode autonome et les sous-agents du coordinateur.
- **Correctif d'affichage** : le radar ne tronquait plus que la 1ʳᵉ ligne (≤200 car.) de
  chaque réponse — il déroule désormais le **texte complet avec retour à la ligne**
  (`emit_log` conserve le multi-ligne ; rendu via `wrap_plain`), et colore distinctement
  « Vous » (vert) / « Coordinateur » (magenta) / agents (cyan).

## Radar : défilement + rendu Markdown (post-Phase 5) ✅

- **Défilement du radar** : `PgUp`/`PgDn` (et `↑`/`↓`) remontent dans l'historique
  (`App::radar_scroll`), retour automatique en bas à chaque nouveau message ; titre du radar
  qui indique le défilement.
- **Rendu Markdown dans la conversation** : les messages sont stylés bloc par bloc
  (`markdown::styled_blocks` — titres, listes, citations, code) puis repliés à la largeur
  (`wrap_plain`). Le visualiseur plein écran `[2]` conserve en plus le style en ligne.
- **Indicateur d'activité** : un événement `AgentEvent::Thinking` est émis avant chaque
  appel LLM (coordinateur et sous-agents) ; l'UI affiche un **spinner animé** avec **temps
  écoulé** « ⠋ {agent} réfléchit… {n}s » (en-tête + bas du flux), effacé dès qu'une sortie
  arrive. On voit ainsi qui « mouline » en arrière-plan et depuis combien de temps.
- **Saisie de chat multi-ligne** : Maj/Alt+Entrée insère un retour à la ligne, Entrée
  envoie ; la zone de saisie grandit dynamiquement. Activation best-effort des
  *keyboard enhancement flags* (crossterm) pour distinguer Maj+Entrée sur les terminaux
  compatibles.

## Disposition « cockpit » + orchestre live (post-Phase 5) ✅

- Le dashboard passe en **multi-panneaux** : sidebar « 🎻 Orchestre » (gauche) toujours
  visible + zone centrale + barre de saisie. La sidebar se masque sous ~60 colonnes.
- **Statut live par agent** (`App::agent_status` : Idle/Thinking/Working/Done) dérivé des
  événements ; icône par agent (`○` / spinner / `▸` / `✔`), réinitialisé à chaque run.
- La zone centrale conserve le système de vues existant (radar/docs/agents/éditeur), rendu
  désormais dans le panneau central plutôt qu'en plein écran.

## Mémoire partagée d'espace + prompt caching (post-Phase 5) ✅

- **Mémoire partagée** : module `memory` (`.orchestra/memory.md`) + primitives universelles
  `Remember{note}` / `Recall{query?}` exposées à **tous** les agents (dispatch `memory::handles`
  dans le runtime, aiguillé comme les intégrations). Notes numérotées + attribuées à l'agent,
  durables entre sessions, listées dans le navigateur `[2]`.
- **Token-smart** : le prompt système n'injecte qu'un rappel court ; le contenu se lit à la
  demande via `Recall` (filtre par mot-clé). Une source résumée une fois est relue en synthèse,
  pas en brut — la mémoire sert de compression de contexte.
- **Prompt caching (Anthropic)** : le bloc `system` (stable d'un tour/agent à l'autre) est marqué
  `cache_control: ephemeral` → les tours suivants paient une fraction des tokens d'entrée.
- **Divulgation progressive des fiches** (`Load_Skill`) : le prompt ne porte que nom+description
  des fiches assignées ; le corps est chargé à la demande via `Load_Skill{id}`, exposé uniquement
  aux agents ayant au moins une fiche. Bénéficie aux deux fournisseurs (prompt plus court).
- **Context caching Gemini : volontairement non implémenté.** Le caching explicite Gemini est un
  flux *stateful* (ressource `CachedContent` créée par un appel séparé, TTL à gérer) avec un seuil
  de tokens minimal élevé — faible ROI sur des boucles d'agents courtes, et difficile à tester
  hors-ligne. Les modèles Gemini récents font du caching *implicite* automatique ; la divulgation
  progressive réduit déjà le prompt côté Gemini. À reconsidérer si les boucles s'allongent.

## Orchestration réelle : plan → approbation → exécution → synthèse (post-Phase 5) ✅

- L'objectif `[1]` ne diffuse plus une consigne plate : le chef **décompose** en `Plan` de
  `Task`s assignées + dépendances (`crate::orchestration`), validé (cycles, agents, deps).
- Planification via LLM (outil `submit_plan`) avec **repli linéaire déterministe** hors-ligne ;
  exécution en **ordre topologique** (`execute_plan`) où chaque tâche reçoit en contexte les
  sorties de ses dépendances et **trace son résultat en mémoire** (hand-off), puis **synthèse**.
- **Écran d'approbation** : le plan est montré (`AgentEvent::PlanReady`) et l'utilisateur
  l'exécute (`Entrée`) ou l'annule (`Échap`) ; le radar suit l'avancement par tâche
  (`TaskStarted`/`TaskDone`/`TaskFailed`) dans un panneau Plan.
- **Exécution par vagues concurrentes** : les tâches indépendantes s'exécutent **en parallèle**
  (`futures::future::join_all`) ; une tâche n'attend que ses dépendances directes.
- **Re-planification itérative** (auto-correction) : après chaque manche, `evaluate_objective`
  (LLM) juge l'objectif atteint ou renvoie un **plan correctif** ré-approuvé → nouvelle manche,
  bornée par `MAX_ROUNDS`. La mémoire fait le pont entre manches. Mécanisme **agnostique** :
  aucun agent/skill dédié (en Dev, l'`Agent_Testeur` + `Execute_Terminal_Command` existants
  suffisent). Hors-ligne → une seule manche.
- **Orchestration depuis le chat `[5]`** : le coordinateur dispose d'un outil `orchestrate`
  (en plus des outils de délégation) qui lance la boucle complète (plan validé → exécution
  parallèle → auto-correction → synthèse) **en pleine conversation** ; `ChatHandle.approve`
  porte l'approbation, réutilisant le même écran de plan. La synthèse revient au coordinateur
  comme résultat d'outil et nourrit sa réponse. Logique d'orchestration mutualisée
  (`run_orchestration`) entre `[1]` et `[5]`.
- Correctif au passage : nom d'outil de délégation slugifié (agents accentués → API valide).
- Hors périmètre (à suivre) : support MCP, parallélisme inter-manches plus fin.

## GUI bureau Dioxus — tranche verticale (post-Phase 5) 🚧

- Nouveau crate `orchestra-desktop` : interface graphique **tout-Rust** (Dioxus) qui consomme
  directement `orchestra-core` — **aucune frontière IPC, aucun Node**. Bénéfice direct du
  découplage : 2e consommateur du même cœur, à côté du TUI.
- Tranche verticale : charge l'espace exemple, liste les agents, lance `runtime::orchestrate`
  et streame les `AgentEvent` (radar + panneau Plan + bouton d'approbation) — `match` natif sur
  le contrat d'événements.
- Choix : **Dioxus** plutôt que Tauri+React (rester mono-langage, appel direct du cœur).
- Limite : Dioxus desktop exige une webview système (WebView2 Windows / `webkit2gtk` Linux) —
  non compilable dans le conteneur cloud (libs GUI absentes) ; build/run sur poste (Windows).
  Premier jet, à affiner au premier build.

## Bascule automatique de fournisseur LLM (Claude ↔ Gemini) (post-Phase 5) ✅

- `LlmClient` gère une liste ordonnée de `Backend` (Claude préféré, Gemini en repli, selon les
  clés). `complete()` bascule sur le fournisseur suivant si l'actuel est indisponible (réseau,
  surcharge, 429, auth, ou **crédit épuisé** → 400 « credit balance too low »).
- Échec **permanent** (clé invalide / plus de crédit) → le backend est écarté pour la suite
  (`active: AtomicUsize`). Une requête malformée (400 hors facturation) remonte sans bascule.
- `ORCHESTRA_PROVIDER` force un fournisseur unique ; `describe()` affiche l'actif + le repli
  dans l'en-tête (ex. `Claude · claude-opus-4-8 (repli : Gemini)`).
- Tests : classification crédit-épuisé / auth / rate-limit / requête-malformée.

## Sélecteur de skills (catalogue à cocher) (post-Phase 5) ✅

- Assignation des skills repensée : `[6]` → `[s]` ouvre un **sélecteur** au lieu d'un champ
  texte. Catalogue navigable des primitives (`skills::catalog()`, id + description) et des
  fiches (`markdown_skill::load_all`), avec cases à cocher (`Espace`), `[e]` éditer une fiche,
  `[n]` en créer. Les skills assignés non branchés restent listés (`inact`) pour pouvoir les
  retirer. Résout le « il faut connaître les noms par cœur ».
- Cœur : `skills::catalog()` expose les primitives (id + description) depuis le registre.
- TUI : `SkillPicker`/`SkillEntry` + rendu dédié ; suppression de l'ancienne saisie texte
  (`AgentField::Skills`). Persistance via `save_config`.

## Skills « fiches » Markdown + création depuis l'UI (post-Phase 5) ✅

- Modèle « skill = dossier + `SKILL.md` » (façon *Agent Skills*) : `crate::markdown_skill`
  charge `.orchestra/skills/<id>/SKILL.md` (en-tête `name`/`description` + corps).
- Le runtime injecte les instructions des fiches **assignées** à un agent dans son prompt
  système (section « ## Compétences ») — aucun code, aucune recompilation.
- **Création depuis l'interface** : menu Agents → `[n]` saisit un nom → `markdown_skill::create`
  scaffolde la fiche, qui s'ouvre dans l'éditeur de texte (généralisé persona/skill, `Ctrl+S`
  enregistre via le cœur). Le menu marque ces skills **(fiche)** en cyan, distincts des
  primitives exécutables (vert) et des étiquettes inactives (gris).
- Deux couches assumées : **primitives = code** (registre `skills`), **skills = fichiers**
  (`markdown_skill`) qui orchestrent les primitives.

## Règle de parité TUI ⇄ GUI + gestion complète Agents & skills (post-Phase 5) ✅

- **Règle permanente** (cf. `CLAUDE.md`) : toute feature/amélioration est livrée **dans les deux
  interfaces** (`orchestra-tui` *et* `orchestra-desktop`), comportement identique. Tenable grâce au
  découplage : la logique vit dans le cœur, les UIs ne font qu'appeler.
- **Nouveau module cœur `catalog`** (partagé) : `SkillEntry`/`SkillKind` (Primitive/Fiche/Unwired),
  `skill_entries`, `agent_templates`/`inactive_agent_templates`, `wire_skill` (crée la fiche d'un
  skill non branché). Testé. Le TUI **et** le desktop consomment ces mêmes fonctions (fini la
  duplication ; parité par construction).
- **Documentaliste activable après coup** : c'était un drapeau `documentalist_enabled` fixé
  seulement à l'init → il « disparaissait » (ex. mode Langue). Désormais un **bouton/touche le
  bascule** dans les deux UIs (TUI `[t]`, desktop bouton du bandeau), persisté.
- **Brancher un skill « non branché »** : dans les deux UIs, un skill assigné sans implémentation
  ni fiche peut être **branché** (création de sa fiche `SKILL.md`, puis édition). TUI `[b]` dans le
  sélecteur ; desktop bouton « brancher ».
- **Agents suggérés** : ajout en un geste des rôles du catalogue du type de projet pas encore
  présents (`inactive_agent_templates`). TUI `[g]`, desktop « + Agent suggéré ».
- **Desktop — vue Agents reconstruite** en menu complet : toggle Documentaliste, liste d'agents +
  ajout suggéré/personnalisé, renommer / éditer le rôle / supprimer, sélecteur de skills à cocher,
  brancher, créer/éditer les fiches (éditeur intégré). Parité atteinte avec le menu `[6]` du TUI,
  enrichi des nouveautés ci-dessus des deux côtés.

## Desktop — visualiseur Markdown rendu + Mermaid (post-Phase 5) ✅

- La vue **Documents** du desktop n'affiche plus le Markdown brut : rendu **HTML** via
  `pulldown-cmark` (`state::render_markdown_html`) — titres (`#`/`##`/`###`), listes, code,
  tableaux, citations — injecté avec `dangerous_inner_html` + CSS dédié `.markdown`.
- **Diagrammes Mermaid affichés visuellement** : les blocs ` ```mermaid ` deviennent des
  `<pre class="mermaid">`, rendus par **mermaid.js** (chargé une fois depuis le CDN, exécuté
  via `document::eval` à chaque changement de document). Dégradé propre si indisponible
  (affiche le code source). *Skill produit par l'Agent Documentaliste enfin lisible comme un
  vrai schéma.*
- Parité : le **TUI rend déjà le Markdown** (`markdown.rs` → lignes ratatui) ; le rendu
  graphique d'un diagramme Mermaid est propre au médium graphique (le terminal montre le code,
  ce qu'il faisait déjà). La capacité « voir ses documents mis en forme » est donc des deux côtés.
- Prérequis runtime : accès réseau de la webview pour le CDN mermaid (build/poste).
- **Édition des documents** depuis la vue Documents (persona, memory, ADR, `.md` du workspace) :
  nouveau `model::save_document(path, content)` (écriture centralisée). TUI : `[e]` dans le
  visualiseur édite **n'importe quel** document (plus seulement le persona) — `EditTarget::Document`,
  Ctrl+S enregistre. Desktop : bouton « ✏ Éditer » dans `DocumentsView` (textarea + 💾 Enregistrer).

## Création d'espaces dans l'app + catalogue d'agents DEV (post-Phase 5) ✅

- **Création d'un nouvel espace depuis les deux apps** (plus seulement via `orchestra init`) :
  formulaire **nom · type (Dev/Langue) · workspace (Dev) · objectifs ·
  documentaliste**. TUI : sélecteur `[3]` → `[n]` (formulaire navigable Tab/↑↓, `←/→` type,
  Entrée crée) ; Desktop : bouton « ➕ Nouveau space » dans `SpaceBar`. À la création, l'espace
  est ouvert et mémorisé (registre). L'espace est créé dans `dossier_parent/<slug(nom)>`.
- **Cœur** : `InitOptions` gagne `objectives` (injecté dans le persona, section « ## Objectifs du
  projet ») et `agents` (squad choisie ; vide → squad de départ). `scaffold_space` inchangé côté
  signature d'usage.
- **Catalogue d'agents DEV (cycle de vie complet)** : `catalog::agent_catalog(kind)` — pour Dev :
  Architecte · Codeur · Testeur · Reviewer · Debuggeur · Refactoreur · DevOps · Sécurité · DBA ·
  Release (du local à la mise en prod). `default_agents` reste la squad *de départ* (Architecte,
  Codeur, Testeur) ; le reste s'active via le menu Agents (« + Agent suggéré » puise désormais dans
  ce catalogue). L'utilisateur compose sa squad comme il veut, puis avance via le chat.
- **Correctif Windows** : `Execute_Terminal_Command` lançait toujours `sh -c …` → `npm`/`npx`
  (scripts `.cmd`) « introuvables » côté agent sur Windows. Désormais shell **selon la plateforme** :
  `cmd /C` sur Windows (résout `.cmd` via PATHEXT + PATH système), `sh -c` ailleurs. La commande
  hérite de l'environnement du process.
- **Autonomie des commandes longues/non-interactives** : `Execute_Terminal_Command` voyait ses
  `npm create`/`npm install` échouer (délai 30 s + invites bloquantes). Désormais : délai **300 s**
  (surchargeable par `ORCHESTRA_COMMAND_TIMEOUT_SECS`), **stdin neutralisé** (une invite reçoit EOF
  au lieu de bloquer), et **env non-interactif** (`CI=1`, `npm_config_yes`, `npm_config_progress=false`,
  `NO_UPDATE_NOTIFIER`…). La description de l'outil guide le modèle vers des commandes non interactives.
- **Encart de statut des agents (desktop)** : le TUI a déjà sa sidebar « 🎻 Orchestre » (statut live
  via `on_event`) ; le desktop manquait de visibilité. Ajout d'un `SquadPanel` (coordinateur +
  agents + documentaliste) avec statut live (en attente / réfléchit… / actif / terminé) dérivé du
  flux `AgentEvent`, affiché dans les vues Orchestrer et Chat. Parité atteinte des deux côtés.

## Recentrage DEV (post-Phase 5) ✅

- **Suppression des types Immobilier et Nutrition** : `ProjectType` ne garde que `Dev` (focus) et
  `Langue` (conservé, mis de côté). Tous les `match` mis à jour (skills/agents par défaut, persona,
  intention par défaut, wizards CLI/TUI/desktop). Exemple `examples/recherche-immo-aix` supprimé ;
  l'espace de démarrage du desktop devient `examples/apprentissage-espagnol`.
- Docs (README, FONCTIONNEL, ARCHITECTURE) alignées sur le focus développement.
- Cap produit : un IDE de l'ère agentique **orienté création/reprise de projets de dev**.

## Suivi des modifications de fichiers (diffs) + plan enrichi (post-Phase 5) ✅

- **Nouveau `AgentEvent::FileChanged { path, added, removed, diff }`** émis par le runtime quand un
  agent écrit un fichier (`Write_File_Validated`) : on lit le contenu avant/après autour de l'appel
  et on calcule un **diff par lignes** (module cœur `diff`, sans dépendance, testé).
- **TUI** : vue **Modifications** `[7]` — liste des fichiers changés (`+a -r`) + diff coloré du
  fichier sélectionné. Réinitialisée à chaque run.
- **Desktop** : onglet **Modifications** (`ChangesView`) — même liste + diff coloré.
- **Plan enrichi** : le panneau Plan montre objectif + dépendances par tâche (déjà le cas côté TUI ;
  ajouté côté desktop). Avec l'encart Squad, on voit *qui* fait *quoi* et *ce qui change*.

## Thème VS Code + cadrage de projet (point 2) (post-Phase 5) ✅

- **Thème VS Code (Dark+)** côté desktop : réécriture du CSS (variables de thème), onglets à liseré
  actif, boutons primaire/secondaire/danger, champs à focus, listes survol/sélection, diff coloré,
  barre de statut, scrollbars fines. (Le TUI conserve son rendu terminal — parité **fonctionnelle**.)
- **Cadrage (point 2)** : `runtime::cadrage_message(idea)` (partagé) — premier message qui fait
  **interviewer** l'utilisateur par le coordinateur puis **rédiger un brief** (`docs/brief.md`)
  avant de coder. Desktop : bouton « 🧭 Cadrer le projet » (la saisie = l'idée). TUI : **Ctrl+G**
  dans le chat. Même comportement des deux côtés.

## Reprise de projet existant (point 3) (post-Phase 5) ✅

- `scaffold::adopt_project(root)` (testé) : initialise `.orchestra/` **dans** un dossier de code
  existant (type Dev, workspace = ce dossier, Documentaliste activé) → le projet devient pilotable.
- `runtime::comprehension_message()` (partagé) : premier message qui fait **scanner** le code,
  **rédiger `docs/comprehension.md`** et poser des questions **avant** toute évolution.
- Desktop : bouton « reprendre (Dev) » sur un dossier non-espace du navigateur ; bouton
  « 🔎 Analyser le projet » dans le chat. TUI : `[r]` dans le navigateur, **Ctrl+R** dans le chat.
- S'appuie sur le suivi des modifications déjà en place (on voit ce que les agents changent).

## Simplification : un seul espace conversationnel (post-Phase 5) ✅

- **Fusion Orchestrer + Chat** côté desktop en un onglet unique **« Assistant »** (conversation).
  L'exécution passe désormais par **un seul chemin** : le coordinateur. Trois actions au-dessus du
  fil : **▶ Objectif rapide**, **🧭 Cadrer**, **🔎 Analyser** — toutes envoient un message dédié
  (`runtime::orchestrate_message` / `cadrage_message` / `comprehension_message`).
- Suppression de l'onglet Orchestrer, de `drive_orchestration`, du radar et de l'objectif séparés
  côté desktop (le plan + l'approbation s'affichent inline dans la conversation).
- TUI : `[5]` Assistant (conversation) + `[1]` Objectif rapide ; libellés clarifiés. Capacités
  identiques des deux côtés (converser · objectif rapide · cadrer · analyser).
- **Modifications en direct à côté de la conversation** : desktop — l'Assistant est en 2 colonnes
  (conversation à gauche, panneau `LiveChanges` à droite : liste + diff au clic, historique du run).
  TUI — section « 📝 Modifs récentes » ajoutée à la sidebar toujours visible (6 derniers fichiers
  + `+a/-r`). L'onglet/vue Modifications plein écran reste disponible pour la revue détaillée.

## Actions de l'Assistant propres au type de projet (post-Phase 5) ✅

- `runtime::quick_actions(kind)` (data-driven, testé) : la barre d'actions de l'Assistant s'adapte
  au type de projet.
  - **Dev** : ▶ Objectif rapide · 🧭 Cadrer · 🔎 Analyser.
  - **Langue** : 📚 Plan d'apprentissage (cours → leçons, écrit dans `docs/plan-apprentissage.md`) ·
    ▶ Leçon & exercice du jour (prochaine leçon selon la progression). Le chat sert à poser des
    questions / s'entraîner.
- Messages partagés : `learning_plan_message` / `daily_lesson_message` (+ orchestrate/cadrage/
  comprehension pour Dev).
- Desktop : boutons générés depuis `quick_actions` (`action_button`). TUI : **F1..Fn** dans le chat,
  aide listée dynamiquement. Mêmes actions des deux côtés.

## Refonte — Phase 1 : l'Orchestrateur PTAC (post-Phase 5) ✅

Recentrage du produit sur **un agent principal unique et puissant** qui déploie sa propre équipe.
- `COORDINATOR` → **« Orchestrateur »**. La conversation ne s'appuie plus sur un roster figé
  (suppression de `coordinator_prompt`, `run_coordinator_turn`, `delegation_tool`, `orchestrate_tool`).
- **Prompt PTAC** (`orchestrator_prompt`) : boucle **Perceive → Think → Act → Check**, agent
  autonome/robuste, transparent, documente au fil de l'eau.
- **Outillage complet** (`orchestrator_tools`) : tous les skills exécutables (`skills::all_tool_specs`)
  + intégrations + mémoire + `Load_Skill` + **`spawn_agent`**.
- **`spawn_agent(role, instruction)`** : l'Orchestrateur **crée des sous-agents ad hoc** à la volée
  (multi-agents composé par lui, plus par l'utilisateur). Récursion async bornée (`Box::pin` ; les
  sous-agents n'ont pas `spawn_agent`).
- UIs inchangées (contrat `AgentEvent` intact) : elles dialoguent maintenant avec l'Orchestrateur.
- **Suite** : Phase 2 (retrait de `ProjectType`/agents pré-définis/menu Agents ; « espace » →
  « session ») ; Phase 3 (refonte UI façon Cursor : explorateur · centre code/diff · chat).

## Refonte — Phase 2 : ménage (catalogue, roster, types) ✅

Suppression de tout ce qui pré-câblait l'orchestre : l'Orchestrateur compose désormais son
équipe seul, la config ne décrit plus qu'un espace nu.
- **Tranche 2** — suppression du catalogue et des matrices par défaut : `catalog.rs`,
  `model/skill_id.rs` (`default_agents`/`default_skills`) supprimés ; plus de skills/agents
  injectés à la création (une session démarre vierge).
- **Tranche 3a** — retrait du **moteur de roster/orchestration** de `runtime.rs`
  (`spawn`/`run_agent`/`roster`/`run_waves`/`plan_objective`/`synthesize`…). Seul subsiste le
  chemin PTAC de l'Orchestrateur. `AgentContext` allégé, `agent_tools`/`build_system_prompt`
  simplifiés, `orchestrate()` en one-shot PTAC.
- **Tranche 3b** — retrait de **`ProjectType`** (Dev/Langue) et du schéma associé. `ProjectConfig`
  se réduit à `project_name` / `workspace_path` / `integrations` ; les anciens champs
  (`project_type`, `agents`, `skills`, `documentalist_enabled`, `AgentDef`) présents dans
  d'anciens `config.json` sont **ignorés au chargement** (serde tolère l'inconnu). `quick_actions()`
  devient générique (plus de branche Langue ; messages `learning_plan`/`daily_lesson` retirés).
  `InitOptions` allégé ; persona générique ; assistant `orchestra init` sans choix de type ;
  formulaires « nouvel espace » (TUI + desktop) réduits aux champs utiles ; `SquadPanel`/sidebar
  affichent l'Orchestrateur + les sous-agents **apparus à la volée** (plus de roster figé).
- Parité TUI ⇄ GUI respectée ; `orchestra-core` + `orchestra-tui` verts, `clippy` sans warning.
- **Suite** : Phase 3 (refonte UI façon Cursor).

## Refonte — Phase 2 (suite) : sessions en onglets ✅

Les Espaces s'ouvrent désormais comme des **onglets** : plusieurs sessions ouvertes en même
temps, chacune avec **son propre contexte et son historique**, et on **bascule** de l'une à
l'autre sans rien perdre.
- **Cœur** — module `session::Sessions<T>`, générique sur un trait `Tabbed` (titre + racine) :
  mécanique d'onglets **testée** (ouvrir/dédup par racine/activer/fermer/cycler/`index_of`),
  agnostique de l'affichage. Rouvrir un Espace déjà ouvert **réactive** son onglet (historique
  préservé, pas de doublon).
- **TUI** — la boucle gère N sessions. Chaque onglet garde son `App` et ses canaux ; une session
  en arrière-plan continue de tourner (ses événements s'empilent dans son `rx`). Barre d'onglets
  en tête (dès 2 sessions), **Tab / Maj+Tab** pour circuler, **Ctrl+W** pour fermer. Ouvrir un
  Espace (sélecteur / navigateur / saisie / création) ouvre un onglet.
- **Desktop** — même modèle via `Sessions<DesktopSession>` dans un `Signal` : l'état vivant de
  chaque session (messages, plan, modifs, statuts, canaux) vit dans le store ; les composants
  lisent une **projection** de la session active. Les flux d'événements écrivent dans **leur**
  session (repérée par sa racine), donc une session en arrière-plan n'altère jamais l'affichage
  courant. Barre d'onglets cliquable (basculer / fermer).
- **Suite** : Phase 3 (refonte UI façon Cursor : explorateur · centre code/diff · chat).

## Refonte — Phase 3 (suite) : layout fidèle au template + thèmes 🚧 (desktop)

Reproduction du template fourni (façon Cursor / Claude desktop), **épuré**, avec panneaux
repliables et bascule de thème.
- **Layout 5 zones** : rail **Checkpoints** (fin) · **Explorateur** (repliable) · **Conversation** ·
  **Visualiseur** code+diff (repliable) · rail **Tâches / Mémoire / Contexte**. Replié, l'écran
  correspond exactement au template (explorateur caché → checkpoints + conversation + code + tâches).
- **Barre supérieure** : onglets de session (browser-like) + « + » + bascules (explorateur,
  visualiseur, thème). Pas de bouton « parasite » : chaque contrôle agit.
- **Thème clair par défaut**, **sombre** en un clic (variables CSS, bascule instantanée).
- **Réel léger** : Checkpoints = jalons de conversation (clic → défilement, navigationnel) ;
  **Terminal** = sortie réelle des commandes des agents (nouvel `AgentEvent::Terminal`) ;
  **Mémoire** = notes de `.orchestra/memory.md` (`memory::entries`) ; **Contexte** = fichiers
  lus/écrits dans la session ; pastille de statut = état réel de l'agent.
- Fenêtre par défaut 1280×820 (tient en 1920×1080). Warning `draft` corrigé.
- **Itération finition** : thème **sombre à dominante verte par défaut** (comme le template),
  polish (transitions, coins arrondis, ombres, spinner du plan, pastilles d'onglet vertes,
  bouton d'envoi rond vert, anneau de focus). Boutons de repli **dans le coin** de l'explorateur
  et du visualiseur (+ stub cliquable pour rouvrir), retirés de la barre supérieure. Suppression
  des **actions rapides** obsolètes (Objectif rapide / Cadrer / Analyser) côté cœur + TUI + desktop.
  Indicateur **LLM** dans la barre de statut (Claude / Gemini / mode simulé selon les clés d'env).
- **Finition 2** : suppression des emojis « datés » (logo, dossiers/fichiers, statuts) remplacés par
  des marqueurs CSS propres ; bords lissés / arrondis ; **chat aéré façon template** (Orchestrateur
  en texte plein sans bulle ni label, message utilisateur en bulle arrondie discrète, sous-agents
  en pilule repliable) ; spinner CSS sur l'étape de plan en cours.
- **Plan réel → rail Tâches** : l'Orchestrateur dispose désormais de deux outils —
  `Set_Plan(steps)` (publie le plan) et `Update_Step(step, status)` (running/done/failed) — qui
  émettent `PlanReady`/`Task*`. Le rail « Tâches » se remplit et suit l'avancement en direct, au
  lieu que le plan ne reste qu'en prose dans le chat. Le prompt PTAC impose leur usage. Publier un
  plan n'exige plus d'approbation (l'Orchestrateur pilote lui-même).
- `orchestra-core` + `orchestra-tui` verts, `clippy` sans warning ; desktop à vérifier au build local.

## Refonte — Phase 3 : shell « Cursor » + « orchestre en verre » 🚧 (desktop)

Refonte de l'UI desktop en **3 panneaux** (façon Cursor / Claude desktop) et introduction de
l'axe innovant **« orchestre en verre »** : on *voit* l'équipe d'agents travailler dans le code.
- **Ménage** : suppression des projets d'exemple (espagnol…) et des boutons « parasites » —
  fini le sélecteur de vues (Assistant/Documents/Modifications) et le panneau latéral redondant.
- **Cœur** — l'activité fichier est désormais **attribuée à l'agent** : `AgentEvent::FileChanged`
  porte `agent`, et un nouvel `AgentEvent::FileRead { agent, path }` signale les lectures. Nouveau
  module testé `explorer` (arborescence du workspace à plat, dossiers bruyants ignorés, bornée).
- **Desktop** — shell `explorateur · centre · conversation` :
  - **Explorateur** (gauche) **annoté en temps réel** : chaque fichier s'illumine selon l'agent
    qui le **lit** (👁, liseré bleu) ou l'**écrit** (✎, liseré vert). C'est le « verre » posé sur
    l'orchestre : on suit l'équipe qui parcourt et modifie le code, en direct.
  - **Centre** : le fichier sélectionné — **diff** coloré s'il a été modifié par un agent dans la
    session, sinon Markdown **rendu** (+ Mermaid) ou texte, avec édition/enregistrement.
  - **Droite** : l'Orchestrateur (squad live + conversation + actions rapides).
- **Parité** : périmètre desktop d'abord (choix utilisateur) ; le TUI conserve ses capacités
  (il ignore `FileRead` pour l'instant) et sera aligné ensuite (explorateur annoté côté ratatui).
- `orchestra-core` + `orchestra-tui` verts, `clippy` sans warning ; desktop à vérifier au build
  local (webview absente en cloud).

## Registre des espaces connus (récents) (post-Phase 5) ✅

- Nouveau module cœur `registry` (testé) : liste **persistante** des espaces déjà ouverts
  (`<config>/orchestra/spaces.json` — `%APPDATA%`/`$XDG_CONFIG_HOME`/`$HOME/.config`).
  `known_spaces()`, `remember_space()` (valide l'espace + lit son nom, récents d'abord, dédup),
  `forget_space()`. Mémorisation automatique à chaque ouverture réussie.
- **TUI** : `[3]` ouvre désormais un **sélecteur d'espaces** (au lieu de la saisie directe) —
  ↑↓ choisir, Entrée ouvrir, `[a]` saisir un chemin, `[x]` ne plus suivre. Ouverture centralisée
  (`open_space`) côté saisie et sélecteur.
- **Desktop** : composant `SpaceBar` — saisie de chemin + **puces** des espaces connus (clic pour
  rouvrir, × pour retirer). Fini de retaper les chemins de mémoire.
- Parité respectée : même registre cœur, même comportement des deux côtés.
- **Navigateur de dossiers** (module cœur `browser`, testé) pour **découvrir un espace sans
  taper de chemin** : `browse(dir)` liste les sous-dossiers et marque ceux qui sont des espaces
  (`.orchestra/config.json`), `parent()`, `home_dir()`. TUI : sélecteur `[3]` → `[b]` ouvre un
  navigateur (↑↓ · Entrée ouvrir/entrer · `[u]`/← remonter). Desktop : bouton « 📂 Parcourir »
  dans `SpaceBar` (navigation par dossiers, ouverture des espaces repérés). La **saisie manuelle
  de chemin a disparu** côté desktop au profit des récents + du navigateur.

## Registre de skills exécutables (post-Phase 5) ✅

- Les skills sont **activés systématiquement** via un registre : id → définition
  ([`tool_definition`]) + exécution ([`execute_skill`]), source de vérité
  `skills::EXECUTABLE_SKILLS`. Un skill assigné à un agent (menu `[6]`) devient un **vrai
  outil** s'il est dans le registre ; sinon il reste une étiquette.
- `skills::is_executable(id)` exposé ; le menu Agents marque les skills **exécutables** (vert)
  vs **(inactif)** (gris).
- Nouveau skill exécutable **`Web_Fetch`** (lit une URL http/https) — démontre l'extensibilité
  (1 entrée au catalogue + 1 bras dans chaque match). `dev_tool_definitions` → `tool_specs`.

## Gestionnaire d'agents (post-Phase 5) ✅

- **Modèle structuré** : `config.agents` passe de `Vec<String>` à `Vec<AgentDef { name, role,
  skills }>`, avec **désérialisation rétro-compatible** (un agent écrit en chaîne reste
  valide). `default_agents` fournit nom + rôle + skills par type.
- **Runtime** : chaque agent utilise son **rôle** (injecté dans le prompt système) et ses
  **skills propres** (repli sur les skills de l'espace s'il n'en a pas) ; le coordinateur
  expose le rôle dans ses outils de délégation.
- **Menu `[6]` Agents** : liste + fiche (rôle, skills, **stats de session** : invocations +
  temps de réflexion) ; édition complète — renommer, rôle, skills, ajouter, supprimer —
  **persistée dans `config.json`** via `ContextSpace::save_config` (l'écriture reste au cœur).
- `[1]` pré-remplit aussi une **intention d'exemple** selon le type de projet.

## `[1]` repurposé en « Lancer une intention » (post-Phase 5) ✅

- `[1]` ne lance plus une rafale autonome générique : il **saisit un objectif** puis
  l'exécute **en one-shot** via le coordinateur (envoi d'un seul message dans une
  `ChatHandle`, puis fermeture immédiate du canal → la tâche se termine et rend un
  compte-rendu). Identité claire : `[1]` = tâche autonome, `[5]` = conversation.
  `runtime::spawn` (rafale parallèle) reste disponible côté bibliothèque mais n'est plus
  câblé à l'UI.

## Améliorations UX (post-Phase 5) ✅

- `orchestra init` (Dev) : le **workspace est résolu en chemin absolu** (fini la fragilité
  du `.` selon le répertoire de lancement), et l'assistant **propose de configurer Git et
  GitHub** (token jamais saisi — seul le nom de variable est enregistré). `InitOptions`
  porte désormais les intégrations.
- Dashboard : au lancement `[1]`, si le persona contient encore des « à compléter » **et**
  qu'un LLM est actif, un avertissement s'affiche au lieu d'un appel LLM voué à l'échec.
- **Éditeur de persona intégré** (`[4]`) : mini éditeur multi-ligne (`orchestra-tui::editor`,
  UTF-8) pour renseigner le contexte sans quitter l'outil ; `Ctrl+S` persiste via
  `ContextSpace::save_persona` (l'écriture disque reste dans le cœur).
- **Navigateur de documents + visualiseur Markdown** (`[2]`) : `ContextSpace::documents()`
  agrège persona, ADRs et Markdown du workspace (ceux produits par le Documentaliste
  compris) ; ouverture dans un visualiseur Markdown stylé (`orchestra-tui::markdown`), `e`
  pour éditer le persona. Lecture via `load_document()` (cœur). Objectif : limiter les
  actions hors logiciel.

## Phase 4c — Intégration Jira ⏳ (optionnelle, à venir)

**Visé**
- Même schéma que GitHub : Skills Jira (créer / transitionner un ticket) exposés si
  `integrations.jira` est configuré, token via variable d'environnement.

## Phase 5 — Documentaliste + finitions ⏳ (à venir)

**Visé**
- Agent Documentaliste (mise à jour de doc automatique, diagrammes Mermaid), interactions
  `[2]`/`[3]` du dashboard, polissage.

## Recentrage total sur le dev : Git/Docker/Fichiers structurés + parité complète TUI⇄Desktop ✅

Chantier de fond : recentrer l'outil sur les besoins concrets d'un dev au quotidien — voir
l'état de son code (Fichiers), de son dépôt (Git), de son conteneur (Docker), de ce que
l'Orchestrateur fait (Plan/Tâches) et de qui travaille (Agents) — et purger tout ce qui ne
sert pas cet objectif. Constat de départ : le **code** était déjà 100 % dev-focused (les
anciens types de projet Langue/Immobilier/Nutrition avaient déjà été retirés du code lors du
« Recentrage DEV » et de la « Refonte » ci-dessus) ; seule la **documentation** en gardait des
traces, et il manquait des panneaux passifs de constat (Git structuré, Docker) que le
runtime — piloté par événements d'agents — ne pouvait pas fournir nativement.

**Cœur (`orchestra-core`)**
- Nouveau module **`git.rs`** : `GitStatus`/`GitFileStatus` structurés, à partir d'un parsing
  de `git status --porcelain=v2 --branch` (format stable, contrairement à `--short`) —
  branche, upstream, ahead/behind, fichiers staged/unstaged/untracked. `git::diff(root, path)`
  pour le diff non indexé. Jamais d'erreur remontée à l'UI (`is_repo: false` en état neutre).
  `integrations.rs` (outils Git de l'agent) **délègue** désormais son shell-out à
  `git::run_command` — une seule source de vérité pour tout appel `git`, agent ou UI passive.
- Nouveau module **`docker.rs`**, lecture seule : détecte `Dockerfile`/fichier compose à la
  racine, interroge `docker compose ps --format json` (gère à la fois le tableau JSON et le
  NDJSON selon la version de Compose) si le démon est joignable. Jamais bloquant : binaire ou
  démon absent → état neutre (`docker_available: false`).
- Ces deux modules répondent à un modèle **pull** (l'UI appelle directement, à l'ouverture
  d'un écran ou sur rafraîchi) plutôt que **push** (`AgentEvent`) — cohérent avec leur nature
  de panneaux de constat indépendants de toute conversation en cours.
- 15 nouveaux tests (parsing porcelain v2, parsing JSON/NDJSON `compose ps`, détection sur
  dossier temporaire, dépôt Git temporaire réel). `cargo test -p orchestra-core` : 68 → verts,
  `clippy` propre.

**TUI (`orchestra-tui`)**
- Six nouveaux écrans : **Fichiers** (`[6]`, arborescence + activité live), **Git** (`[8]`,
  branche/statut + diff au clic), **Docker** (`[9]`, conteneurs ou état neutre), **Terminal**
  (`[0]`, commandes exécutées par les agents — événement déjà émis mais jamais affiché avant
  ce chantier), **Mémoire** (`[m]`), **Contexte** (`[c]`, fichiers touchés cette session).
  Récupération Git/Docker via des tâches `tokio` + canaux `oneshot`, sur le même modèle que le
  reste des appels asynchrones du TUI.
- `cargo test -p orchestra-core -p orchestra-tui` : 103 tests verts (dont rendu **headless**
  des 6 nouveaux écrans à plusieurs tailles de terminal — a capturé et corrigé un panic
  `clamp(min > max)` sur petit terminal dans le rendu Git). `clippy` propre.

**Desktop (`orchestra-desktop`)**
- Rail Tâches enrichi de trois sections : **DOCS** (persona/ADR/mémoire/`.md`, clic → édition
  dans le panneau central — le persona n'était éditable nulle part côté desktop avant ce
  chantier), **GIT** (branche/statut, clic fichier → diff réel), **DOCKER** (conteneurs,
  lecture seule). `CenterPane` gagne trois modes exclusifs : diff Git réel (prioritaire),
  document d'espace éditable, fichier workspace (comportement historique inchangé).
  Rafraîchissement Git/Docker au changement de session active (`use_memo` sur la racine de
  travail), pas à chaque `AgentEvent` — évite un `git status`/`docker compose ps` en boucle.
- Compilation non vérifiable dans cet environnement (webview Linux absente, comme documenté
  de longue date) — relecture manuelle attentive à la place (a détecté et corrigé un double
  déplacement (`move`) d'un `PathBuf` entre deux closures de bouton). **Build local requis**
  avant de considérer ce chantier définitivement clos côté desktop.

**Nettoyage**
- Suppression du dossier non suivi par git `examples/recherche-immo-aix/` — résidu d'un ancien
  test « immobilier », hors du positionnement du produit.
- Réécriture complète de `README.md`, `docs/ARCHITECTURE.md` et `docs/FONCTIONNEL.md` :
  suppression de toute trace documentaire du roster fixe / des types de projet Dev-Langue
  d'une architecture antérieure à la Refonte (le code n'en gardait déjà plus rien, seule la
  doc était restée figée) ; nouveau contenu structuré autour des 5 piliers ; documentation
  honnête du fait que `orchestration.rs` (modèle `Plan`/`Task` avec tri topologique) n'est
  **plus câblé** au chemin d'exécution actuel (remplacé en pratique par
  `Set_Plan`/`Update_Step`), pour éviter de reproduire le même écart doc/code à l'avenir.

## Fournisseur LLM local : Ollama, sans clé API ✅

Ajout d'un troisième fournisseur dans `orchestra-core::llm` — **Ollama**, un serveur de
modèles local (`http://localhost:11434` par défaut), qui ne demande **aucune clé API**.
Motivé par un besoin concret : travailler avec des modèles déjà installés en local
(ex. `qwen2.5-coder`) sans dépendre d'un compte cloud.

- `Provider::Ollama` + rendu/parsing dédiés (`ollama_body`/`parse_ollama`) du format
  `/api/chat` (proche d'OpenAI) : un message assistant fusionne texte + `tool_calls`, un
  résultat d'outil devient un message `tool` par appel (Ollama n'accepte pas de résultats
  groupés), `arguments` accepté en objet **ou** en chaîne JSON selon le modèle.
- `ORCHESTRA_PROVIDER=ollama` (ou `local`) force Ollama en fournisseur unique — modèle
  `qwen2.5-coder` par défaut (le plus orienté code du catalogue Ollama courant),
  surchargeable par `ORCHESTRA_MODEL` ou `ORCHESTRA_OLLAMA_MODEL` ; `ORCHESTRA_OLLAMA_HOST`
  pour un serveur distant.
- **Repli automatique opt-in** : sans fournisseur forcé, Ollama ne rejoint la chaîne
  Claude→Gemini que si `ORCHESTRA_OLLAMA_MODEL` est explicitement défini — jamais par défaut,
  pour ne changer le comportement (mode simulé si aucune clé cloud) d'aucune installation
  existante qui n'a pas Ollama.
- 4 nouveaux tests (rendu de requête, parsing texte/tool_calls, `arguments` en chaîne,
  contenu vide). `cargo test -p orchestra-core` : 72 tests verts, `clippy --workspace` propre
  — et `orchestra-desktop` compile et passe clippy dans cet environnement (webview désormais
  disponible ici), confirmant que le nouveau `Provider::Ollama` ne casse aucun `match`
  exhaustif côté UI.
- **Correctif de parité au passage** : le TUI affiche déjà le fournisseur actif via
  `LlmClient::describe()` (source de vérité unique) ; le Desktop, lui, redérivait un libellé
  **localement** à partir des seules variables `ANTHROPIC_API_KEY`/`GEMINI_API_KEY` — avec
  Ollama, ça aurait affiché « mode simulé » dans la barre de statut alors que l'app utilise
  bel et bien un modèle local. `state::llm_status()` délègue désormais à
  `LlmClient::from_env().map(|c| c.describe())`, comme le TUI : « Ollama · qwen2.5-coder »
  s'affiche correctement des deux côtés, sans dérive possible entre les deux heuristiques.
- **Correctif de timeout (retour d'usage réel)** : premier essai en conditions réelles
  (`qwen2.5-coder:7b` local) → une analyse de projet avec plusieurs outils s'est terminée par
  une « erreur réseau » (`error sending request for url`) au lieu d'une vraie réponse. Cause
  probable : le délai partagé de 120 s (calibré pour des API cloud) coupe la connexion en
  pleine génération sur un modèle local, souvent bien plus lent (CPU notamment). Ajout d'un
  délai **dédié** à Ollama (`ORCHESTRA_OLLAMA_TIMEOUT_SECS`, 600 s par défaut, via
  `RequestBuilder::timeout` sur la seule requête Ollama) — n'affecte pas les 120 s des
  backends cloud.
- **Correctif de fond (retour d'usage réel, suite)** : une fois le timeout réglé, `qwen2.5
  -coder:7b` posait bien des appels d'outils (`Recall`, `Read_File`…) mais **en texte pur**
  (`{"name": "Recall", "arguments": {...}}` affiché comme un message au lieu d'être exécuté)
  plutôt que dans le champ structuré `message.tool_calls` de l'API Ollama — le modèle « mime »
  l'appel plutôt que d'utiliser le mécanisme natif, un comportement documenté comme variable
  selon les modèles/quantisations chez Ollama. `parse_ollama` récupère désormais ce cas : si
  `tool_calls` est absent/vide et que `content` est **entièrement** un JSON `{"name": ...,
  "arguments": ...}` (éventuellement dans un bloc ```/```json), il est traité comme un vrai
  appel d'outil plutôt que comme du texte inerte. Volontairement strict (le contenu doit être
  *exclusivement* ce JSON) pour ne jamais réinterpréter à tort un texte narratif contenant des
  accolades. 5 nouveaux tests ; `tool_calls` natif reste prioritaire quand présent (le texte
  qui l'accompagne n'est jamais réinterprété).

## Indicateur « réfléchit… Ns » côté Desktop (comble un écart de parité) ✅

Le TUI affichait déjà, depuis longtemps, un chronomètre pendant qu'un agent attend une
réponse LLM (« ⠋ {agent} réfléchit… {n}s », en-tête + bas du radar, piloté par
`App::busy_since`/`busy_elapsed_secs`) — utile pour distinguer un modèle local qui mouline
plusieurs dizaines de secondes d'un vrai blocage. Le Desktop n'avait qu'un « … » statique,
sans durée : écart de parité repéré en creusant la demande d'un retour visuel sur le temps de
réflexion.

- `DesktopSession` gagne `busy_since: Option<Instant>` / `busy_agent: Option<String>`, posés
  sur `AgentEvent::Thinking` et effacés sur `Log`/`Done`/redémarrage — même sémantique qu'un
  seul chrono partagé côté TUI (un appel LLM à la fois bloque l'orchestre, pas un chrono par
  agent).
- **Rafraîchissement sans nouvel événement** : contrairement au TUI (qui redessine à un tick
  fixe), Dioxus ne se re-rend que sur changement de signal — un `use_future` fait tourner un
  chrono d'affichage (`tick`, +1 chaque seconde, pour la durée de vie de l'app) pour que le
  texte « …Ns » se mette à jour même pendant un silence prolongé côté agent.
- Affiché à deux endroits, comme le TUI : la pastille de la barre de statut (« réfléchit…
  Ns ») et une bulle atténuée en bas du chat (« {agent} réfléchit… Ns »).
- `orchestra-desktop/Cargo.toml` : feature `time` ajoutée à `tokio` (nécessaire à
  `tokio::time::sleep`, jusque-là seulement `sync`).
- Workspace complet (core + TUI + desktop) vert en tests et clippy dans cet environnement.

## Ollama : fenêtre de contexte explicite (`num_ctx`) — diagnostic d'usage réel ✅

Retour d'usage : « il faut être très précis, qwen n'arrive pas à discuter/lire la doc pour
suggérer une feature ». Ni un bug de tool-calling ni un modèle incapable — le suspect le plus
probable est la **fenêtre de contexte**. Ollama utilise par défaut un `num_ctx` conservateur
(souvent 2048-4096 selon le modèle) que `ollama_body` ne surchargeait pas jusqu'ici. Le seul
system prompt PTAC (`orchestrator_prompt`) + les définitions de **tous** les outils de
l'Orchestrateur (Skills exécutables, intégrations, mémoire, `spawn_agent`, `Set_Plan`/
`Update_Step`, éventuellement `Load_Skill`) peuvent déjà approcher ou dépasser cette limite
avant la moindre conversation — au-delà, le modèle perd le début du contexte, ce qui se
manifeste exactement comme décrit : des messages courts et précis passent (peu de contexte à
tenir), une conversation exploratoire (« relis la doc, suggère des évolutions ») échoue (gros
contexte à tenir : fichiers lus, historique, outils).

- `ollama_body` fixe désormais `options.num_ctx` à **8192** par défaut (au lieu de laisser
  Ollama choisir), surchargeable par `ORCHESTRA_OLLAMA_NUM_CTX` — à monter si la RAM/VRAM le
  permet (le cache KV grandit avec `num_ctx`), à baisser sur une machine contrainte.
- Documenté comme le premier réflexe de diagnostic dans `FONCTIONNEL.md`/`ARCHITECTURE.md`.
- Reste à confirmer en conditions réelles (pas d'Ollama dans cet environnement) : c'est un
  correctif raisonné à partir d'une cause connue et très fréquente avec les agents outillés
  sur Ollama, pas une reproduction directe du problème.
- Question de fond distincte (non technique) : `qwen2.5-coder` est un modèle **spécialisé
  code**, pas un modèle de chat généraliste — pour du brainstorming ouvert (« quelles features
  ajouter »), un modèle plus généraliste du même poste (`mistral`, `gpt-oss`) peut donner de
  meilleurs résultats, `qwen2.5-coder` restant préférable pour l'écriture de code proprement
  dite. Le mode conversationnel (`[5]` Assistant) n'a aucune restriction fonctionnelle à ce
  sujet : c'est la même boucle Orchestrateur, avec accès à `Read_File`/`Recall`/`Web_Fetch`,
  qu'on lui demande de coder ou de discuter.

## Ollama : appel d'outil « mimé » précédé de prose, aussi récupéré ✅

Confirmé en usage réel (et sur le dépôt du projet lui-même — voir incident ci-dessous) : le
correctif précédent (« appel d'outil mimé en texte ») exigeait que le message **entier** soit
le JSON de l'appel. Or `qwen2.5-coder` produit aussi une forme mixte, très fréquente en
pratique : une phrase d'explication **puis** l'appel dans un bloc ```json — ex. « D'accord,
essayons une approche différente. Je vais créer des dossiers…\n\`\`\`json\n{"name": …}\n\`\`\` ».
Cette forme n'était PAS récupérée : le message restait un simple texte affiché, jamais exécuté
— d'où l'impression que l'agent « n'agit pas », alors qu'il pose bien l'appel, juste pas au bon
endroit du message.

`fake_tool_call_from_text` tente maintenant, dans l'ordre : (1) le contenu entier est le JSON
(comportement précédent), (2) à défaut, le **premier bloc de code** ``` / ```json trouvé
n'importe où dans le message. Toujours strict : seul l'intérieur d'un bloc de code délimité est
scanné, jamais du texte libre (une accolade isolée dans une phrase normale ne déclenche rien —
testé). 2 nouveaux tests, dont une reproduction directe du message observé.

**Incident associé (corrigé manuellement, pas par ce correctif) :** avant ce correctif, une
première tentative de l'agent (message JSON pur, donc déjà récupérée par le correctif
précédent) a exécuté `Execute_Terminal_Command` avec `mv *.md docs/` sur **ce dépôt lui-même**
— déplaçant `README.md` et `CLAUDE.md` hors de la racine. Restauré via `git checkout --
CLAUDE.md README.md` + suppression des copies erronées dans `docs/`. Rappel pour l'utilisateur :
`Execute_Terminal_Command` est une capacité **assumée, non sandboxée** (cf. `README.md`,
`FONCTIONNEL.md`) — un agent local moins fiable peut générer des commandes destructrices avec
des globs trop larges ; travailler dans un dépôt Git (recovery facile via `git checkout`/
`git status`) est la meilleure protection actuelle, pas une garantie applicative.

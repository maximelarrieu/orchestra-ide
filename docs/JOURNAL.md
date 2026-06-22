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

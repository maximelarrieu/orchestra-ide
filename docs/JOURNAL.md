# Journal de bord — Orchestra IDE

> Avancement par phase : ce qui a été livré, les limites connues, ce qui suit. Mis à jour
> à chaque phase. Détails techniques dans [`ARCHITECTURE.md`](./ARCHITECTURE.md), vision
> produit dans [`FONCTIONNEL.md`](./FONCTIONNEL.md).

Plan global en 5 phases :

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
  **Claude** (`claude-opus-4-8`) ou **Gemini** (`gemini-2.0-flash`) au choix, via une
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

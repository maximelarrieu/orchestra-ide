# Journal de bord — Orchestra IDE

> Avancement par phase : ce qui a été livré, les limites connues, ce qui suit. Mis à jour
> à chaque phase. Détails techniques dans [`ARCHITECTURE.md`](./ARCHITECTURE.md), vision
> produit dans [`FONCTIONNEL.md`](./FONCTIONNEL.md).

Plan global en 5 phases :

1. Modèle des Espaces de Contexte + coquille du dashboard ✅
2. Commande `orchestra init` (scaffolding interactif) ✅
3. Runtime d'agents + flux temps réel → radar vivant ✅
4a. Intégration LLM (Claude **ou** Gemini) + Skills Dev exécutables (tool use) ✅
4b. Intégrations écosystème : Git / GitHub / Jira ⏳
5. Agent Documentaliste (Doc_Auto_Update, Mermaid) + finitions ⏳

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

## Phase 4b — Intégrations écosystème ⏳ (à venir)

**Visé**
- Git / GitHub / Jira (tokens via variables d'environnement, jamais en clair), exposés
  comme Skills supplémentaires au LLM.

## Phase 5 — Documentaliste + finitions ⏳ (à venir)

**Visé**
- Agent Documentaliste (mise à jour de doc automatique, diagrammes Mermaid), interactions
  `[2]`/`[3]` du dashboard, polissage.

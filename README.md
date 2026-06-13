# Orchestra IDE — Prototype CLI (Rust)

Moteur + interface d'un « IDE pour l'ère agentique ». TUI aujourd'hui (`ratatui`),
portage Tauri + React prévu — d'où le **découplage strict** logique métier / affichage.

## 📚 Documentation

| Doc | Contenu |
|---|---|
| [`docs/FONCTIONNEL.md`](docs/FONCTIONNEL.md) | Vision produit, Espaces de Contexte, parcours utilisateur, état des fonctionnalités |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Crates, modèle de données, flux d'événements `mpsc`, décisions techniques (diagrammes Mermaid) |
| [`docs/JOURNAL.md`](docs/JOURNAL.md) | Avancement par phase : livré / limites connues / à venir |

Doc technique générée du code : `cargo doc -p orchestra-core --open`.

## Structure (workspace Cargo)

```
crates/
├─ orchestra-core/   # domaine pur — AUCUNE dépendance UI
│  └─ src/{error,events,runtime,scaffold,model/{project_type,config,space,skill_id}}.rs
└─ orchestra-tui/    # frontend ratatui + CLI — consomme orchestra-core
   └─ src/{main,app,dashboard,wizard}.rs
```

## Phase 1 — Modèle + dashboard ✅

Modèle des Espaces de Contexte agnostiques + coquille ASCII du dashboard (3 zones :
en-tête / écran radar / menu). **Sans LLM, sans agent** : le radar est vide.

```bash
# Ouvre l'espace exemple fourni
cargo run -p orchestra-tui -- examples/recherche-immo-aix
# Quitter : q ou Échap
```

Sans argument, l'outil tente de charger un `.orchestra/config.json` du répertoire courant ;
s'il n'en trouve pas, le dashboard s'affiche en état « Aucun espace chargé ».

## Phase 2 — `orchestra init` ✅

Assistant interactif qui génère un Espace de Contexte selon le type de projet
(Dev / Nutrition / Langue / Immobilier), en pré-remplissant la matrice de Skills et
d'agents par défaut.

```bash
# Crée .orchestra/{config.json, persona.md, adr/} dans le dossier cible (défaut : .)
cargo run -p orchestra-tui -- init ./mon-espace
```

Pour un projet **Dev**, l'assistant demande le **workspace** (résolu en chemin absolu) et
propose de configurer les intégrations **Git** et **GitHub** (le token n'est jamais saisi —
seul le *nom* de la variable d'environnement est enregistré). La logique d'écriture vit dans
`orchestra-core::scaffold` (pure, testée) ; les prompts terminal restent dans
`orchestra-tui::wizard` — découplage métier/affichage respecté. Un espace déjà initialisé
n'est jamais écrasé (`SpaceAlreadyExists`).

> 📝 **Pense à remplir le persona** (remplace les « à compléter ») : c'est le contexte donné
> aux agents. Tu peux le faire **directement dans l'interface** avec la touche `[4]`. Si le
> persona est incomplet et qu'un LLM est actif, `[1]` affiche un avertissement plutôt que de
> gaspiller un appel.

## Phase 3 — Runtime d'agents, radar vivant ✅

Le radar n'est plus une coquille. `orchestra-core::runtime::spawn` lance chaque agent de
l'espace comme une tâche `tokio` qui publie des `AgentEvent` sur un canal
`tokio::sync::mpsc` ; le TUI les consomme en direct.

```bash
cargo run -p orchestra-tui -- examples/recherche-immo-aix
# Dans le dashboard : [1] lance l'orchestre → le radar défile en temps réel.  [q] quitte.
```

Boucle async multiplexée par `tokio::select!` (clavier via `EventStream` + flux d'agents +
tick de rafraîchissement). L'agrégation du flux (`orchestra-tui::app::App`) est isolée du
rendu et testée.

## Phase 4a — LLM (Claude **ou** Gemini) + Skills Dev exécutables ✅

Les agents deviennent réellement intelligents. `orchestra-core::llm` appelle l'API d'un
fournisseur d'IA **au choix** en HTTP brut (pas de SDK Rust officiel) ; une représentation
neutre découple la boucle agentique du format de chaque provider. Chaque agent mène une
**boucle agentique** : le modèle raisonne, demande un *outil*, on l'exécute, on lui renvoie
le résultat. Les trois Skills Dev sont branchés sur le système (`orchestra-core::skills`) :
`Read_File`, `Write_File_Validated`, `Execute_Terminal_Command`, confinés au workspace.

```bash
# Claude (défaut claude-opus-4-8) :
export ANTHROPIC_API_KEY="sk-ant-..."
# …ou Gemini (défaut gemini-2.0-flash) :
export GEMINI_API_KEY="..."
# Forcer le fournisseur / le modèle si besoin :
export ORCHESTRA_PROVIDER=gemini      # anthropic | gemini
export ORCHESTRA_MODEL=gemini-2.0-flash

cargo run -p orchestra-tui -- examples/recherche-immo-aix
# [1] lance l'orchestre — le radar affiche les actions réelles des agents.
```

Sélection automatique selon la clé présente (`ANTHROPIC_API_KEY` puis `GEMINI_API_KEY`),
`ORCHESTRA_PROVIDER` ayant priorité. Clés **jamais en dur**, lues depuis l'environnement.

**Repli automatique** : sans aucune clé (ou si l'API est injoignable), on retombe sur les
agents *simulés* de la Phase 3 — l'appli reste pleinement fonctionnelle hors-ligne, et la
compilation/les tests n'exigent aucune clé. L'en-tête indique le mode (`🤖 <modèle>` ou
`simulé · clé API absente`), et le radar rappelle alors quelles variables définir. La
signature de `runtime::spawn` n'a pas changé.

> ⚠️ `Execute_Terminal_Command` exécute des commandes shell dans le workspace (capacité
> assumée pour un IDE de dev) : sortie plafonnée, délai max 30 s, chemins confinés.

## Phase 4b — Intégrations Git + GitHub ✅

`orchestra-core::integrations` expose de nouveaux Skills au LLM — **mais seulement si
l'intégration est configurée** dans l'espace (`config.integrations`) :

- **Git (local)** : `Git_Status`, `Git_Diff`, `Git_Create_Branch`, `Git_Commit` (binaire
  `git` dans le workspace).
- **GitHub (REST)** : `GitHub_List_Issues`, `GitHub_Create_Issue_Comment`,
  `GitHub_Create_Pull_Request` — token lu depuis la variable d'env déclarée (`token_env_var`),
  jamais en dur ; les Skills GitHub ne sont exposés que si ce token est présent.

Exemple de configuration dans `.orchestra/config.json` :

```json
"integrations": {
  "git": { "auto_branching": true, "main_branch": "main" },
  "github": { "repo": "owner/repo", "token_env_var": "GITHUB_TOKEN" }
}
```

```bash
export GITHUB_TOKEN="ghp_..."     # requis pour activer les Skills GitHub
```

> ⚠️ Certaines actions modifient l'état (`Git_Commit`, `Git_Create_Branch`) ou sont
> *sortantes* (création de PR/commentaire) : l'utilisateur les autorise en configurant
> l'intégration dans son espace.

## Phase 5 — Agent Documentaliste + finitions ✅

**Agent Documentaliste.** Si `documentalist_enabled: true` dans la config, un agent dédié
rejoint l'orchestre avec une mission documentation : il lit les fichiers, met à jour la doc
(`Write_File_Validated`) et produit des diagrammes via le nouveau Skill
`Write_Mermaid_Diagram` (écrit un `.md` avec un bloc ` ```mermaid `, type de diagramme
validé). Outils et prompt dédiés, indépendants de la liste de Skills du projet.

### Disposition « cockpit »

Le dashboard est organisé en **multi-panneaux** : une **sidebar « 🎻 Orchestre »** à gauche
affiche en permanence le **statut live de chaque agent** (`○` au repos · spinner *réfléchit*
· `▸` agit · `✔` terminé), à côté de la **zone centrale** (conversation / radar / documents /
agents / éditeur) et d'une **barre de saisie** en bas. La sidebar se masque automatiquement
sur les terminaux étroits.

**Les touches du menu** :

| Touche | Action |
|---|---|
| `[1]` | **Orchestrer un objectif** : tu saisis un but → le chef établit un **plan** (tâches assignées + dépendances), tu l'**approuves** (`Entrée`) ou l'annules (`Échap`), puis les agents s'enchaînent (passage de relais via la mémoire) et une **synthèse** finale est rendue. Le panneau Plan suit l'avancement par tâche |
| `[5]` | **Converser** avec le chef d'orchestre (dialogue continu, voir ci-dessous) |
| `[2]` | **Navigateur de documents** de l'espace (persona, ADRs, docs Markdown) avec **visualiseur Markdown** intégré |
| `[3]` | **Changer d'Espace** (saisie d'un chemin, `Entrée` charge / `Échap` annule) |
| `[4]` | **Éditer le persona** dans l'interface (`Ctrl+S` enregistre, `Échap` annule) |
| `[6]` | **Gérer les agents** : rôle, stats ; renommer/ajouter/supprimer ; `[s]` ouvre le **sélecteur de skills** (catalogue à cocher) |
| `q` / `Échap` | Quitter |

### Gérer les agents (`[6]`)

Chaque agent est désormais un objet **`{ nom, rôle, skills }`** (modèle rétro-compatible avec
les anciennes configs où l'agent n'était qu'un nom). Le menu `[6]` liste les agents et, pour
le sélectionné, affiche **rôle, skills et stats de session** (invocations + temps de
réflexion). Tu peux le rendre modulable :

| Touche | Action |
|---|---|
| `↑`/`↓` | choisir un agent |
| `r` | renommer |
| `o` | modifier le rôle (oriente son prompt) |
| `s` | modifier ses skills (liste séparée par des virgules) |
| `a` | ajouter un agent |
| `d` | supprimer l'agent sélectionné |

Les modifications sont **enregistrées dans `.orchestra/config.json`** (via le cœur,
`ContextSpace::save_config`). Le runtime utilise le **rôle** (dans le prompt système) et les
**skills propres** de chaque agent (repli sur les skills de l'espace s'il n'en a pas).

> 🔌 **Skill exécutable vs étiquette.** Un skill n'agit que s'il est **enregistré** (du code
> Rust derrière). Le menu marque les skills **exécutables** (en vert) ; les autres restent
> des étiquettes « (inactif) ». Le registre (`orchestra-core::skills`) est la source de
> vérité ; ajouter un skill = une entrée au catalogue `EXECUTABLE_SKILLS` + un bras dans
> `tool_definition` et `execute_skill`. Skills exécutables : `Read_File`,
> `Write_File_Validated`, `Execute_Terminal_Command`, `Write_Mermaid_Diagram`, **`Web_Fetch`**
> (lire une URL), + Git/GitHub si l'intégration est configurée.

> 📝 **Skills « fiches » (sans code).** Au-delà des primitives, un skill peut être une simple
> **fiche d'instructions** : un fichier `.orchestra/skills/<id>/SKILL.md` (en-tête `name`/
> `description` + corps Markdown du « comment faire »). Aucune recompilation. **Divulgation
> progressive** (économie de tokens) : le prompt ne porte que le **nom + la description** des
> fiches assignées ; l'agent charge la procédure complète à la demande via la primitive
> **`Load_Skill{id}`**.
>
> **Assignation par sélecteur** (plus besoin de connaître les noms) : menu Agents `[6]` → `[s]`
> ouvre un **catalogue à cocher** listant primitives (`prim.`) et fiches (`fiche`) avec leur
> description ; `Espace` assigne/retire, `[e]` édite une fiche, `[n]` en crée une. Crée-en une
> **depuis l'interface** : menu Agents `[6]` → **`[n]`** → saisis
> un nom → la fiche s'ouvre dans l'éditeur (rédige puis `Ctrl+S`). Le menu marque ces skills
> **(fiche)** en cyan. Idéal pour `Creation_Quiz` (pur texte) ou un `Web_Search` qui s'appuie
> sur la primitive `Web_Fetch`.

> 🧠 **Mémoire partagée + économie de tokens.** Tous les agents disposent de deux primitives
> universelles : **`Remember{note}`** (consigne un fait/décision/synthèse dans
> `.orchestra/memory.md`) et **`Recall{query?}`** (relit la mémoire, filtrée par mot-clé). La
> mémoire est durable entre sessions et visible dans le navigateur `[2]`. C'est aussi un levier
> d'économie : un agent résume une source volumineuse **une fois**, les autres lisent la
> synthèse au lieu de relire le fichier. Le prompt système ne contient qu'un **rappel court** (le
> contenu est lu à la demande via `Recall`). Côté infra, le system prompt — stable d'un tour à
> l'autre — est marqué **cacheable** (Anthropic *prompt caching*) : les tours suivants paient une
> fraction des tokens d'entrée sur ce préfixe.

### Converser avec le chef d'orchestre (`[5]`)

Au-delà de l'exécution autonome (`[1]`), `[5]` ouvre une **conversation persistante** avec
un **agent coordinateur**. Tu écris un message (ligne de saisie en bas, `Entrée` pour
envoyer), il te répond, **peut te poser des questions**, et surtout **délègue aux agents
spécialisés** : chaque agent de l'espace (Tuteur, Correcteur, Documentaliste…) est exposé au
coordinateur comme un *outil* qu'il invoque selon le besoin ; tu vois leur activité défiler,
puis le coordinateur synthétise. L'historique est conservé d'un tour à l'autre. `Échap`
quitte la conversation.

Pour un **objectif complexe**, le coordinateur peut lancer une **orchestration complète** en
pleine conversation (outil `orchestrate`) : il propose un **plan** (que tu approuves avec
`Entrée` / annules avec `Échap`, comme en `[1]`), l'exécute en parallèle, s'auto-corrige, puis
intègre la synthèse dans sa réponse — et le dialogue continue.

```
[5] → « Fais-moi une leçon de 10 min sur les verbes à particule séparable »
   → le coordinateur délègue à Agent_Tuteur, récupère son retour, te répond et te questionne
   → tu réponds → … (conversation continue)
```

Le radar **rend le Markdown** des réponses (titres, listes, citations, blocs de code) et se
**défile** : `PgUp`/`PgDn` (ou `↑`/`↓`) pour remonter dans l'historique de la conversation,
retour automatique en bas à chaque nouveau message. Les interlocuteurs sont colorés : 🟢
**Vous**, 🟣 **Coordinateur**, 🔵 **agents**. Pendant qu'un agent attend le modèle, un
**indicateur animé** « ⠋ {agent} réfléchit… {n}s » (avec temps écoulé) montre ce qui
travaille en arrière-plan. Dans la saisie, **Entrée** envoie et **Maj/Alt+Entrée** insère un
retour à la ligne (message multi-ligne).

Dans le navigateur `[2]` : `↑↓` choisir, `Entrée` ouvrir un document dans le visualiseur
(Markdown rendu : titres, listes, citations, blocs de code, gras/`code`), `↑↓` y défiler,
`Échap` revenir. Sur le persona, `e` ouvre directement l'éditeur. **Tout reste dans l'outil :**
persona éditable, docs et ADRs consultables — la lecture/écriture passe par le cœur
(`ContextSpace::documents` / `load_document` / `save_persona`), l'UI ne touche jamais le
système de fichiers directement.

## Phases suivantes

4c (optionnel). Intégration Jira (même schéma : Skills exposés si configuré).

# Documentation fonctionnelle — Orchestra IDE

> Ce que fait l'outil, pour qui, et comment on s'en sert. Pour les détails d'implémentation,
> voir [`ARCHITECTURE.md`](./ARCHITECTURE.md) ; pour l'avancement, [`JOURNAL.md`](./JOURNAL.md).

## 1. Vision

Orchestra IDE est un **outil de dev piloté par IA** — rien d'autre. Pas un compagnon
généraliste, pas un produit à tiroirs : un poste de pilotage où un **Orchestrateur** (un
agent IA unique, qui déploie sa propre équipe de sous-agents à la volée) fait avancer du code
pour toi, pendant que tu gardes une vue claire et honnête sur l'état réel du projet.

Cinq questions, cinq réponses toujours visibles :

| Question | Pilier |
|---|---|
| Qu'est-ce qui a changé dans mon code ? | **Fichiers** |
| Où j'en suis avec Git ? | **Git** |
| Mon appli tourne, dans Docker ? | **Docker** |
| Qu'est-ce que l'IA est en train de faire, et dans quel ordre ? | **Plan / Tâches** |
| Qui travaille, là, maintenant ? | **Agents** |

## 2. Concept clé : l'Espace de Contexte

Tout part d'un **Espace de Contexte** : un dossier (`.orchestra/`) qui rassemble ce qu'il
faut savoir pour que l'Orchestrateur travaille efficacement sur un projet de code donné.

| Élément | Rôle |
|---|---|
| **Workspace** | Le dossier de code réellement piloté (créé neuf, ou un projet existant « adopté ») |
| **Persona** (`persona.md`) | Contexte et critères rédigés par l'utilisateur (stack, conventions, objectifs…) |
| **Mémoire** (`memory.md`) | Notes partagées entre agents et entre sessions (faits, décisions, synthèses) |
| **ADRs** | Décisions d'architecture structurantes (`adr/*.md`) |
| **Intégrations** | Git / GitHub (actives si configurées) ; Jira déclarable, pas encore implémentée |

**Aucun agent ni skill n'est pré-câblé.** L'Orchestrateur compose son équipe lui-même, tâche
par tâche, en fonction de ce qu'il y a à faire — pas de roster figé à maintenir.

## 3. Parcours utilisateur

### a) Démarrer un projet — nouveau ou existant

```bash
# Nouveau projet : crée .orchestra/{config.json, persona.md, adr/}
cargo run -p orchestra-tui -- init ./mon-projet

# Reprendre un projet de code existant : l'outil l'« adopte » comme Espace de Contexte
cargo run -p orchestra-tui -- ./mon-projet-existant
```

Un Espace déjà initialisé n'est **jamais écrasé**. Le sélecteur d'Espaces (TUI `[3]` /
barre d'espaces du Desktop) garde en mémoire les projets déjà ouverts et propose un
navigateur de dossiers pour en découvrir sans taper de chemin.

### b) Piloter l'Orchestrateur

Deux façons de lui donner du travail, dans les deux UIs :

1. **Objectif rapide** (TUI `[1]`) — tu saisis un but, l'Orchestrateur l'exécute en one-shot
   (déploie les sous-agents utiles, travaille, te rend un compte-rendu).
2. **Assistant** (TUI `[5]` / Desktop, panneau de conversation) — conversation persistante :
   tu discutes, poses des questions, donnes des instructions au fil de l'eau ; l'historique
   est conservé.

Dans les deux cas, l'Orchestrateur **publie et tient à jour un plan visible** (pilier
Plan/Tâches) et **déploie des sous-agents à la volée** (pilier Agents) — tu vois qui fait
quoi, dans quel ordre, en direct.

### c) Les 5 piliers, en détail

**Fichiers** — l'arborescence du workspace, annotée en direct : un fichier s'illumine selon
qu'un agent le **lit** ou l'**écrit**, pendant la session (« orchestre en verre » — tu vois
l'équipe parcourir et modifier le code). Sélectionner un fichier l'ouvre : rendu Markdown
(avec diagrammes Mermaid côté Desktop) ou texte brut, diff s'il a été modifié par un agent
cette session, édition/sauvegarde possible.

**Git** — branche courante, ahead/behind par rapport à l'amont, fichiers indexés/modifiés/non
suivis. Cliquer un fichier modifié affiche son diff. Fonctionne **sans aucune configuration**
dès que le dossier est un dépôt Git — c'est un panneau de lecture, indépendant des outils Git
que l'Orchestrateur peut utiliser lui-même (voir plus bas).

**Docker** — si le projet a un `Dockerfile`/`docker-compose.yml`, l'outil interroge les
conteneurs (image, état, ports) sans rien demander à configurer. **Lecture seule** dans cette
version : c'est un constat, pas un panneau de pilotage (pas de start/stop/logs pour l'instant).
Si Docker n'est pas installé ou son démon injoignable, message neutre — jamais une erreur qui
bloque l'interface.

**Plan / Tâches** — le plan que l'Orchestrateur publie et met à jour au fil de son travail :
liste d'étapes, chacune passant de « en attente » à « en cours » puis « fait » (ou « échec »).
Pas une simple prose dans le chat : un vrai suivi visuel de la progression.

**Agents** — statut live de l'Orchestrateur et de chaque sous-agent qu'il a déployé (repos /
réfléchit / actif / terminé). L'équipe n'est jamais fixée à l'avance : elle apparaît à mesure
que l'Orchestrateur la compose pour la tâche en cours.

S'y ajoutent, complémentaires : **Modifications** (diffs de tout ce que les agents ont écrit
cette session), **Terminal** (commandes réellement exécutées), **Mémoire** (notes partagées
entre agents, durables entre sessions), **Contexte** (fichiers touchés cette session) et
**Docs** (persona/ADRs/mémoire, consultables et éditables sans quitter l'outil).

## 4. État des fonctionnalités

| Capacité | État |
|---|---|
| Espace de Contexte (créer / adopter un projet existant) | ✅ |
| Orchestrateur unique, compose sa propre équipe (`spawn_agent`) | ✅ |
| Objectif rapide (one-shot) + Assistant (conversation persistante) | ✅ |
| Plan / Tâches en direct (`Set_Plan`/`Update_Step`) | ✅ |
| Agents intelligents (LLM Claude **ou** Gemini, bascule automatique) | ✅ avec clé API |
| Repli simulé hors-ligne (sans clé) | ✅ |
| Skills Dev exécutables (fichiers, terminal, Mermaid, web) | ✅ |
| Skills « fiches » Markdown (sans code) + divulgation progressive | ✅ |
| Mémoire partagée d'espace (`Remember`/`Recall`) | ✅ |
| **Panneau Fichiers** (arborescence annotée) | ✅ (TUI + Desktop) |
| **Panneau Git** (branche, ahead/behind, statut, diff) | ✅ (TUI + Desktop) |
| **Panneau Docker** (conteneurs, lecture seule) | ✅ (TUI + Desktop) |
| Panneau Modifications (diffs des agents) | ✅ |
| Panneau Terminal (commandes exécutées) | ✅ |
| Panneau Mémoire / Contexte | ✅ |
| Navigateur de documents (persona/ADR/mémoire) + édition | ✅ |
| Sessions en onglets (plusieurs projets ouverts) | ✅ |
| Registre des espaces connus + navigateur de dossiers | ✅ |
| Intégration Git (outils agent : statut, diff, branche, commit) | ✅ si configuré |
| Intégration GitHub (issues, commentaire, PR) | ✅ si configuré + token |
| Intégration Jira | ❌ (déclarable en config, pas implémentée) |
| Actions Docker (start/stop/logs) | ❌ (volontairement hors périmètre v1, lecture seule) |

## 5. Activer le LLM — Claude, Gemini ou un modèle local (Ollama)

```bash
export ANTHROPIC_API_KEY="sk-ant-..."   # Claude (défaut claude-opus-4-8)
# ou
export GEMINI_API_KEY="..."             # Gemini (défaut gemini-2.5-flash)

# Optionnel : forcer le fournisseur / le modèle
export ORCHESTRA_PROVIDER=gemini        # anthropic | gemini | ollama
export ORCHESTRA_MODEL=gemini-2.5-flash
```

Si les deux clés cloud sont présentes, **Claude est préféré, Gemini sert de repli**
automatique (réseau, surcharge, quota ou crédit épuisé) — sans interrompre l'Orchestrateur.

**Pas envie d'attendre une clé API ?** Un modèle tournant en local via
[Ollama](https://ollama.com) fonctionne aussi bien, sans compte ni clé :

```bash
export ORCHESTRA_PROVIDER=ollama        # modèle qwen2.5-coder par défaut (le plus « code »)
export ORCHESTRA_OLLAMA_MODEL=mistral   # optionnel : un autre modèle déjà tiré (`ollama pull`)
export ORCHESTRA_OLLAMA_TIMEOUT_SECS=600  # optionnel : monte-le si le modèle est lent (CPU)
export ORCHESTRA_OLLAMA_NUM_CTX=8192      # optionnel : fenêtre de contexte (voir note ci-dessous)
```

> ⚠️ Le nom de modèle attendu est le nom **exact** listé par `ollama list` (souvent avec un
> tag, ex. `qwen2.5-coder:7b`) — un nom approximatif renvoie une erreur 404 « model not
> found ».

> ⚠️ **Le modèle « oublie » le début de la conversation, ou faut être anormalement précis pour
> obtenir un résultat correct ?** C'est presque toujours la fenêtre de contexte (`num_ctx`) —
> le défaut Ollama (souvent 2048-4096 selon le modèle) est trop court pour un agent outillé
> (le seul system prompt + les définitions d'outils peuvent déjà l'approcher). Orchestra fixe
> `num_ctx` à **8192** par défaut ; monte `ORCHESTRA_OLLAMA_NUM_CTX` si ta machine a la RAM/VRAM
> pour plus (une fenêtre plus large consomme davantage de mémoire pour le cache du modèle).

Sans forcer de fournisseur, Ollama ne rejoint la chaîne de repli automatique (après
Claude/Gemini) que si `ORCHESTRA_OLLAMA_MODEL` est défini — jamais par défaut, pour ne rien
changer aux installations qui n'ont pas Ollama. Sans aucune clé ni Ollama configuré (ou si
tout échoue), l'outil bascule en **mode simulé**, pleinement utilisable hors-ligne.

> ⚠️ Le Skill `Execute_Terminal_Command` exécute de vraies commandes shell dans le workspace
> — capacité assumée pour un outil de dev, encadrée (confiné au workspace, délai max, sortie
> plafonnée) mais à utiliser en connaissance de cause.

## 6. Activer les intégrations Git / GitHub (outils agent)

Le panneau **Git passif** (branche/diff) ne demande aucune configuration. Pour que
l'**Orchestrateur lui-même** puisse committer, créer une branche, ou agir sur GitHub, il faut
déclarer l'intégration dans `.orchestra/config.json` :

```json
"integrations": {
  "git": { "auto_branching": true, "main_branch": "main" },
  "github": { "repo": "owner/repo", "token_env_var": "GITHUB_TOKEN" }
}
```

```bash
export GITHUB_TOKEN="ghp_..."     # requis pour activer les outils GitHub de l'agent
```

Le modèle ne voit que les outils réellement actionnables : sans intégration configurée (ou
sans token), ils n'apparaissent simplement pas. Jira suivra le même schéma le jour où elle
sera implémentée.

## 7. Démarrage

Aucun espace n'est fourni : ouvre un dossier de projet existant (adopté automatiquement) ou
lance `orchestra init` pour en créer un neuf. L'Orchestrateur déploie ensuite sa propre
équipe à la volée — rien à pré-configurer côté agents ou skills.

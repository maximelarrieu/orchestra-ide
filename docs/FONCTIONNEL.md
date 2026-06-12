# Documentation fonctionnelle — Orchestra IDE

> Ce que fait l'outil, pour qui, et comment on s'en sert. Pour les détails d'implémentation,
> voir [`ARCHITECTURE.md`](./ARCHITECTURE.md) ; pour l'avancement, [`JOURNAL.md`](./JOURNAL.md).

## 1. Vision

Orchestra IDE est un **« IDE pour l'ère agentique »** : un poste de pilotage où l'on ne
manipule pas du code ligne à ligne, mais un **orchestre d'agents** qui travaillent pour
nous sur un objectif. L'outil est **agnostique du domaine** : il sert aussi bien à
développer un logiciel qu'à organiser une recherche immobilière, un plan nutritionnel ou
l'apprentissage d'une langue.

## 2. Concept clé : l'Espace de Contexte

Tout part d'un **Espace de Contexte** : un dossier qui rassemble *tout ce qu'il faut
savoir* pour qu'un orchestre d'agents travaille sur un sujet donné.

Un Espace contient :

| Élément | Rôle |
|---|---|
| **Type de projet** | `Dev`, `Nutrition`, `Langue` ou `Immobilier` — détermine les agents et Skills par défaut |
| **Persona** (`persona.md`) | Le contexte et les critères rédigés par l'utilisateur (budget, régime, niveau, conventions de code…) |
| **Agents** | Les membres de l'orchestre (ex. `Agent_Scraper`, `Agent_Codeur`) |
| **Skills** | Les capacités activées (ex. `Scrape_Web_Page`, `Read_File`) |
| **ADRs** | Les décisions structurantes consignées (`adr/*.md`) |
| **Intégrations** | Git / GitHub / Jira (à venir, Phase 4) |

### Matrice des types de projet

| Type | Agents par défaut | Skills par défaut |
|---|---|---|
| **Dev** | Agent_Architecte, Agent_Codeur, Agent_Testeur | Read_File, Write_File_Validated, Execute_Terminal_Command |
| **Nutrition** | Agent_Planificateur, Agent_Nutritionniste | Web_Search, Calorie_Calculator, File_Append |
| **Langue** | Agent_Tuteur, Agent_Correcteur | Generate_Quiz, Translate_Text, Text_To_Speech |
| **Immobilier** | Agent_Scraper, Agent_Filtrage | Scrape_Web_Page, Extract_JSON_From_HTML, Geocoding_Calcul |

## 3. Parcours utilisateur

### a) Créer un Espace — `orchestra init`

```bash
cargo run -p orchestra-tui -- init ./ma-recherche
```

Un assistant interactif pose quelques questions :

```mermaid
flowchart TD
    A([orchestra init chemin]) --> B[Nom du projet ?]
    B --> C[Type ? 1.Dev 2.Nutrition 3.Langue 4.Immobilier]
    C --> D{Type = Dev ?}
    D -->|oui| E[Chemin du code à piloter ?]
    D -->|non| F[Agent Documentaliste ? o/N]
    E --> F
    F --> G[[Génère .orchestra/ : config.json + persona.md + adr/]]
    G --> H([Espace prêt — complète persona.md])
```

Le résultat est un dossier `.orchestra/` pré-rempli avec les agents et Skills adaptés au
type choisi. Un Espace déjà existant **n'est jamais écrasé**.

### b) Piloter l'orchestre — le tableau de bord

```bash
cargo run -p orchestra-tui -- ./ma-recherche
```

Le tableau de bord (TUI) s'ouvre en 3 zones :

```
┌─ ORCHESTRA IDE v0.1.0 | [Recherche_Immo_Aix] (Immobilier) | ● au repos ──┐
├─ 🛰  ÉCRAN RADAR (FLUX D'ACTIVITÉ DES AGENTS) ───────────────────────────┤
│   Prêt. Appuie sur [1] pour lancer l'orchestre.                          │
├─ 📋 OPTIONS & MENUS ─────────────────────────────────────────────────────┤
│  [1] Lancer l'orchestre  [2] Voir les ADRs  [3] Changer d'Espace  [q]…   │
└──────────────────────────────────────────────────────────────────────────┘
```

| Touche | Action | État |
|---|---|---|
| `[1]` | Lancer l'orchestre → le radar défile en temps réel | ✅ actif |
| `[2]` | Voir les ADRs | ⏳ à venir |
| `[3]` | Changer d'Espace | ⏳ à venir |
| `q` / `Échap` | Quitter | ✅ actif |

Quand l'orchestre tourne, l'en-tête indique `▶ N agent(s) en cours`, le radar liste les
démarrages, les logs et les fins d'agents, puis bascule en `✓ terminé`.

## 4. État des fonctionnalités

| Capacité | État | Phase |
|---|---|---|
| Modèle d'Espace de Contexte agnostique | ✅ | 1 |
| Tableau de bord 3 zones | ✅ | 1 |
| Création d'Espace assistée (`init`) | ✅ | 2 |
| Radar temps réel (flux d'agents) | ✅ | 3 |
| **Agents intelligents (LLM Claude)** | ✅ avec clé API | 4a |
| Skills Dev exécutables (lecture/écriture fichier, terminal) | ✅ | 4a |
| Repli simulé hors-ligne (sans clé) | ✅ | 4a |
| Intégrations Git / GitHub / Jira | ❌ | 4b |
| Consultation des ADRs / changement d'Espace dans l'UI | ❌ | 4b–5 |
| Agent Documentaliste (doc auto, Mermaid) | ❌ | 5 |

### Activer le LLM

Les agents appellent réellement Claude dès qu'une clé API est exposée :

```bash
export ANTHROPIC_API_KEY="sk-ant-..."     # lue depuis l'environnement, jamais en dur
cargo run -p orchestra-tui -- examples/recherche-immo-aix
```

L'en-tête du dashboard affiche le mode : `🤖 claude-opus-4-8` quand le LLM est actif,
`simulé` sinon. **Sans clé (ou si l'API est injoignable), l'appli bascule automatiquement
en mode simulé** — elle reste pleinement utilisable hors-ligne.

> ⚠️ Le Skill `Execute_Terminal_Command` exécute de vraies commandes shell dans le
> workspace. C'est une capacité assumée pour un IDE de développement, encadrée (workspace
> uniquement, délai max, sortie plafonnée) — mais à utiliser en connaissance de cause.

## 5. Exemple fourni

`examples/recherche-immo-aix/` est un Espace Immobilier prêt à ouvrir pour découvrir le
tableau de bord et le radar sans rien créer.

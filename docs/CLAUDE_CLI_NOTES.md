# Notes sur le CLI `claude`

Ce que l'on a vérifié sur la machine, version par version. À compléter à chaque
découverte : le format du flux est notre dépendance la plus fragile.

## Version observée

`claude 2.1.276`, authentification par abonnement (OAuth), aucun `ANTHROPIC_API_KEY`.

## Mode headless

```sh
claude -p "<prompt>" --output-format stream-json --verbose [--input-format stream-json]
```

Émet du NDJSON :

| Ligne | Contenu utile |
|---|---|
| `system` / `init` | `session_id`, `model`, `tools`, `cwd`, `claude_code_version`, `permissionMode` |
| `assistant` | `message.id` (l'identifiant API `msg_…`), `message.model`, `message.usage`, `parent_tool_use_id` |
| `user` | résultats d'outils |
| `result` | `subtype`, `is_error`, `result`, `total_cost_usd`, `usage`, `modelUsage`, `num_turns`, `duration_ms`, `permission_denials`, `subagent_stats` |

`message.usage` contient `input_tokens`, `output_tokens`, `cache_creation_input_tokens`,
`cache_read_input_tokens` et `output_tokens_details.thinking_tokens`.

**À confirmer en phase 2** : où atterrit exactement la sortie de `--json-schema`
(champ `structured_output` du `result`, ou `result` à parser en JSON) et le nom exact
des `subtype` d'erreur (`error_max_turns`, budget dépassé…).

## Pilotage

- Message utilisateur sur stdin, une ligne :
  `{"type":"user","message":{"role":"user","content":"…"}}`
- `SIGINT` annule proprement le tour en cours.
- **À confirmer en phase 3** : est-ce que `-p` sans prompt positionnel attend bien
  stdin, et le comportement d'un second message envoyé pendant un tour.

## Sessions d'arrière-plan

`claude --bg` est incompatible avec `-p` et n'offre aucun moyen non interactif
d'envoyer un message. On ne l'utilise donc pas pour les agents pilotés ;
`claude --resume <session_id>` sert de porte de sortie « reprendre en manuel ».

## Transcripts

`~/.claude/projects/<slug-du-cwd>/<session_id>.jsonl`, où le slug est le chemin de
travail avec les caractères non alphanumériques remplacés par des tirets. Les
sous-agents ont leur propre fichier dans `<session_id>/subagents/agent-<id>.jsonl`.

Chaque ligne `assistant` porte `message.id`, `message.model`, `message.usage`,
`sessionId`, `cwd`, `gitBranch`, `timestamp`, `uuid`, `parentUuid`, `requestId`,
`isSidechain`, `agentId`, `version`.

Trois pièges confirmés, mesurés sur 19 fichiers et 4 900 lignes :

1. **Une réponse API produit plusieurs lignes `assistant`** qui partagent le même
   `message.id`, une par bloc de contenu, distinguées par `apiBlockIndex`. Sur ce
   corpus : 1 469 lignes pour 637 réponses, jusqu'à 14 lignes pour une seule réponse.
   Compter chaque ligne multiplierait les coûts.
2. **Pire : ces copies ne portent pas le même `usage`.** Les premiers blocs sont
   écrits pendant que la réponse est en cours et annoncent un `output_tokens`
   partiel ; seul le dernier a le total. Exemple réel, une seule réponse :

   | bloc | output_tokens | iterations |
   |---|---|---|
   | 0 | 5 | 0 |
   | 1 | 5 | 0 |
   | 2 | 5 | 0 |
   | 3 | 787 | 1 |

   Le même `apiBlockIndex` peut même apparaître deux fois, une version partielle et
   une complète. Garder la première copie sous-estimerait le coût d'un facteur cent.
   Orchestra garde donc le **maximum par champ** : l'opération est commutative, donc
   l'ordre d'arrivée du flux et du transcript n'a pas d'importance. 124 réponses sur
   637 étaient concernées sur ce corpus.
3. Les fichiers contiennent une trentaine de types de lignes qui ne sont pas des
   messages (`attachment`, `ai-title`, `cost-state`, `file-history-snapshot`…).
   Seules les lignes `assistant` portent un `usage`. Un pré-filtre sur la chaîne
   `"type":"assistant"` évite de parser les quatre cinquièmes du fichier.

Les lignes de sous-agent portent le `sessionId` **du parent**, plus `agentId`,
`isSidechain: true` et `attributionAgent` (le type de sous-agent). Les coûts d'un
sous-agent remontent donc naturellement à la session qui l'a lancé.

Une ligne `assistant` peut avoir `model: "<synthetic>"` : ce sont des messages
fabriqués localement par Claude Code (erreurs, avis), sans coût réel.

## Tarifs : le cache est facturé à l'heure

Mesure sur un appel réel (`claude -p --model haiku --output-format json`) :

```
usage   : input 10, output 43, cache_read 14053, cache_creation 8084
rapporté: total_cost_usd = 0.0177983
```

En résolvant, le tarif d'écriture de cache vaut exactement **2,0 fois le prix
d'entrée**, soit le tarif du cache d'une heure, et non 1,25 fois (cinq minutes).
Les transcripts le confirment : `cache_creation.ephemeral_1h_input_tokens` domine
largement `ephemeral_5m_input_tokens`. La grille par défaut d'Orchestra utilise donc
le tarif d'une heure ; avec celui de cinq minutes, l'estimation était un tiers trop
basse. Un test rejoue cette mesure pour que la grille ne dérive pas.

Les sessions **interactives** écrivent le même `usage` : Orchestra peut donc
comptabiliser toute l'activité Claude Code de la machine, pas seulement la sienne.

## Hooks

Passés par session via `--settings '<json>'`, sans toucher `~/.claude/settings.json`.
La charge utile arrive sur stdin et contient toujours `session_id`, `cwd` et
`hook_event_name`. Une sortie 2 avec un motif sur stderr refuse l'appel d'outil.

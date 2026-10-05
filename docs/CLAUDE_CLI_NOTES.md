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

### Sortie structurée (`--json-schema`) — confirmé

La ligne `result` porte **deux** copies de la réponse :

- `structured_output` : l'objet déjà analysé, conforme au schéma ;
- `result` : le même JSON sous forme de chaîne.

Orchestra lit `structured_output` et retombe sur `result` analysé en JSON si le champ
manque, ce qui couvre une version antérieure ou une future disparition du champ.

Le `result` complet expose aussi `stop_reason`, `terminal_reason`, `num_turns`,
`total_cost_usd`, `usage`, `modelUsage`, `permission_denials`, `subagent_stats`,
`duration_ms`, `duration_api_ms` et quelques mesures de latence (`ttft_ms`…).

**Le schéma ne contraint pas les valeurs par lui-même.** Sur un essai avec un champ
`role` libre, le modèle a inventé « Backend Engineer » et « Performance Engineer »
plutôt que de reprendre les rôles du catalogue. Orchestra génère donc le schéma avec
un `enum` des rôles réellement disponibles, et valide de toute façon la réponse avant
de l'accepter.

Autre observation : le flux contient des lignes `system` de sous-type inattendu, par
exemple `thinking_tokens`. Le parseur les ignore sans broncher, ce qui est exactement
la raison pour laquelle il est tolérant.

**Reste à confirmer en phase 3** : le nom exact des `subtype` d'erreur
(`error_max_turns`, dépassement de budget…).

## Pilotage — confirmé

- `-p` **sans prompt positionnel attend bien stdin** quand `--input-format stream-json`
  est passé. Le message tient sur une ligne :
  `{"type":"user","message":{"role":"user","content":"…"}}`
- **L'entrée reste ouverte** : le processus attend d'autres tours. Un appel qui ne
  sera jamais piloté doit fermer stdin, sinon il ne se termine jamais et sa sortie
  standard n'atteint jamais la fin de fichier.
- **`SIGINT` produit une ligne `result` propre**, puis le processus sort avec le code 0 :

  ```
  subtype        : error_during_execution
  is_error       : true
  stop_reason    : tool_use
  terminal_reason: aborted_streaming
  ```

  Une interruption ressemble donc à un échec. C'est le superviseur, qui sait qu'il a
  envoyé le signal, qui tranche entre « annulé » et « échoué » ; le flux ne le dit pas.

## Hooks — confirmé

Passés par session avec `--settings '<json>'`, sans toucher `~/.claude/settings.json`.
**L'environnement du processus est hérité** par le hook, ce qui permet de lui passer
le périmètre du ticket sans fichier de configuration.

Charge utile reçue sur l'entrée standard, pour un `PreToolUse` sur Bash :

```json
{"session_id":"…","transcript_path":"…","cwd":"/tmp/spike3",
 "permission_mode":"bypassPermissions","hook_event_name":"PreToolUse",
 "tool_name":"Bash","tool_input":{"command":"cat fichier.txt","description":"…"},
 "tool_use_id":"toolu_…"}
```

**Une sortie 2 avec un motif sur stderr refuse l'appel.** L'agent le voit arriver
comme un `tool_result` en erreur dont le texte est
`PreToolUse:Bash hook error: [<commande>]: <motif>`, et le `result` final compte le
refus dans `permission_denials`. Orchestra reconnaît ce marqueur dans le flux pour
afficher un événement « bloqué » : aucun aller-retour supplémentaire n'est nécessaire.

Conséquence de conception : **le garde décide localement**, à partir de son
environnement et de la charge utile, sans parler au daemon. Il tourne à chaque appel
d'outil de chaque agent ; un aller-retour sur socket y ajouterait de la latence et
ferait dépendre l'agent de la santé du daemon.

Le flux contient aussi des lignes `system` de sous-type `hook_started` et
`hook_response`, mais seulement pour certains hooks sans `--include-hook-events`.
On ne s'appuie donc pas dessus.

### Sortie structurée dans une session à outils — confirmé (2.1.289)

`--json-schema` tient aussi pour un agent qui lit, écrit et lance des commandes,
prompt reçu en `--input-format stream-json` sur stdin : les outils tournent
normalement, puis la ligne `result` porte `structured_output`. Le relecteur s'en
sert pour son verdict ; fixture réelle : `tests/fixtures/reviewer_result.jsonl`.

### `--max-turns` — confirmé (2.1.289), absent de `--help`

Accepté et appliqué : une limite dépassée termine la ligne `result` par
`subtype: "error_max_turns"`, `terminal_reason: "max_turns"`, `is_error: true`.
Observé avec `--max-turns 1` : `num_turns` vaut 2 à l'arrêt, le tour d'outil en
cours étant compté.

## Isolation de l'environnement utilisateur — confirmé (2.1.289)

Sans précaution, un agent hérite de tout `~/.claude` : plugins, leurs hooks, serveurs
MCP, skills. Mesuré sur le message `system/init`, sur la machine de développement :

| | défaut | `--setting-sources project --strict-mcp-config` |
|---|---|---|
| outils | 86 | 30 |
| serveurs MCP | 11 | 0 |
| skills | 326 | 19 |
| hooks utilisateur (`SessionStart`, `Stop`) | déclenchés | aucun |

Les plugins qui restent sont ceux livrés ou imposés par l'administration (`managed`).
**Un hook passé par `--settings` se déclenche toujours** sous ces deux drapeaux :
le garde d'Orchestra n'en dépend pas. `ClaudeCommand::isolated`, vrai par défaut,
pose les deux drapeaux pour l'orchestrateur comme pour les agents.

Écartés : `--bare` coupe aussi l'OAuth (seul `ANTHROPIC_API_KEY` reste),
`--restricted` refuse `bypassPermissions` et retire Bash, `--safe-mode` désactive
tous les hooks, garde compris.

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

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

Deux pièges confirmés :

1. **Une réponse API produit plusieurs lignes `assistant`** qui partagent le même
   `message.id` (une par bloc de contenu, distinguées par `apiBlockIndex`). Compter
   chaque ligne multiplierait les coûts. C'est pourquoi `usage_samples.message_id`
   est une clé primaire.
2. Les fichiers contiennent une trentaine de types de lignes qui ne sont pas des
   messages (`attachment`, `ai-title`, `file-history-snapshot`…). Seules les lignes
   `assistant` portent un `usage`.

Les sessions **interactives** écrivent le même `usage` : Orchestra peut donc
comptabiliser toute l'activité Claude Code de la machine, pas seulement la sienne.

## Hooks

Passés par session via `--settings '<json>'`, sans toucher `~/.claude/settings.json`.
La charge utile arrive sur stdin et contient toujours `session_id`, `cwd` et
`hook_event_name`. Une sortie 2 avec un motif sur stderr refuse l'appel d'outil.

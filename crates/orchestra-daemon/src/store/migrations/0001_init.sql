-- Orchestra v2, initial schema.
-- Every table is written only by the daemon, in a single process.

CREATE TABLE projects (
  id             TEXT PRIMARY KEY,
  name           TEXT NOT NULL,
  path           TEXT NOT NULL UNIQUE,
  default_branch TEXT NOT NULL DEFAULT 'main',
  zellij_tab     TEXT,
  kind           TEXT NOT NULL CHECK (kind IN ('managed', 'discovered')),
  created_at     TEXT NOT NULL
);

CREATE TABLE tickets (
  id            TEXT PRIMARY KEY,
  project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  number        INTEGER NOT NULL,
  title         TEXT NOT NULL,
  brief         TEXT NOT NULL,
  status        TEXT NOT NULL,
  branch        TEXT,
  worktree_path TEXT,
  proposal_json TEXT,
  team_json     TEXT,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  UNIQUE (project_id, number)
);
CREATE INDEX tickets_project_status ON tickets(project_id, status);

CREATE TABLE agents (
  id             TEXT PRIMARY KEY,
  ticket_id      TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  project_id     TEXT NOT NULL,
  role           TEXT NOT NULL,
  objective      TEXT NOT NULL,
  stage          INTEGER NOT NULL,
  session_id     TEXT NOT NULL UNIQUE,
  model          TEXT NOT NULL,
  effort         TEXT NOT NULL,
  max_budget_usd REAL,
  status         TEXT NOT NULL,
  exit_reason    TEXT,
  pid            INTEGER,
  pane_id        TEXT,
  attempt        INTEGER NOT NULL DEFAULT 1,
  handoff        TEXT,
  started_at     TEXT,
  ended_at       TEXT
);
CREATE INDEX agents_ticket ON agents(ticket_id, stage);
CREATE INDEX agents_status ON agents(status);

-- Append-only spine. `seq` is what clients resume from.
CREATE TABLE events (
  seq        INTEGER PRIMARY KEY AUTOINCREMENT,
  ts         TEXT NOT NULL,
  project_id TEXT,
  ticket_id  TEXT,
  agent_id   TEXT,
  kind       TEXT NOT NULL,
  payload    TEXT NOT NULL
);
CREATE INDEX events_agent ON events(agent_id, seq);
CREATE INDEX events_ticket ON events(ticket_id, seq);
CREATE INDEX events_kind ON events(kind, seq);

-- Every Claude Code session seen on this machine, ours or not.
CREATE TABLE sessions (
  session_id     TEXT PRIMARY KEY,
  cwd            TEXT NOT NULL,
  project_id     TEXT,
  agent_id       TEXT,
  managed        INTEGER NOT NULL DEFAULT 0,
  name           TEXT,
  first_seen     TEXT NOT NULL,
  last_seen      TEXT NOT NULL,
  claude_version TEXT
);
CREATE INDEX sessions_project ON sessions(project_id);

-- One row per API response. `message_id` dedupes the stdout stream against the
-- transcript, and the transcript's several content-block lines against itself.
CREATE TABLE usage_samples (
  message_id     TEXT PRIMARY KEY,
  session_id     TEXT NOT NULL,
  subagent_id    TEXT,
  agent_id       TEXT,
  ticket_id      TEXT,
  project_id     TEXT,
  model          TEXT NOT NULL,
  ts             TEXT NOT NULL,
  source         TEXT NOT NULL,
  input          INTEGER NOT NULL,
  output         INTEGER NOT NULL,
  cache_read     INTEGER NOT NULL,
  cache_creation INTEGER NOT NULL,
  thinking       INTEGER NOT NULL
);
CREATE INDEX usage_ts ON usage_samples(ts);
CREATE INDEX usage_agent ON usage_samples(agent_id);
CREATE INDEX usage_ticket ON usage_samples(ticket_id);
CREATE INDEX usage_project_model ON usage_samples(project_id, model);
CREATE INDEX usage_session ON usage_samples(session_id);

-- Watcher cursors, so a restart does not re-read every transcript.
CREATE TABLE transcript_files (
  path        TEXT PRIMARY KEY,
  inode       INTEGER,
  offset      INTEGER NOT NULL DEFAULT 0,
  session_id  TEXT,
  subagent_id TEXT,
  last_seen   TEXT NOT NULL
);

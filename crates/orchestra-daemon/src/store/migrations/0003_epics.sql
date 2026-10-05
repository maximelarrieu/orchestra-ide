-- Epics: one request the orchestrator splits into several tickets, each with
-- its own team and worktree. The link lives here rather than on `tickets`,
-- so a ticket stays what it was and an epic is only an ordering over them.

CREATE TABLE epics (
  id            TEXT PRIMARY KEY,
  project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  title         TEXT NOT NULL,
  brief         TEXT NOT NULL,
  status        TEXT NOT NULL CHECK (status IN ('draft', 'split', 'active', 'done', 'cancelled')),
  proposal_json TEXT,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);
CREATE INDEX epics_project ON epics(project_id, status);

-- A ticket belongs to at most one epic, at one position, and waits for the
-- tickets of that epic listed in `depends_on` (a JSON array of ticket ids).
CREATE TABLE epic_tickets (
  ticket_id  TEXT PRIMARY KEY REFERENCES tickets(id) ON DELETE CASCADE,
  epic_id    TEXT NOT NULL REFERENCES epics(id) ON DELETE CASCADE,
  position   INTEGER NOT NULL,
  depends_on TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX epic_tickets_epic ON epic_tickets(epic_id, position);

-- Personal todos: ideas or tasks, unrelated to any project.

CREATE TABLE todos (
  id                 TEXT PRIMARY KEY,
  title              TEXT NOT NULL,
  notes              TEXT NOT NULL DEFAULT '',
  status             TEXT NOT NULL CHECK (status IN ('open', 'in_progress', 'done', 'dropped')),
  urgent             INTEGER NOT NULL DEFAULT 0,
  due_at             TEXT,
  promoted_ticket_id TEXT REFERENCES tickets(id) ON DELETE SET NULL,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);
CREATE INDEX todos_status ON todos(status);
CREATE INDEX todos_due ON todos(due_at);

-- Events can now scope to a todo, alongside project/ticket/agent.
ALTER TABLE events ADD COLUMN todo_id TEXT;
CREATE INDEX events_todo ON events(todo_id, seq);

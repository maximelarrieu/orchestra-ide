//! Schema migrations.
//!
//! `user_version` holds the applied version. Migrations are embedded, applied
//! in order inside one transaction each, and never edited once shipped.

use anyhow::{Context, Result};
use rusqlite::Connection;

struct Migration {
    version: i32,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "todos",
        sql: include_str!("migrations/0002_todos.sql"),
    },
];

/// Highest version this binary knows about.
pub fn latest_version() -> i32 {
    MIGRATIONS.last().map(|m| m.version).unwrap_or(0)
}

pub fn current_version(conn: &Connection) -> Result<i32> {
    let v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(v)
}

/// Apply everything newer than `user_version`. Returns the versions applied.
pub fn migrate(conn: &mut Connection) -> Result<Vec<i32>> {
    let from = current_version(conn)?;
    if from > latest_version() {
        anyhow::bail!(
            "la base est en version {from}, ce binaire ne connaît que la version {}. \
             Mets Orchestra à jour.",
            latest_version()
        );
    }
    let mut applied = Vec::new();
    for m in MIGRATIONS.iter().filter(|m| m.version > from) {
        let tx = conn.transaction()?;
        tx.execute_batch(m.sql)
            .with_context(|| format!("migration {:04} ({})", m.version, m.name))?;
        // PRAGMA does not accept a bound parameter.
        tx.pragma_update(None, "user_version", m.version)?;
        tx.commit()?;
        tracing::info!(version = m.version, name = m.name, "migration appliquée");
        applied.push(m.version);
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let mut c = mem();
        let first = migrate(&mut c).unwrap();
        assert_eq!(first, vec![1, 2]);
        assert_eq!(current_version(&c).unwrap(), latest_version());
        let second = migrate(&mut c).unwrap();
        assert!(second.is_empty());
    }

    #[test]
    fn schema_has_every_table() {
        let mut c = mem();
        migrate(&mut c).unwrap();
        let mut stmt = c
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            tables,
            vec![
                "agents",
                "events",
                "projects",
                "sessions",
                "tickets",
                "todos",
                "transcript_files",
                "usage_samples",
            ]
        );
    }

    #[test]
    fn a_newer_database_is_refused() {
        let mut c = mem();
        migrate(&mut c).unwrap();
        c.pragma_update(None, "user_version", latest_version() + 5)
            .unwrap();
        let err = migrate(&mut c).unwrap_err();
        assert!(
            err.to_string().contains("mets orchestra à jour")
                || err.to_string().contains("Mets Orchestra à jour")
        );
    }
}

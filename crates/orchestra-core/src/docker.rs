//! État Docker du workspace, en **lecture seule** — panneau Docker des deux UIs.
//!
//! Détecte si le projet a un `Dockerfile` et/ou un fichier `docker-compose`, et si le
//! démon Docker est joignable interroge les conteneurs du projet via
//! `docker compose ps`. Aucune action (start/stop/logs) n'est exposée dans cette version :
//! c'est un panneau de constat, pas de pilotage. Logique pure et partagée TUI ⇄ GUI.
//!
//! Ne dépend d'aucun outil externe manquant : si `docker` n'est pas installé, ou si son
//! démon n'est pas joignable, [`detect`] renvoie un état neutre (`docker_available:
//! false`) — jamais d'erreur bloquante pour l'UI.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

const DOCKER_TIMEOUT: Duration = Duration::from_secs(10);

const COMPOSE_FILENAMES: &[&str] =
    &["docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"];

/// Un conteneur du projet (ligne de `docker compose ps`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DockerContainer {
    pub name: String,
    /// Nom du service `docker-compose.yml`, si connu.
    pub service: Option<String>,
    pub image: String,
    /// État brut Docker (`running`, `exited`, `restarting`…).
    pub state: String,
    /// Texte de statut lisible (ex. `Up 3 minutes (healthy)`).
    pub status_text: String,
    /// Ports publiés, formatés pour l'affichage (ex. `8080->80`).
    pub ports: String,
}

/// État Docker du projet, pour affichage direct.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DockerStatus {
    /// Vrai si le binaire `docker` est présent ET son démon joignable.
    pub docker_available: bool,
    pub dockerfile_present: bool,
    pub compose_file: Option<PathBuf>,
    /// Vide si pas de fichier compose, si le démon est indisponible, ou si l'appel échoue.
    pub containers: Vec<DockerContainer>,
}

/// Détecte l'état Docker de `root`. Ne bloque jamais et ne remonte jamais d'erreur :
/// toute indisponibilité (binaire absent, démon injoignable, pas de fichier compose)
/// se traduit par des champs à leur valeur neutre.
pub async fn detect(root: &Path) -> DockerStatus {
    let dockerfile_present = root.join("Dockerfile").is_file();
    let compose_file = find_compose_file(root);
    let docker_available = docker_daemon_reachable().await;

    let containers = match (docker_available, &compose_file) {
        (true, Some(file)) => compose_ps(file, root).await.unwrap_or_default(),
        _ => Vec::new(),
    };

    DockerStatus { docker_available, dockerfile_present, compose_file, containers }
}

fn find_compose_file(root: &Path) -> Option<PathBuf> {
    COMPOSE_FILENAMES.iter().map(|f| root.join(f)).find(|p| p.is_file())
}

async fn docker_daemon_reachable() -> bool {
    let mut cmd = Command::new("docker");
    cmd.args(["version", "--format", "{{.Server.Version}}"]);
    matches!(timeout(DOCKER_TIMEOUT, cmd.output()).await, Ok(Ok(out)) if out.status.success())
}

async fn compose_ps(compose_file: &Path, root: &Path) -> Option<Vec<DockerContainer>> {
    let mut cmd = Command::new("docker");
    cmd.args(["compose", "-f"])
        .arg(compose_file)
        .args(["ps", "--format", "json"])
        .current_dir(root);
    let out = timeout(DOCKER_TIMEOUT, cmd.output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_compose_ps(&String::from_utf8_lossy(&out.stdout)))
}

/// `docker compose ps --format json` renvoie soit un tableau JSON, soit du NDJSON (un objet
/// par ligne) selon la version de Compose — on gère les deux.
fn parse_compose_ps(text: &str) -> Vec<DockerContainer> {
    if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) {
        return items.iter().map(container_from_json).collect();
    }
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .map(|v| container_from_json(&v))
        .collect()
}

fn container_from_json(v: &Value) -> DockerContainer {
    let str_field = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let ports = v
        .get("Ports")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| v.get("Publishers").and_then(Value::as_array).map(|p| format_publishers(p)))
        .unwrap_or_default();
    DockerContainer {
        name: str_field("Name"),
        service: v.get("Service").and_then(Value::as_str).map(str::to_string),
        image: str_field("Image"),
        state: str_field("State"),
        status_text: str_field("Status"),
        ports,
    }
}

fn format_publishers(list: &[Value]) -> String {
    list.iter()
        .filter_map(|p| {
            let target = p.get("TargetPort").and_then(Value::as_u64)?;
            let published = p.get("PublishedPort").and_then(Value::as_u64).unwrap_or(0);
            Some(if published == 0 { format!("{target}") } else { format!("{published}->{target}") })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_dockerfile_and_compose_file() {
        let dir = std::env::temp_dir().join(format!("orch-docker-detect-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Dockerfile"), "FROM scratch").unwrap();
        fs::write(dir.join("docker-compose.yml"), "services: {}").unwrap();

        assert!(dir.join("Dockerfile").is_file());
        assert_eq!(find_compose_file(&dir), Some(dir.join("docker-compose.yml")));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_compose_file_when_absent() {
        let dir = std::env::temp_dir().join(format!("orch-docker-nocompose-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert_eq!(find_compose_file(&dir), None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_ndjson_compose_ps() {
        let text = r#"{"Name":"app-web-1","Service":"web","Image":"app:latest","State":"running","Status":"Up 2 minutes","Publishers":[{"TargetPort":80,"PublishedPort":8080}]}
{"Name":"app-db-1","Service":"db","Image":"postgres:16","State":"exited","Status":"Exited (0)"}"#;
        let containers = parse_compose_ps(text);
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0].name, "app-web-1");
        assert_eq!(containers[0].service.as_deref(), Some("web"));
        assert_eq!(containers[0].ports, "8080->80");
        assert_eq!(containers[1].state, "exited");
        assert_eq!(containers[1].ports, "");
    }

    #[test]
    fn parses_json_array_compose_ps() {
        let text = r#"[{"Name":"app-web-1","Service":"web","Image":"app:latest","State":"running","Status":"Up","Ports":"0.0.0.0:8080->80/tcp"}]"#;
        let containers = parse_compose_ps(text);
        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0].ports, "0.0.0.0:8080->80/tcp");
    }

    #[tokio::test]
    async fn detect_never_panics_without_docker_or_project_files() {
        let dir = std::env::temp_dir().join(format!("orch-docker-detect-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let status = detect(&dir).await;
        assert!(!status.dockerfile_present);
        assert!(status.compose_file.is_none());
        assert!(status.containers.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}

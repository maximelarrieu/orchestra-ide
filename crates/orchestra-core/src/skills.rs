//! Skills exécutables de l'écosystème Dev (Phase 4a).
//!
//! Trois Skills sont réellement branchés sur le système, exposés au LLM comme *tools*
//! (tool use) : `Read_File`, `Write_File_Validated`, `Execute_Terminal_Command`. Le
//! modèle demande un outil, on l'exécute ici, on lui renvoie le résultat.
//!
//! **Frontière de sécurité.** Lectures/écritures sont confinées au `workspace` (chemins
//! absolus et composants `..` refusés). `Execute_Terminal_Command` exécute une commande
//! shell dans le workspace, avec délai maximal et sortie plafonnée — capacité puissante,
//! assumée pour un IDE de développement piloté par l'utilisateur. Les Skills non-Dev ne
//! sont pas exécutables à ce stade.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::timeout;

use crate::llm::ToolSpec;

/// Identifiants des Skills Dev branchés sur le système.
pub const READ_FILE: &str = "Read_File";
pub const WRITE_FILE: &str = "Write_File_Validated";
pub const EXEC_COMMAND: &str = "Execute_Terminal_Command";

const MAX_OUTPUT: usize = 12_000; // plafond de caractères renvoyés au modèle
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Résultat d'un Skill renvoyé au modèle (texte + drapeau d'erreur pour `tool_result`).
pub struct SkillOutcome {
    pub text: String,
    pub is_error: bool,
}

impl SkillOutcome {
    pub(crate) fn ok(text: impl Into<String>) -> Self {
        Self { text: truncate(text.into()), is_error: false }
    }
    pub(crate) fn err(text: impl Into<String>) -> Self {
        Self { text: truncate(text.into()), is_error: true }
    }
}

/// Définitions d'outils neutres ([`ToolSpec`]) pour les Skills *exécutables* présents dans
/// `enabled`. Les Skills inconnus/non-Dev sont ignorés : le LLM ne voit que ce qu'il peut
/// réellement actionner. Chaque provider (Claude/Gemini) rend ces specs dans son format.
pub fn dev_tool_definitions(enabled: &[String]) -> Vec<ToolSpec> {
    enabled
        .iter()
        .filter_map(|id| tool_definition(id))
        .collect()
}

fn tool_definition(id: &str) -> Option<ToolSpec> {
    let spec = |name: &str, description: &str, parameters: Value| ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    };
    match id {
        READ_FILE => Some(spec(
            READ_FILE,
            "Lit un fichier texte du workspace et renvoie son contenu. Chemin relatif au workspace.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Chemin relatif du fichier" } },
                "required": ["path"]
            }),
        )),
        WRITE_FILE => Some(spec(
            WRITE_FILE,
            "Écrit (ou remplace) un fichier texte dans le workspace. Crée les dossiers parents au besoin.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Chemin relatif du fichier" },
                    "content": { "type": "string", "description": "Contenu à écrire" }
                },
                "required": ["path", "content"]
            }),
        )),
        EXEC_COMMAND => Some(spec(
            EXEC_COMMAND,
            "Exécute une commande shell dans le workspace et renvoie stdout/stderr et le code de sortie.",
            json!({
                "type": "object",
                "properties": { "command": { "type": "string", "description": "Commande shell à exécuter" } },
                "required": ["command"]
            }),
        )),
        _ => None,
    }
}

/// Exécute un Skill par son nom d'outil avec l'`input` JSON fourni par le modèle.
pub async fn execute_skill(name: &str, input: &Value, workspace: &Path) -> SkillOutcome {
    match name {
        READ_FILE => read_file(input, workspace),
        WRITE_FILE => write_file(input, workspace),
        EXEC_COMMAND => exec_command(input, workspace).await,
        other => SkillOutcome::err(format!("Skill « {other} » non exécutable à ce stade (Phase 4a).")),
    }
}

fn read_file(input: &Value, workspace: &Path) -> SkillOutcome {
    let Some(rel) = input.get("path").and_then(Value::as_str) else {
        return SkillOutcome::err("paramètre `path` manquant.");
    };
    let path = match safe_join(workspace, rel) {
        Ok(p) => p,
        Err(e) => return SkillOutcome::err(e),
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => SkillOutcome::ok(content),
        Err(e) => SkillOutcome::err(format!("lecture impossible ({rel}) : {e}")),
    }
}

fn write_file(input: &Value, workspace: &Path) -> SkillOutcome {
    let Some(rel) = input.get("path").and_then(Value::as_str) else {
        return SkillOutcome::err("paramètre `path` manquant.");
    };
    let Some(content) = input.get("content").and_then(Value::as_str) else {
        return SkillOutcome::err("paramètre `content` manquant.");
    };
    let path = match safe_join(workspace, rel) {
        Ok(p) => p,
        Err(e) => return SkillOutcome::err(e),
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return SkillOutcome::err(format!("création du dossier impossible : {e}"));
        }
    }
    match std::fs::write(&path, content) {
        Ok(()) => SkillOutcome::ok(format!("écrit {} octets dans {rel}", content.len())),
        Err(e) => SkillOutcome::err(format!("écriture impossible ({rel}) : {e}")),
    }
}

async fn exec_command(input: &Value, workspace: &Path) -> SkillOutcome {
    let Some(command) = input.get("command").and_then(Value::as_str) else {
        return SkillOutcome::err("paramètre `command` manquant.");
    };

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command).current_dir(workspace);

    let run = async {
        let out = cmd.output().await?;
        Ok::<_, std::io::Error>(out)
    };

    match timeout(COMMAND_TIMEOUT, run).await {
        Err(_) => SkillOutcome::err(format!("commande interrompue après {}s.", COMMAND_TIMEOUT.as_secs())),
        Ok(Err(e)) => SkillOutcome::err(format!("exécution impossible : {e}")),
        Ok(Ok(out)) => {
            let code = out.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let text = format!("exit={code}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}");
            if out.status.success() {
                SkillOutcome::ok(text)
            } else {
                SkillOutcome::err(text)
            }
        }
    }
}

/// Joint un chemin relatif au workspace en refusant toute évasion (absolu ou `..`).
fn safe_join(workspace: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(format!("chemin absolu refusé : {rel}"));
    }
    for comp in rel_path.components() {
        match comp {
            Component::ParentDir => return Err(format!("composant `..` refusé : {rel}")),
            Component::Prefix(_) | Component::RootDir => {
                return Err(format!("chemin hors workspace refusé : {rel}"))
            }
            _ => {}
        }
    }
    Ok(workspace.join(rel_path))
}

fn truncate(mut s: String) -> String {
    if s.len() > MAX_OUTPUT {
        s.truncate(MAX_OUTPUT);
        s.push_str("\n…(sortie tronquée)");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_definitions_only_for_executable_skills() {
        let enabled = vec![
            READ_FILE.to_string(),
            "Web_Search".to_string(), // non-Dev → ignoré
            EXEC_COMMAND.to_string(),
        ];
        let defs = dev_tool_definitions(&enabled);
        let names: Vec<_> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec![READ_FILE, EXEC_COMMAND]);
    }

    #[test]
    fn safe_join_rejects_escapes() {
        let ws = Path::new("/tmp/ws");
        assert!(safe_join(ws, "../secret").is_err());
        assert!(safe_join(ws, "/etc/passwd").is_err());
        assert_eq!(safe_join(ws, "src/main.rs").unwrap(), ws.join("src/main.rs"));
    }

    #[test]
    fn read_then_write_round_trips_within_workspace() {
        let ws = std::env::temp_dir().join(format!("orch-skill-{}", std::process::id()));
        std::fs::create_dir_all(&ws).unwrap();

        let w = write_file(&json!({"path": "notes/a.txt", "content": "bonjour"}), &ws);
        assert!(!w.is_error, "{}", w.text);

        let r = read_file(&json!({"path": "notes/a.txt"}), &ws);
        assert!(!r.is_error);
        assert_eq!(r.text, "bonjour");

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn exec_command_reports_exit_and_output() {
        let ws = std::env::temp_dir();
        let out = exec_command(&json!({"command": "echo orchestra"}), &ws).await;
        assert!(!out.is_error, "{}", out.text);
        assert!(out.text.contains("orchestra"));
        assert!(out.text.contains("exit=0"));
    }
}

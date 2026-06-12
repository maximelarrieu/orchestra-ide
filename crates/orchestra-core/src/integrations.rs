//! Intégrations écosystème Dev (Phase 4b) — Git (local) et GitHub (REST).
//!
//! Comme les Skills Dev, ces capacités sont exposées au LLM sous forme de *tools* — mais
//! uniquement lorsque l'intégration correspondante est **configurée** dans l'espace
//! (`config.integrations`). Git est local (binaire `git` dans le workspace) ; GitHub passe
//! par l'API REST avec un token lu depuis la variable d'environnement déclarée
//! (`token_env_var`) — jamais codé en dur, et n'est exposé que si ce token est présent.
//!
//! ⚠️ Certaines actions modifient l'état (commit, création de branche) ou sont
//! *sortantes* (création de PR, commentaire). L'utilisateur les autorise en configurant
//! l'intégration dans son espace.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::timeout;

use crate::llm::ToolSpec;
use crate::model::space::ContextSpace;
use crate::skills::SkillOutcome;

// Noms d'outils Git (locaux).
pub const GIT_STATUS: &str = "Git_Status";
pub const GIT_DIFF: &str = "Git_Diff";
pub const GIT_CREATE_BRANCH: &str = "Git_Create_Branch";
pub const GIT_COMMIT: &str = "Git_Commit";

// Noms d'outils GitHub (REST).
pub const GH_LIST_ISSUES: &str = "GitHub_List_Issues";
pub const GH_COMMENT: &str = "GitHub_Create_Issue_Comment";
pub const GH_CREATE_PR: &str = "GitHub_Create_Pull_Request";

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const GITHUB_API: &str = "https://api.github.com";

/// Connexion GitHub résolue : dépôt `owner/repo` + token (depuis l'environnement).
#[derive(Clone)]
pub struct GithubConn {
    pub repo: String,
    pub token: String,
}

/// Contexte d'intégrations prêt à l'emploi pour un espace.
#[derive(Clone)]
pub struct IntegrationConn {
    pub git_enabled: bool,
    pub github: Option<GithubConn>,
    http: reqwest::Client,
}

impl IntegrationConn {
    pub fn from_space(space: &ContextSpace) -> Self {
        let cfg = &space.config.integrations;
        let git_enabled = cfg.git.is_some();
        let github = cfg.github.as_ref().and_then(|g| {
            std::env::var(&g.token_env_var)
                .ok()
                .filter(|t| !t.trim().is_empty())
                .map(|token| GithubConn { repo: g.repo.clone(), token })
        });
        let http = reqwest::Client::builder()
            .user_agent("orchestra-ide")
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self { git_enabled, github, http }
    }
}

/// Vrai si `name` est un outil d'intégration (Git/GitHub) géré par ce module.
pub fn handles(name: &str) -> bool {
    matches!(
        name,
        GIT_STATUS | GIT_DIFF | GIT_CREATE_BRANCH | GIT_COMMIT | GH_LIST_ISSUES | GH_COMMENT | GH_CREATE_PR
    )
}

/// Outils exposés au modèle selon les intégrations effectivement disponibles.
pub fn tool_definitions(conn: &IntegrationConn) -> Vec<ToolSpec> {
    let mut tools = Vec::new();
    let spec = |name: &str, description: &str, parameters: Value| ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    };

    if conn.git_enabled {
        tools.push(spec(
            GIT_STATUS,
            "Affiche l'état Git du workspace (branche + fichiers modifiés).",
            json!({ "type": "object", "properties": {}, "required": [] }),
        ));
        tools.push(spec(
            GIT_DIFF,
            "Affiche le diff Git non indexé. Optionnellement limité à un chemin.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Chemin à differ (optionnel)" } },
                "required": []
            }),
        ));
        tools.push(spec(
            GIT_CREATE_BRANCH,
            "Crée et bascule sur une nouvelle branche Git.",
            json!({
                "type": "object",
                "properties": { "name": { "type": "string", "description": "Nom de la branche" } },
                "required": ["name"]
            }),
        ));
        tools.push(spec(
            GIT_COMMIT,
            "Indexe tous les changements (git add -A) puis crée un commit avec le message donné.",
            json!({
                "type": "object",
                "properties": { "message": { "type": "string", "description": "Message de commit" } },
                "required": ["message"]
            }),
        ));
    }

    if conn.github.is_some() {
        tools.push(spec(
            GH_LIST_ISSUES,
            "Liste les issues ouvertes du dépôt GitHub configuré.",
            json!({
                "type": "object",
                "properties": { "state": { "type": "string", "description": "open | closed | all (défaut open)" } },
                "required": []
            }),
        ));
        tools.push(spec(
            GH_COMMENT,
            "Ajoute un commentaire sur une issue (ou PR) du dépôt GitHub configuré.",
            json!({
                "type": "object",
                "properties": {
                    "issue_number": { "type": "integer", "description": "Numéro de l'issue/PR" },
                    "body": { "type": "string", "description": "Contenu du commentaire" }
                },
                "required": ["issue_number", "body"]
            }),
        ));
        tools.push(spec(
            GH_CREATE_PR,
            "Crée une pull request sur le dépôt GitHub configuré.",
            json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "head": { "type": "string", "description": "Branche source" },
                    "base": { "type": "string", "description": "Branche cible" },
                    "body": { "type": "string", "description": "Description (optionnel)" }
                },
                "required": ["title", "head", "base"]
            }),
        ));
    }

    tools
}

/// Exécute un outil d'intégration. Suppose `handles(name) == true`.
pub async fn execute(name: &str, input: &Value, workspace: &Path, conn: &IntegrationConn) -> SkillOutcome {
    match name {
        GIT_STATUS => git(&["status", "--short", "--branch"], workspace).await,
        GIT_DIFF => match input.get("path").and_then(Value::as_str) {
            Some(p) => git(&["diff", "--", p], workspace).await,
            None => git(&["diff"], workspace).await,
        },
        GIT_CREATE_BRANCH => match input.get("name").and_then(Value::as_str) {
            Some(n) if valid_branch(n) => git(&["checkout", "-b", n], workspace).await,
            Some(_) => SkillOutcome::err("nom de branche invalide."),
            None => SkillOutcome::err("paramètre `name` manquant."),
        },
        GIT_COMMIT => match input.get("message").and_then(Value::as_str) {
            Some(msg) => {
                let add = git(&["add", "-A"], workspace).await;
                if add.is_error {
                    return add;
                }
                git(&["commit", "-m", msg], workspace).await
            }
            None => SkillOutcome::err("paramètre `message` manquant."),
        },
        GH_LIST_ISSUES | GH_COMMENT | GH_CREATE_PR => match &conn.github {
            Some(gh) => github_execute(name, input, gh, &conn.http).await,
            None => SkillOutcome::err("intégration GitHub non configurée."),
        },
        other => SkillOutcome::err(format!("outil d'intégration inconnu : {other}")),
    }
}

async fn git(args: &[&str], ws: &Path) -> SkillOutcome {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(ws);
    match timeout(GIT_TIMEOUT, cmd.output()).await {
        Err(_) => SkillOutcome::err("commande git interrompue (délai dépassé)."),
        Ok(Err(e)) => SkillOutcome::err(format!("git introuvable ou non exécutable : {e}")),
        Ok(Ok(out)) => {
            let code = out.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let text = format!("git {} → exit={code}\n{stdout}{stderr}", args.join(" "));
            if out.status.success() {
                SkillOutcome::ok(text)
            } else {
                SkillOutcome::err(text)
            }
        }
    }
}

fn valid_branch(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
}

async fn github_execute(name: &str, input: &Value, gh: &GithubConn, http: &reqwest::Client) -> SkillOutcome {
    match name {
        GH_LIST_ISSUES => {
            let state = input.get("state").and_then(Value::as_str).unwrap_or("open");
            let url = format!("{GITHUB_API}/repos/{}/issues?state={state}&per_page=30", gh.repo);
            match github_send(http, gh, reqwest::Method::GET, &url, None).await {
                Ok(v) => SkillOutcome::ok(format_issues(&v)),
                Err(e) => SkillOutcome::err(e),
            }
        }
        GH_COMMENT => {
            let Some(num) = input.get("issue_number").and_then(Value::as_i64) else {
                return SkillOutcome::err("paramètre `issue_number` manquant.");
            };
            let Some(body) = input.get("body").and_then(Value::as_str) else {
                return SkillOutcome::err("paramètre `body` manquant.");
            };
            let url = format!("{GITHUB_API}/repos/{}/issues/{num}/comments", gh.repo);
            match github_send(http, gh, reqwest::Method::POST, &url, Some(json!({ "body": body }))).await {
                Ok(v) => SkillOutcome::ok(format!(
                    "commentaire ajouté : {}",
                    v.get("html_url").and_then(Value::as_str).unwrap_or("(ok)")
                )),
                Err(e) => SkillOutcome::err(e),
            }
        }
        GH_CREATE_PR => {
            let title = input.get("title").and_then(Value::as_str);
            let head = input.get("head").and_then(Value::as_str);
            let base = input.get("base").and_then(Value::as_str);
            let (Some(title), Some(head), Some(base)) = (title, head, base) else {
                return SkillOutcome::err("paramètres `title`, `head` et `base` requis.");
            };
            let body = input.get("body").and_then(Value::as_str).unwrap_or("");
            let url = format!("{GITHUB_API}/repos/{}/pulls", gh.repo);
            let payload = json!({ "title": title, "head": head, "base": base, "body": body });
            match github_send(http, gh, reqwest::Method::POST, &url, Some(payload)).await {
                Ok(v) => SkillOutcome::ok(format!(
                    "PR créée : {}",
                    v.get("html_url").and_then(Value::as_str).unwrap_or("(ok)")
                )),
                Err(e) => SkillOutcome::err(e),
            }
        }
        _ => SkillOutcome::err("outil GitHub inconnu."),
    }
}

/// Envoie une requête GitHub authentifiée et renvoie le corps JSON, ou un message d'erreur.
async fn github_send(
    http: &reqwest::Client,
    gh: &GithubConn,
    method: reqwest::Method,
    url: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut req = http
        .request(method, url)
        .bearer_auth(&gh.token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.map_err(|e| format!("erreur réseau GitHub : {e}"))?;
    let status = resp.status();
    let value: Value = resp.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        Ok(value)
    } else {
        let msg = value.get("message").and_then(Value::as_str).unwrap_or("erreur");
        Err(format!("GitHub {status} : {msg}"))
    }
}

fn format_issues(v: &Value) -> String {
    let Some(items) = v.as_array() else {
        return "réponse GitHub inattendue.".to_string();
    };
    let lines: Vec<String> = items
        .iter()
        .filter(|i| i.get("pull_request").is_none()) // exclut les PR (l'API les mêle aux issues)
        .map(|i| {
            let num = i.get("number").and_then(Value::as_i64).unwrap_or(0);
            let title = i.get("title").and_then(Value::as_str).unwrap_or("");
            format!("#{num} {title}")
        })
        .collect();
    if lines.is_empty() {
        "aucune issue.".to_string()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::{GitIntegration, GithubIntegration, Integrations, ProjectConfig};
    use crate::model::project_type::ProjectType;
    use std::path::PathBuf;

    fn space(integrations: Integrations) -> ContextSpace {
        ContextSpace {
            root: PathBuf::from("."),
            config: ProjectConfig {
                project_name: "T".into(),
                project_type: ProjectType::Dev,
                workspace_path: None,
                documentalist_enabled: false,
                skills: vec![],
                agents: vec![],
                integrations,
            },
            persona: None,
            adrs: vec![],
        }
    }

    #[test]
    fn git_tools_only_when_git_configured() {
        let conn = IntegrationConn::from_space(&space(Integrations {
            git: Some(GitIntegration { auto_branching: false, main_branch: "main".into() }),
            github: None,
            jira: None,
        }));
        let names: Vec<_> = tool_definitions(&conn).iter().map(|t| t.name.clone()).collect();
        assert!(names.contains(&GIT_STATUS.to_string()));
        assert!(!names.iter().any(|n| n.starts_with("GitHub_")));
    }

    #[test]
    fn github_tools_hidden_without_token() {
        // github configuré mais variable d'env absente → pas d'outils GitHub exposés.
        std::env::remove_var("ORCH_TEST_GH_TOKEN");
        let conn = IntegrationConn::from_space(&space(Integrations {
            git: None,
            github: Some(GithubIntegration {
                repo: "owner/repo".into(),
                token_env_var: "ORCH_TEST_GH_TOKEN".into(),
            }),
            jira: None,
        }));
        assert!(conn.github.is_none());
        assert!(tool_definitions(&conn).is_empty());
    }

    #[test]
    fn branch_name_validation() {
        assert!(valid_branch("feature/x-1"));
        assert!(!valid_branch("-bad"));
        assert!(!valid_branch("a b"));
        assert!(!valid_branch(""));
    }

    #[tokio::test]
    async fn git_status_and_branch_in_temp_repo() {
        let ws = std::env::temp_dir().join(format!("orch-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();

        // Init + config minimale (sinon commit échoue sur identité absente).
        for args in [
            ["init", "-q"].as_slice(),
            ["config", "user.email", "t@t.io"].as_slice(),
            ["config", "user.name", "t"].as_slice(),
        ] {
            assert!(!git(args, &ws).await.is_error);
        }

        let status = execute(GIT_STATUS, &json!({}), &ws, &dummy_conn()).await;
        assert!(!status.is_error, "{}", status.text);

        let branch = execute(GIT_CREATE_BRANCH, &json!({ "name": "wip/test" }), &ws, &dummy_conn()).await;
        assert!(!branch.is_error, "{}", branch.text);

        let _ = std::fs::remove_dir_all(&ws);
    }

    fn dummy_conn() -> IntegrationConn {
        IntegrationConn { git_enabled: true, github: None, http: reqwest::Client::new() }
    }
}

//! État Git **structuré**, pour affichage direct par les UIs (panneau Git) — indépendant
//! du chemin LLM (`integrations.rs` expose Git comme *tool* texte pour l'agent ; ce module
//! est la source de vérité partagée par les deux : l'UI l'appelle directement, le module
//! `integrations` s'appuie sur [`run_command`] pour ses propres outils).
//!
//! Logique pure et partagée TUI ⇄ GUI : aucune dépendance d'affichage.

use std::path::Path;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// État d'un fichier dans l'index et/ou l'arbre de travail (un caractère de statut Git par
/// zone : `M`odifié, `A`jouté, `D`supprimé, `R`enommé, `.` = inchangé dans cette zone…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileStatus {
    pub path: String,
    pub index: char,
    pub worktree: char,
}

/// État Git du workspace, pour affichage direct (panneau Git des deux UIs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// Faux si le dossier n'est pas un dépôt Git (ou si `git` est indisponible) — dans ce
    /// cas tous les autres champs restent à leur valeur par défaut, jamais d'erreur bloquante.
    pub is_repo: bool,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// Fichiers avec des changements indexés (prêts à être commit).
    pub staged: Vec<GitFileStatus>,
    /// Fichiers avec des changements non indexés dans l'arbre de travail.
    pub unstaged: Vec<GitFileStatus>,
    /// Fichiers non suivis par Git.
    pub untracked: Vec<String>,
}

/// Sortie brute d'une commande `git` (utilisée par [`crate::integrations`] pour ses outils LLM).
pub(crate) struct GitOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub code: i32,
}

/// Exécute `git <args>` dans `root`. Erreur uniquement si la commande n'a pas pu être
/// lancée du tout (binaire absent, timeout) — un exit code non nul reste un `Ok` avec
/// `success: false`, pour laisser l'appelant décider quoi en faire.
pub(crate) async fn run_command(args: &[&str], root: &Path) -> Result<GitOutput, String> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(root);
    match timeout(GIT_TIMEOUT, cmd.output()).await {
        Err(_) => Err("commande git interrompue (délai dépassé).".to_string()),
        Ok(Err(e)) => Err(format!("git introuvable ou non exécutable : {e}")),
        Ok(Ok(out)) => Ok(GitOutput {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            success: out.status.success(),
            code: out.status.code().unwrap_or(-1),
        }),
    }
}

/// État Git structuré de `root` (branche, ahead/behind, fichiers modifiés). Ne remonte
/// jamais d'erreur côté appelant : si `root` n'est pas un dépôt Git (ou `git` absent),
/// renvoie `GitStatus::default()` (`is_repo: false`) — l'UI affiche alors un état neutre.
pub async fn status(root: &Path) -> GitStatus {
    match run_command(&["status", "--porcelain=v2", "--branch"], root).await {
        Ok(out) if out.success => parse_porcelain_v2(&out.stdout),
        _ => GitStatus::default(),
    }
}

/// Diff `git diff` (non indexé), optionnellement limité à un fichier. Chaîne vide si le
/// dossier n'est pas un dépôt Git, si `git` est indisponible, ou en l'absence de changement.
pub async fn diff(root: &Path, path: Option<&str>) -> String {
    let args: Vec<&str> = match path {
        Some(p) => vec!["diff", "--", p],
        None => vec!["diff"],
    };
    match run_command(&args, root).await {
        Ok(out) if out.success => out.stdout,
        _ => String::new(),
    }
}

fn parse_porcelain_v2(output: &str) -> GitStatus {
    let mut status = GitStatus { is_repo: true, ..GitStatus::default() };
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            if rest != "(detached)" {
                status.branch = Some(rest.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            status.upstream = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            for tok in rest.split_whitespace() {
                if let Some(n) = tok.strip_prefix('+') {
                    status.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = tok.strip_prefix('-') {
                    status.behind = n.parse().unwrap_or(0);
                }
            }
        } else if let Some(rest) = line.strip_prefix("1 ") {
            push_ordinary(&mut status, rest);
        } else if let Some(rest) = line.strip_prefix("2 ") {
            push_renamed(&mut status, rest);
        } else if let Some(rest) = line.strip_prefix("u ") {
            push_unmerged(&mut status, rest);
        } else if let Some(path) = line.strip_prefix("? ") {
            status.untracked.push(path.to_string());
        }
        // Lignes "!" (ignorées) : non demandées (pas de --ignored), donc jamais présentes.
    }
    status
}

/// `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`
fn push_ordinary(status: &mut GitStatus, rest: &str) {
    let mut parts = rest.splitn(8, ' ');
    let Some(xy) = parts.next() else { return };
    for _ in 0..6 {
        parts.next();
    }
    let Some(path) = parts.next() else { return };
    add_file(status, xy, path.to_string());
}

/// `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path><TAB><origPath>`
fn push_renamed(status: &mut GitStatus, rest: &str) {
    let mut parts = rest.splitn(9, ' ');
    let Some(xy) = parts.next() else { return };
    for _ in 0..7 {
        parts.next();
    }
    let Some(paths) = parts.next() else { return };
    let path = paths.split('\t').next().unwrap_or(paths);
    add_file(status, xy, path.to_string());
}

/// `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>` (conflit de fusion).
fn push_unmerged(status: &mut GitStatus, rest: &str) {
    let mut parts = rest.splitn(10, ' ');
    let Some(xy) = parts.next() else { return };
    for _ in 0..8 {
        parts.next();
    }
    let Some(path) = parts.next() else { return };
    add_file(status, xy, path.to_string());
}

fn add_file(status: &mut GitStatus, xy: &str, path: String) {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    if x != '.' {
        status.staged.push(GitFileStatus { path: path.clone(), index: x, worktree: y });
    }
    if y != '.' {
        status.unstaged.push(GitFileStatus { path, index: x, worktree: y });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_header_and_ahead_behind() {
        let out = "# branch.oid abc123\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n";
        let s = parse_porcelain_v2(out);
        assert!(s.is_repo);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!(s.ahead, 2);
        assert_eq!(s.behind, 1);
    }

    #[test]
    fn detached_head_has_no_branch_name() {
        let out = "# branch.head (detached)\n";
        let s = parse_porcelain_v2(out);
        assert!(s.branch.is_none());
    }

    #[test]
    fn parses_ordinary_staged_and_unstaged() {
        // "MM" : modifié en index ET dans l'arbre de travail → apparaît dans les deux listes.
        let out = "1 MM N... 100644 100644 100644 aaaa bbbb src/main.rs\n\
                   1 .M N... 100644 100644 100644 aaaa bbbb README.md\n\
                   1 A. N... 000000 100644 100644 0000 cccc new_file.rs\n";
        let s = parse_porcelain_v2(out);
        assert!(s.staged.iter().any(|f| f.path == "src/main.rs" && f.index == 'M'));
        assert!(s.unstaged.iter().any(|f| f.path == "src/main.rs" && f.worktree == 'M'));
        assert!(s.unstaged.iter().any(|f| f.path == "README.md"));
        assert!(!s.staged.iter().any(|f| f.path == "README.md"));
        assert!(s.staged.iter().any(|f| f.path == "new_file.rs" && f.index == 'A'));
    }

    #[test]
    fn parses_untracked() {
        let out = "? new.txt\n? dir/other.txt\n";
        let s = parse_porcelain_v2(out);
        assert_eq!(s.untracked, vec!["new.txt".to_string(), "dir/other.txt".to_string()]);
    }

    #[test]
    fn parses_renamed_entry_using_new_path() {
        let out = "2 R. N... 100644 100644 100644 aaaa bbbb R100 new_name.rs\told_name.rs\n";
        let s = parse_porcelain_v2(out);
        assert!(s.staged.iter().any(|f| f.path == "new_name.rs" && f.index == 'R'));
    }

    #[tokio::test]
    async fn status_of_non_repo_dir_is_neutral() {
        let dir = std::env::temp_dir().join(format!("orch-git-status-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let s = status(&dir).await;
        assert!(!s.is_repo);
        assert!(s.staged.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn status_of_real_temp_repo_reports_branch_and_changes() {
        let dir = std::env::temp_dir().join(format!("orch-git-status-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        for args in [
            ["init", "-q"].as_slice(),
            ["config", "user.email", "t@t.io"].as_slice(),
            ["config", "user.name", "t"].as_slice(),
            // Nom de branche explicite (indépendant de `init.defaultBranch` sur la machine) :
            // fonctionne même sans commit, `checkout -b` sur un dépôt vide ne fait que
            // pointer HEAD vers la nouvelle branche.
            ["checkout", "-q", "-b", "main"].as_slice(),
        ] {
            assert!(run_command(args, &dir).await.unwrap().success);
        }
        std::fs::write(dir.join("README.md"), "hello").unwrap();

        let s = status(&dir).await;
        assert!(s.is_repo);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert!(s.untracked.contains(&"README.md".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

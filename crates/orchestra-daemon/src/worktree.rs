//! Git worktrees: the box an agent works in.
//!
//! One worktree per ticket, on its own branch, outside the project directory.
//! That is the whole safety model: an agent cannot damage the working copy it
//! was not given, and a run that goes wrong is thrown away by deleting a
//! directory and leaving a branch behind.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use orchestra_core::model::{ticket_slug, Project, Ticket};

/// A worktree created for a ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
}

/// Where a ticket's worktree goes and what its branch is called.
pub fn plan_for(
    project: &Project,
    ticket: &Ticket,
    worktrees_dir: &Path,
    branch_prefix: &str,
) -> Worktree {
    let slug = ticket_slug(ticket.number, &ticket.title);
    let project_slug = project
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project.name.clone());
    Worktree {
        path: worktrees_dir.join(project_slug).join(&slug),
        branch: format!("{branch_prefix}{slug}"),
    }
}

/// Create the worktree, or adopt the one already there.
///
/// Re-running a launch must not fail: a ticket relaunched after a crash finds
/// its worktree and its branch exactly as the previous run left them.
pub fn ensure(project: &Project, plan: &Worktree) -> Result<Worktree> {
    if plan.path.join(".git").exists() {
        return Ok(plan.clone());
    }
    if plan.path.exists() {
        let empty = std::fs::read_dir(&plan.path)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !empty {
            bail!(
                "{} existe déjà et n'est pas un worktree git",
                plan.path.display()
            );
        }
        let _ = std::fs::remove_dir(&plan.path);
    }
    if let Some(parent) = plan.path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("création de {}", parent.display()))?;
    }

    // `git worktree add -b` fails when the branch exists, which is exactly what
    // a relaunch looks like, so an existing branch is checked out instead.
    let exists = branch_exists(&project.path, &plan.branch);
    let mut args: Vec<String> = vec!["worktree".into(), "add".into()];
    if !exists {
        args.push("-b".into());
        args.push(plan.branch.clone());
    }
    args.push(plan.path.to_string_lossy().to_string());
    if exists {
        args.push(plan.branch.clone());
    } else {
        args.push(project.default_branch.clone());
    }

    git(&project.path, &args).with_context(|| {
        format!(
            "création du worktree {} sur {}",
            plan.path.display(),
            plan.branch
        )
    })?;
    Ok(plan.clone())
}

/// Remove the worktree directory. The branch is always kept: it holds the work.
pub fn remove(project: &Project, worktree: &Path) -> Result<()> {
    git(
        &project.path,
        &[
            "worktree".into(),
            "remove".into(),
            "--force".into(),
            worktree.to_string_lossy().to_string(),
        ],
    )
    .with_context(|| format!("suppression du worktree {}", worktree.display()))?;
    Ok(())
}

/// Forget worktrees whose directory disappeared, so git stops listing them.
pub fn prune(project: &Project) -> Result<()> {
    git(&project.path, &["worktree".into(), "prune".into()])?;
    Ok(())
}

/// Commits on `branch` that are not on the project's default branch.
pub fn commits_ahead(project: &Project, branch: &str) -> Vec<String> {
    let range = format!("{}..{branch}", project.default_branch);
    git(
        &project.path,
        &[
            "log".into(),
            "--oneline".into(),
            "--no-decorate".into(),
            range,
        ],
    )
    .map(|out| out.lines().map(str::to_string).collect())
    .unwrap_or_default()
}

/// Bring the local default branch up to what the remote now holds.
///
/// Called after a pull request was merged over there: without it, this machine
/// keeps an older default branch and the next ticket branches from the past.
/// Fast-forward only and on the default branch, like every move we make here.
pub fn pull_default_branch(project: &Project) -> Result<()> {
    let Some(remote) = default_remote(&project.path) else {
        return Ok(());
    };
    git(&project.path, &["fetch".into(), remote.clone()])
        .with_context(|| format!("fetch de {remote}"))?;
    let head = current_branch(&project.path);
    if head.as_deref() != Some(project.default_branch.as_str()) {
        bail!(
            "le dépôt est sur « {} », pas sur « {} »",
            head.unwrap_or_else(|| "un commit détaché".into()),
            project.default_branch
        );
    }
    let remote_ref = format!("{remote}/{}", project.default_branch);
    git(
        &project.path,
        &["merge".into(), "--ff-only".into(), remote_ref.clone()],
    )
    .with_context(|| format!("avance rapide sur {remote_ref}"))?;
    Ok(())
}

/// The remote a push would go to: `origin` when it exists, else the first one
/// declared, and `None` for a repository that has none.
pub fn default_remote(repo: &Path) -> Option<String> {
    let out = git(repo, &["remote".into()]).ok()?;
    let mut names = out.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = names.next()?.to_string();
    if out.lines().any(|l| l.trim() == "origin") {
        return Some("origin".into());
    }
    Some(first)
}

/// Push a branch to the project's remote.
///
/// Returns the remote it went to. A repository without a remote is not an
/// error: there is simply nowhere to push, and the merge already happened.
pub fn push_branch(repo: &Path, branch: &str) -> Result<Option<String>> {
    let Some(remote) = default_remote(repo) else {
        return Ok(None);
    };
    git(repo, &["push".into(), remote.clone(), branch.to_string()])
        .with_context(|| format!("push de {branch} vers {remote}"))?;
    Ok(Some(remote))
}

/// Tracked files changed but not committed. Untracked files are left out on
/// purpose: they are the user's own business, not a reason to refuse a merge.
pub fn tracked_changes(repo: &Path) -> Vec<String> {
    git(
        repo,
        &[
            "status".into(),
            "--porcelain".into(),
            "--untracked-files=no".into(),
        ],
    )
    .map(|out| out.lines().map(str::to_string).collect())
    .unwrap_or_default()
}

/// Files changed in the worktree but not committed, so a half-finished run is
/// visible rather than silent.
pub fn dirty_files(worktree: &Path) -> Vec<String> {
    git(worktree, &["status".into(), "--porcelain".into()])
        .map(|out| out.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// The branch checked out in a repository, when it is on one.
pub fn current_branch(repo: &Path) -> Option<String> {
    git(
        repo,
        &["rev-parse".into(), "--abbrev-ref".into(), "HEAD".into()],
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty() && s != "HEAD")
}

/// Bring a ticket's branch into the project's default branch, in fast-forward.
///
/// Fast-forward only, and refused outright unless the main repository is on its
/// default branch with a clean working tree. A merge commit made here would be
/// a merge nobody watched, and a dirty tree means the user is in the middle of
/// something: in both cases the answer is to say so, not to be clever. The
/// integrator has already brought the default branch into the ticket's branch,
/// so a fast-forward is exactly what should be possible.
pub fn merge_fast_forward(project: &Project, branch: &str) -> Result<usize> {
    let head = current_branch(&project.path);
    if head.as_deref() != Some(project.default_branch.as_str()) {
        bail!(
            "le dépôt principal est sur « {} », pas sur « {} » : place-le sur sa branche \
             par défaut avant d'intégrer",
            head.unwrap_or_else(|| "un commit détaché".into()),
            project.default_branch
        );
    }
    // Only tracked changes block: an untracked file cannot conflict with a
    // fast-forward, and refusing on one stopped a real merge because a README
    // draft was sitting in the repository. If the merge really would overwrite
    // an untracked file, git says so itself and that message is passed on.
    let dirty = tracked_changes(&project.path);
    if !dirty.is_empty() {
        bail!(
            "le dépôt principal a {} fichier(s) modifié(s) non commité(s) ({}) : \
             la fusion attendra que tu aies rangé",
            dirty.len(),
            dirty
                .iter()
                .map(|l| l.split_whitespace().last().unwrap_or(l))
                .take(3)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let commits = commits_ahead(project, branch).len();
    git(
        &project.path,
        &["merge".into(), "--ff-only".into(), branch.to_string()],
    )
    .with_context(|| {
        format!(
            "fusion en avance rapide de {branch} dans {}",
            project.default_branch
        )
    })?;
    Ok(commits)
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    git(
        repo,
        &[
            "rev-parse".into(),
            "--verify".into(),
            "--quiet".into(),
            format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
}

/// True when the path is a git repository we can create worktrees from.
pub fn is_repository(path: &Path) -> bool {
    git(path, &["rev-parse".into(), "--git-dir".into()]).is_ok()
}

fn git(cwd: &Path, args: &[String]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .with_context(|| format!("exécution de git {}", args.join(" ")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        bail!(if err.is_empty() {
            format!("git {} a échoué", args.join(" "))
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::{ProjectKind, TicketStatus};
    use uuid::Uuid;

    struct Fixture {
        _dir: tempfile::TempDir,
        project: Project,
        worktrees: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("depot");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(&args)
                .output()
                .unwrap();
        }
        std::fs::write(repo.join("fichier.txt"), "départ").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "-A"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["commit", "-q", "-m", "départ"])
            .output()
            .unwrap();

        let project = Project {
            id: Uuid::new_v4(),
            name: "depot".into(),
            path: repo,
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        };
        let worktrees = dir.path().join("worktrees");
        Fixture {
            _dir: dir,
            project,
            worktrees,
        }
    }

    fn ticket(number: i64, title: &str) -> Ticket {
        Ticket {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            number,
            title: title.into(),
            brief: "b".into(),
            status: TicketStatus::Planned,
            branch: None,
            worktree_path: None,
            proposal: None,
            team: None,
            created_at: orchestra_core::now(),
            updated_at: orchestra_core::now(),
        }
    }

    #[test]
    fn the_path_and_branch_come_from_the_ticket() {
        let f = fixture();
        let plan = plan_for(
            &f.project,
            &ticket(12, "Ajouter l'écran Coût !"),
            &f.worktrees,
            "orch/",
        );
        assert_eq!(plan.branch, "orch/12-ajouter-l-ecran-cout");
        assert!(plan.path.ends_with("depot/12-ajouter-l-ecran-cout"));
    }

    #[test]
    fn creating_a_worktree_gives_a_working_checkout_on_a_new_branch() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(1, "cache"), &f.worktrees, "orch/");
        let wt = ensure(&f.project, &plan).unwrap();

        assert!(wt.path.join("fichier.txt").exists(), "le contenu est là");
        assert!(is_repository(&wt.path));
        let branch = git(
            &wt.path,
            &["symbolic-ref".into(), "--short".into(), "HEAD".into()],
        )
        .unwrap();
        assert_eq!(branch, "orch/1-cache");
        // The project's own checkout is untouched.
        let main_branch = git(
            &f.project.path,
            &["symbolic-ref".into(), "--short".into(), "HEAD".into()],
        )
        .unwrap();
        assert_eq!(main_branch, "main");
    }

    /// Commit a file in a worktree, the way an agent would.
    fn commit(path: &Path, name: &str, body: &str) {
        std::fs::write(path.join(name), body).unwrap();
        for args in [vec!["add", "-A"], vec!["commit", "-q", "-m", name]] {
            Command::new("git")
                .arg("-C")
                .arg(path)
                .args(&args)
                .output()
                .unwrap();
        }
    }

    #[test]
    fn a_finished_branch_goes_into_the_default_one_in_fast_forward() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(1, "cache"), &f.worktrees, "orch/");
        let wt = ensure(&f.project, &plan).unwrap();
        commit(&wt.path, "cache.rs", "le cache");

        let commits = merge_fast_forward(&f.project, &wt.branch).unwrap();
        assert_eq!(commits, 1);
        assert!(
            f.project.path.join("cache.rs").exists(),
            "le travail est arrivé dans la branche par défaut"
        );
        assert_eq!(current_branch(&f.project.path).as_deref(), Some("main"));
    }

    #[test]
    fn a_fusion_that_would_need_a_merge_commit_is_refused() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(1, "cache"), &f.worktrees, "orch/");
        let wt = ensure(&f.project, &plan).unwrap();
        commit(&wt.path, "cache.rs", "le cache");
        // Main moves on its own: la branche n'est plus en avance rapide.
        commit(&f.project.path, "autre.rs", "ailleurs");

        let err = merge_fast_forward(&f.project, &wt.branch).unwrap_err();
        assert!(
            format!("{err:#}").contains("avance rapide"),
            "la raison doit être lisible : {err:#}"
        );
    }

    #[test]
    fn a_dirty_or_misplaced_main_repository_stops_the_fusion() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(1, "cache"), &f.worktrees, "orch/");
        let wt = ensure(&f.project, &plan).unwrap();
        commit(&wt.path, "cache.rs", "le cache");

        // Un fichier jamais suivi n'empêche rien : il ne peut pas entrer en
        // conflit avec une avance rapide. C'est ce qui avait bloqué une vraie
        // fusion, pour un brouillon de README posé à la racine.
        std::fs::write(f.project.path.join("brouillon.txt"), "en cours").unwrap();
        merge_fast_forward(&f.project, &wt.branch).expect("un fichier non suivi ne bloque pas");
        assert!(f.project.path.join("cache.rs").exists());
    }

    #[test]
    fn a_repository_without_a_remote_is_not_a_push_failure() {
        // Rien à pousser n'est pas une erreur : la fusion, elle, a bien eu lieu.
        let f = fixture();
        assert!(default_remote(&f.project.path).is_none());
        assert_eq!(push_branch(&f.project.path, "main").unwrap(), None);
    }

    #[test]
    fn the_branch_goes_to_origin_when_there_is_one() {
        let f = fixture();
        // Un dépôt nu fait un remote parfaitement réel.
        let bare = f.project.path.parent().unwrap().join("origine.git");
        Command::new("git")
            .args(["init", "-q", "--bare", "-b", "main"])
            .arg(&bare)
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&f.project.path)
            .args(["remote", "add", "origin"])
            .arg(&bare)
            .output()
            .unwrap();

        assert_eq!(default_remote(&f.project.path).as_deref(), Some("origin"));
        assert_eq!(
            push_branch(&f.project.path, "main").unwrap().as_deref(),
            Some("origin")
        );
        let there = Command::new("git")
            .arg("-C")
            .arg(&bare)
            .args(["log", "--oneline", "main"])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&there.stdout).contains("départ"),
            "le commit est arrivé sur le remote"
        );
    }

    #[test]
    fn a_tracked_change_left_uncommitted_stops_the_fusion() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(1, "cache"), &f.worktrees, "orch/");
        let wt = ensure(&f.project, &plan).unwrap();
        commit(&wt.path, "cache.rs", "le cache");

        // Celui-là est suivi : le fusionner par-dessus écraserait un travail
        // en cours.
        std::fs::write(f.project.path.join("fichier.txt"), "modifié").unwrap();
        let err = merge_fast_forward(&f.project, &wt.branch).unwrap_err();
        assert!(
            format!("{err:#}").contains("fichier.txt"),
            "on nomme ce qui bloque : {err:#}"
        );
    }

    #[test]
    fn relaunching_adopts_the_existing_worktree() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(1, "cache"), &f.worktrees, "orch/");
        let wt = ensure(&f.project, &plan).unwrap();
        std::fs::write(wt.path.join("travail.txt"), "en cours").unwrap();

        // A second launch must not fail, and must not lose the work.
        let again = ensure(&f.project, &plan).unwrap();
        assert_eq!(again, wt);
        assert!(wt.path.join("travail.txt").exists());
    }

    #[test]
    fn a_worktree_removed_from_disk_is_recreated_on_its_branch() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(1, "cache"), &f.worktrees, "orch/");
        let wt = ensure(&f.project, &plan).unwrap();
        std::fs::write(wt.path.join("a.txt"), "x").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&wt.path)
            .args(["add", "-A"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&wt.path)
            .args(["commit", "-q", "-m", "[backend] travail"])
            .output()
            .unwrap();

        remove(&f.project, &wt.path).unwrap();
        assert!(!wt.path.exists());
        // The branch survives the removal: that is where the work lives.
        assert_eq!(commits_ahead(&f.project, &wt.branch).len(), 1);

        // Relaunching lands back on the same branch with its commit.
        let again = ensure(&f.project, &plan).unwrap();
        assert!(again.path.join("a.txt").exists());
    }

    #[test]
    fn commits_on_the_branch_are_listed_for_review() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(3, "revue"), &f.worktrees, "orch/");
        let wt = ensure(&f.project, &plan).unwrap();
        assert!(commits_ahead(&f.project, &wt.branch).is_empty());

        for (name, message) in [("a.txt", "[architect] plan"), ("b.txt", "[backend] code")] {
            std::fs::write(wt.path.join(name), "x").unwrap();
            Command::new("git")
                .arg("-C")
                .arg(&wt.path)
                .args(["add", "-A"])
                .output()
                .unwrap();
            Command::new("git")
                .arg("-C")
                .arg(&wt.path)
                .args(["commit", "-q", "-m", message])
                .output()
                .unwrap();
        }
        let commits = commits_ahead(&f.project, &wt.branch);
        assert_eq!(commits.len(), 2);
        assert!(commits[0].contains("[backend]"), "le plus récent d'abord");
    }

    #[test]
    fn uncommitted_work_is_visible() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(4, "sale"), &f.worktrees, "orch/");
        let wt = ensure(&f.project, &plan).unwrap();
        assert!(dirty_files(&wt.path).is_empty());
        std::fs::write(wt.path.join("oublie.txt"), "pas commité").unwrap();
        let dirty = dirty_files(&wt.path);
        assert_eq!(dirty.len(), 1);
        assert!(dirty[0].contains("oublie.txt"));
    }

    #[test]
    fn an_occupied_directory_is_refused_rather_than_overwritten() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(5, "occupe"), &f.worktrees, "orch/");
        std::fs::create_dir_all(&plan.path).unwrap();
        std::fs::write(plan.path.join("important.txt"), "ne pas perdre").unwrap();

        let err = ensure(&f.project, &plan).unwrap_err();
        assert!(err.to_string().contains("existe déjà"), "{err}");
        assert!(
            plan.path.join("important.txt").exists(),
            "rien n'est écrasé"
        );
    }

    #[test]
    fn pruning_forgets_a_directory_deleted_by_hand() {
        let f = fixture();
        let plan = plan_for(&f.project, &ticket(6, "elague"), &f.worktrees, "orch/");
        let wt = ensure(&f.project, &plan).unwrap();
        std::fs::remove_dir_all(&wt.path).unwrap();
        prune(&f.project).unwrap();
        // git no longer holds the stale registration, so a relaunch works.
        assert!(ensure(&f.project, &plan).is_ok());
    }

    #[test]
    fn a_directory_that_is_not_a_repository_is_recognised() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_repository(dir.path()));
    }
}

//! The guard that keeps an agent inside its worktree.
//!
//! Agents run with permissions bypassed, so this is the only thing standing
//! between a mistaken command and the rest of the machine. It lives in the core
//! crate because two very different programs need the exact same rules: the
//! daemon, to explain them, and `orchestra-hook`, a dependency-free binary that
//! Claude Code runs before every tool call.
//!
//! The decision is deliberately local. Asking the daemon would add a socket
//! round trip to every tool call and would make an agent depend on the daemon
//! being healthy.
//!
//! The v1 of this project had no such guard, and an agent ran `mv *.md docs/`
//! on the repository itself.

use std::path::{Component, Path, PathBuf};

/// What the guard decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Refuse, with the reason the agent will read.
    Deny(String),
}

impl Verdict {
    pub fn is_denied(&self) -> bool {
        matches!(self, Verdict::Deny(_))
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Verdict::Deny(r) => Some(r),
            Verdict::Allow => None,
        }
    }
}

/// What an agent may do with git.
///
/// Confined is what every role that writes code gets: it commits, it reads
/// history, and that is all. `Full` exists for the one role whose job *is*
/// git — fusionner, pousser, régler un conflit. It lifts the subcommand list,
/// nothing else: the path rules still hold, so even an integrator cannot reach
/// the main repository. That is what keeps the worktree rule true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GitPolicy {
    #[default]
    Confined,
    Full,
}

impl GitPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            GitPolicy::Confined => "confined",
            GitPolicy::Full => "full",
        }
    }

    /// Unknown values read as the confined policy: a typo in the environment
    /// must never widen what an agent may do.
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "full" => GitPolicy::Full,
            _ => GitPolicy::Confined,
        }
    }
}

/// Could the agent actually write at this path?
///
/// The guard reads paths, not intentions, and a shell command is full of things
/// that merely look like one: the route `/habitudes` a screenshot is taken of,
/// the regex `/_next/static/[a-z]+\.js`, the sed script `/motif/d`. Each of
/// them blocked real work. What separates them from `/home/u/projet/.env` is
/// not their shape but the filesystem: nothing can be written under `/` by an
/// ordinary user, so nothing there is an escape. The core crate does no I/O, so
/// whoever builds the boundary supplies the answer; `Boundary::new` assumes
/// everything is reachable, which is the strict reading.
pub type Reach = fn(&Path) -> bool;

fn everything_is_reachable(_: &Path) -> bool {
    true
}

/// The box an agent is allowed to touch.
#[derive(Debug, Clone)]
pub struct Boundary {
    /// The worktree; everything the agent writes must be under it.
    pub worktree: PathBuf,
    /// Extra directories it may read and write, such as a scratch space.
    pub extra: Vec<PathBuf>,
    /// What this agent may do with git.
    pub git: GitPolicy,
    /// Whether a path named in a shell command could be written at all.
    pub reach: Reach,
}

impl Boundary {
    pub fn new(worktree: impl Into<PathBuf>) -> Self {
        Boundary {
            worktree: worktree.into(),
            extra: Vec::new(),
            git: GitPolicy::Confined,
            reach: everything_is_reachable,
        }
    }

    /// The same box, told how to ask the filesystem what is writable.
    pub fn with_reach(mut self, reach: Reach) -> Self {
        self.reach = reach;
        self
    }

    /// The same box, for the role whose job is git.
    pub fn with_full_git(mut self) -> Self {
        self.git = GitPolicy::Full;
        self
    }

    fn contains(&self, path: &Path) -> bool {
        let candidate = normalise(path);
        std::iter::once(&self.worktree)
            .chain(self.extra.iter())
            .any(|root| candidate.starts_with(normalise(root)))
    }
}

/// Git subcommands an agent must never run: they move work out of the box or
/// throw it away.
const FORBIDDEN_GIT: [&str; 6] = ["push", "checkout", "switch", "worktree", "remote", "clean"];

/// Decide on one tool call.
///
/// `tool` is Claude Code's tool name and `input` its arguments as the hook
/// receives them.
pub fn check(boundary: &Boundary, tool: &str, input: &serde_json::Value) -> Verdict {
    match tool {
        "Bash" | "BashOutput" => {
            let command = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            check_command(boundary, command)
        }
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let path = input
                .get("file_path")
                .or_else(|| input.get("notebook_path"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            check_write_path(boundary, path)
        }
        // Reading outside the worktree is how an agent learns the codebase it
        // is changing; only writing is confined.
        _ => Verdict::Allow,
    }
}

/// Inspect a shell command.
pub fn check_command(boundary: &Boundary, command: &str) -> Verdict {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Verdict::Allow;
    }

    // Catastrophic patterns are looked for everywhere, heredoc bodies
    // included: `bash <<EOF … EOF` really does run what it contains.
    if let Some(reason) = catastrophic(trimmed) {
        return Verdict::Deny(reason);
    }

    // The path scan stops at a heredoc, because from there the text is data,
    // not arguments. Without this, a CSS comment or a Python snippet written
    // through `<<'PY'` looked like an escape and blocked ordinary work.
    let scanned = command_part(trimmed);
    let words = tokens(scanned);

    // Privilege escalation makes every path writable, so the filesystem's
    // answer no longer means anything and the strict reading applies.
    let escalated = words
        .iter()
        .any(|w| matches!(w.as_str(), "sudo" | "doas" | "pkexec" | "su"));

    for word in &words {
        // A path written out in full is checked whatever the command is: it is
        // the one signal that does not depend on knowing every tool's flags.
        if !looks_like_path(word) {
            continue;
        }
        let path = expand(word);
        if boundary.contains(&path) || scratch_path(&path) {
            continue;
        }
        // A path the agent could not write anyway — a web route, a regex, a
        // sed script — is a false alarm, not an escape.
        if !escalated && !(boundary.reach)(&path) {
            continue;
        }
        {
            return Verdict::Deny(format!(
                "« {} » est hors du worktree du ticket ({}). \
                 Travaille uniquement dans ton répertoire de travail.",
                word,
                boundary.worktree.display()
            ));
        }
    }

    if let Some(invocation) = git_invocation(scanned) {
        let sub = invocation[0].clone();
        // Whatever the policy, git is not a way out of the box: `-C`,
        // `--git-dir` and `--work-tree` name a directory, and a relative one is
        // not caught by the path scan above. Only git's own flags are read this
        // way — `make -C build` and `tar -C /tmp/x` mean something else.
        for dir in git_directories(scanned) {
            let path = expand(&dir);
            let path = if path.is_absolute() {
                path
            } else {
                boundary.worktree.join(path)
            };
            if !boundary.contains(&path) && !scratch_path(&path) {
                return Verdict::Deny(format!(
                    "« git -C {dir} » sort du worktree du ticket ({}). \
                     Le dépôt principal n'appartient à aucun agent.",
                    boundary.worktree.display()
                ));
            }
        }

        // `git worktree list` only says where the agent is; that is how it
        // orients itself, and it was refused along with `worktree add`.
        let read_only =
            sub == "worktree" && invocation.get(1).is_some_and(|w| w == "list");
        if boundary.git == GitPolicy::Confined
            && !read_only
            && FORBIDDEN_GIT.contains(&sub.as_str())
        {
            return Verdict::Deny(format!(
                "« git {sub} » est interdit : ta branche est relue avant toute fusion, \
                 et changer de branche ferait perdre le travail en cours."
            ));
        }
    }

    Verdict::Allow
}

/// Every directory a `git …` invocation points itself at.
fn git_directories(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let words = tokens(command);
    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        for flag in ["-C", "--git-dir", "--work-tree"] {
            if word == flag {
                if let Some(next) = words.get(i + 1) {
                    out.push(next.clone());
                }
            } else if let Some(rest) = word.strip_prefix(&format!("{flag}=")) {
                out.push(rest.to_string());
            }
        }
        i += 1;
    }
    out
}

/// The part of a command line that is still command, not heredoc content.
fn command_part(command: &str) -> &str {
    match command.find("<<") {
        // Keep the line that introduces the heredoc: its redirections are real.
        Some(pos) => {
            let head = &command[..pos];
            head.rsplit_once('\n').map(|(_, last)| last).unwrap_or(head)
        }
        None => command,
    }
}

/// Does this token name a path, rather than merely start with a slash?
///
/// `/*` in a comment or a glob is not a path; `/tmp/x.log` is. Requiring a
/// name character after the slash separates the two.
fn looks_like_path(word: &str) -> bool {
    let rest = if let Some(r) = word.strip_prefix("~/") {
        r
    } else if let Some(r) = word.strip_prefix('/') {
        r
    } else {
        return false;
    };
    rest.chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// Inspect a path a tool wants to write.
pub fn check_write_path(boundary: &Boundary, path: &str) -> Verdict {
    if path.is_empty() {
        return Verdict::Allow;
    }
    let expanded = expand(path);
    // A relative path is resolved against the worktree, which is the agent's
    // working directory.
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        boundary.worktree.join(expanded)
    };
    if boundary.contains(&candidate) || scratch_path(&candidate) {
        Verdict::Allow
    } else {
        Verdict::Deny(format!(
            "écrire dans « {path} » sort du worktree du ticket ({}).",
            boundary.worktree.display()
        ))
    }
}

/// Commands that are wrong wherever they run.
fn catastrophic(command: &str) -> Option<String> {
    let flat = command.split_whitespace().collect::<Vec<_>>().join(" ");
    let patterns = [
        ("rm -rf /", "effacer la racine du système"),
        ("rm -fr /", "effacer la racine du système"),
        ("mkfs", "formater un système de fichiers"),
        ("dd if=", "écrire directement sur un périphérique"),
        (":(){", "une bombe de forks"),
        ("shutdown", "éteindre la machine"),
        ("reboot", "redémarrer la machine"),
    ];
    for (needle, what) in patterns {
        if flat.contains(needle) {
            return Some(format!(
                "commande refusée : elle tenterait de {what}. Rien de ce que \
                 demande ce ticket ne le justifie."
            ));
        }
    }
    None
}

/// A `git …` invocation from its subcommand on, if that is what this is.
///
/// The first element is the subcommand; the rest are its arguments, as far as
/// the tokeniser can tell.
fn git_invocation(command: &str) -> Option<Vec<String>> {
    let mut words = tokens(command).into_iter().peekable();
    while let Some(word) = words.next() {
        if word != "git" {
            continue;
        }
        // Skip `-C <dir>` and other options to reach the subcommand.
        let mut rest = words.clone();
        while let Some(next) = rest.next() {
            if next == "-C" || next == "--git-dir" || next == "--work-tree" {
                rest.next();
                continue;
            }
            if next.starts_with('-') {
                continue;
            }
            return Some(std::iter::once(next).chain(rest).collect());
        }
    }
    None
}

/// Split on whitespace, dropping quotes so a quoted path is still seen.
fn tokens(command: &str) -> Vec<String> {
    command
        .split(|c: char| c.is_whitespace() || c == ';' || c == '|' || c == '&')
        .map(|w| w.trim_matches(['"', '\'', '(', ')']).to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Resolve `.` and `..` textually. The path may not exist yet, so the
/// filesystem cannot be asked.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Paths a command may mention without it meaning an escape.
///
/// System directories hold the binaries and configuration an agent reads, and
/// the temporary directory is scratch space: logs, screenshots, throwaway
/// configuration. Confining an agent to its worktree means protecting the
/// user's code, not forbidding a log file — blocking the temporary directory
/// stopped real work twice in a single run against a real project.
///
/// The user's cache directory is scratch space of the same kind: Playwright
/// keeps its browsers there and an agent was refused for installing them.
/// Nothing in a cache is the user's work, by definition.
fn scratch_path(path: &Path) -> bool {
    if let Some(home) = std::env::var_os("HOME") {
        let cache = PathBuf::from(home).join(".cache");
        if normalise(path).starts_with(normalise(&cache)) {
            return true;
        }
    }
    const ALLOWED: [&str; 10] = [
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/opt",
        "/etc",
        "/dev/null",
        "/proc",
        "/tmp",
        "/var/tmp",
    ];
    let text = path.to_string_lossy();
    ALLOWED
        .iter()
        .any(|prefix| text == *prefix || text.starts_with(&format!("{prefix}/")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A machine where the user can write under `/home` and nowhere else, as
    /// the filesystem would answer it.
    fn user_owns_home(path: &Path) -> bool {
        path.starts_with("/home")
    }

    fn boundary() -> Boundary {
        Boundary::new("/home/u/worktrees/depot/1-cache").with_reach(user_owns_home)
    }

    fn bash(command: &str) -> Verdict {
        check(&boundary(), "Bash", &json!({ "command": command }))
    }

    fn write(path: &str) -> Verdict {
        check(&boundary(), "Write", &json!({ "file_path": path }))
    }

    #[test]
    fn ordinary_work_inside_the_worktree_is_allowed() {
        for command in [
            "cargo test --workspace",
            // `-C` belongs to plenty of commands that are not git.
            "make -C build",
            "tar -C /tmp/paquet -xf archive.tar",
            "ls src",
            "grep -r TODO .",
            "git status",
            "git add -A",
            "git commit -m '[backend] ajoute le cache'",
            "git diff main",
            "npm install",
            "./scripts/build.sh",
            "cat /usr/share/doc/README",
        ] {
            assert_eq!(bash(command), Verdict::Allow, "refusé à tort : {command}");
        }
        for path in [
            "src/main.rs",
            "./docs/plan.md",
            "/home/u/worktrees/depot/1-cache/src/lib.rs",
        ] {
            assert_eq!(write(path), Verdict::Allow, "refusé à tort : {path}");
        }
    }

    #[test]
    fn writing_outside_the_worktree_is_refused_with_the_path() {
        let v = write("/home/u/projets/depot/src/main.rs");
        assert!(v.is_denied());
        assert!(v.reason().unwrap().contains("sort du worktree"));

        // The v1 incident: touching the project's own checkout.
        assert!(write("/home/u/.bashrc").is_denied());
        assert!(write("../../ailleurs.txt").is_denied());
        assert!(write("/home/u/worktrees/depot/2-autre/x.rs").is_denied());
    }

    #[test]
    fn a_heredoc_body_is_data_not_arguments() {
        // Both of these blocked real work on the first run against a real
        // project: the first is a CSS comment, the second Python source.
        assert_eq!(
            bash("cat >> style.css <<'CSS'\n/* Thème clair */\nbody { color: #222; }\nCSS"),
            Verdict::Allow
        );
        assert_eq!(
            bash("/usr/bin/python3 - 'shot.py' <<'PY'\nimport pathlib\np = pathlib.Path('/home/u/ailleurs')\nPY"),
            Verdict::Allow
        );
        // What the heredoc redirects to is still checked.
        assert!(bash("cat > /home/u/ailleurs.txt <<'EOF'\ncontenu\nEOF").is_denied());
        // And a heredoc that really runs something catastrophic is still caught.
        assert!(bash("bash <<'EOF'\nrm -rf /\nEOF").is_denied());
    }

    #[test]
    fn a_glob_is_not_a_path() {
        // `/*` looked like an absolute path and blocked a CSS comment.
        assert_eq!(bash("echo /* commentaire */"), Verdict::Allow);
        assert_eq!(bash("ls src/*.py"), Verdict::Allow);
        // The dangerous shape stays blocked by its own rule.
        assert!(bash("rm -rf /*").is_denied());
    }

    #[test]
    fn what_merely_looks_like_a_path_is_not_an_escape() {
        // Every one of these refused real work: a web route given to a
        // screenshot tool, a regex, a sed script. None of them can be written
        // by the user, so none of them leaves the box.
        for command in [
            "shot /habitudes 1024",
            "curl -s http://localhost:3000/trips | head",
            "grep -oE '/_next/static/chunks/[a-z0-9_-]+\\.js' page.html",
            "sed -i '/sp_on_accent/d' style.css",
            "ls /nexistepas/vraiment",
        ] {
            assert_eq!(bash(command), Verdict::Allow, "refusé à tort : {command}");
        }
        // The same shapes under the user's home are real paths.
        assert!(bash("shot /home/u/habitudes 1024").is_denied());
        // Escalation makes everything writable: back to the strict reading.
        assert!(bash("sudo cp x /srv/ailleurs").is_denied());
        assert!(bash("sudo rm -rf /var/lib/quelque-chose").is_denied());
    }

    #[test]
    fn without_a_filesystem_the_reading_is_strict() {
        // A boundary that was not told what is writable assumes everything is,
        // so the decision can only err on the side of refusing.
        let strict = Boundary::new("/home/u/worktrees/depot/1-cache");
        assert!(check(&strict, "Bash", &json!({"command": "shot /habitudes"})).is_denied());
    }

    #[test]
    fn the_cache_directory_is_scratch_space_too() {
        // Playwright installs its browsers there; an agent was refused for it.
        assert_eq!(
            bash("ls ~/.cache/ms-playwright && npx playwright install chromium"),
            Verdict::Allow
        );
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(write(&format!("{home}/.cache/orchestra/x.png")), Verdict::Allow);
        // A neighbour of the cache is not the cache.
        assert!(write(&format!("{home}/.cache-bis/x")).is_denied());
        assert!(bash(&format!("cat {home}/.config/app/state.json")).is_denied());
    }

    #[test]
    fn the_temporary_directory_is_scratch_space_not_an_escape() {
        // An agent writes logs, screenshots and throwaway configuration there;
        // it holds nothing of the user's.
        for command in [
            "python3 app.py > /tmp/st.log 2>&1",
            "XDG_CONFIG_HOME=$(mktemp -d) xvfb-run -a python3 shot.py",
            "cp capture.png /tmp/capture.png",
            "cat /var/tmp/notes",
        ] {
            assert_eq!(bash(command), Verdict::Allow, "refusé à tort : {command}");
        }
        assert_eq!(write("/tmp/sortie.log"), Verdict::Allow);
        // A directory that merely starts like it is not it.
        assert!(write("/tmpvolé/x").is_denied());
    }

    #[test]
    fn a_command_naming_a_path_outside_the_worktree_is_refused() {
        // The shape of the v1 accident, and the ways it could be spelled.
        for command in [
            "mv *.md /home/u/projets/depot/docs/",
            "cp secret.txt /home/u/ailleurs",
            "rm /home/u/projets/depot/README.md",
            "echo x > /home/u/.ssh/config",
            "cat ~/.aws/credentials",
        ] {
            let v = bash(command);
            assert!(v.is_denied(), "aurait dû être refusé : {command}");
            assert!(v.reason().unwrap().contains("hors du worktree"));
        }
    }

    #[test]
    fn git_commands_that_move_work_out_are_refused() {
        for sub in ["push", "checkout", "switch", "worktree", "remote", "clean"] {
            let v = bash(&format!("git {sub} quelque-chose"));
            assert!(v.is_denied(), "git {sub} aurait dû être refusé");
            assert!(v.reason().unwrap().contains(sub));
        }
        // Options before the subcommand do not hide it.
        assert!(bash("git -C . push origin main").is_denied());
        assert!(bash("cd src && git push").is_denied());
        // Committing and reading history stay allowed.
        assert_eq!(bash("git log --oneline -5"), Verdict::Allow);
        assert_eq!(bash("git commit -am wip"), Verdict::Allow);
        // Listing worktrees only says where the agent is; adding one is
        // still refused.
        assert_eq!(bash("git worktree list"), Verdict::Allow);
        assert_eq!(bash("git worktree list --porcelain"), Verdict::Allow);
        assert!(bash("git worktree add ../autre").is_denied());
        assert!(bash("git worktree remove x").is_denied());
    }

    #[test]
    fn the_integrator_may_use_git_but_still_only_here() {
        let b = boundary().with_full_git();
        let bash = |c: &str| check(&b, "Bash", &json!({ "command": c }));
        for command in [
            "git fetch origin",
            "git merge origin/main",
            "git rebase main",
            "git push origin HEAD",
            "git checkout --theirs src/lib.rs",
            "gh pr create --fill",
        ] {
            assert_eq!(bash(command), Verdict::Allow, "refusé à tort : {command}");
        }
        // Git does not become a way out of the box.
        for command in [
            "git -C /home/u/projets/depot merge orch/1-cache",
            "git -C ../../projets/depot push",
            "git --git-dir=/home/u/projets/depot/.git log",
            "cp x /home/u/projets/depot/y",
        ] {
            assert!(
                bash(command).is_denied(),
                "aurait dû être refusé : {command}"
            );
        }
        assert_eq!(bash("git -C . push"), Verdict::Allow, "son propre worktree");
    }

    #[test]
    fn an_unknown_policy_confines_rather_than_opens() {
        assert_eq!(GitPolicy::parse("full"), GitPolicy::Full);
        for s in ["", "Full", "oui", "confined", "n'importe quoi"] {
            assert_eq!(GitPolicy::parse(s), GitPolicy::Confined, "{s}");
        }
    }

    #[test]
    fn catastrophic_commands_are_refused_wherever_they_point() {
        for command in [
            "rm -rf /",
            "rm  -rf  /",
            "sudo shutdown now",
            "mkfs.ext4 /dev/sda",
        ] {
            let v = bash(command);
            assert!(v.is_denied(), "aurait dû être refusé : {command}");
        }
    }

    #[test]
    fn reading_is_not_confined_only_writing_is() {
        // An agent must be able to read the wider codebase to understand it.
        assert_eq!(
            check(
                &boundary(),
                "Read",
                &json!({"file_path": "/home/u/projets/depot/src/main.rs"})
            ),
            Verdict::Allow
        );
        assert_eq!(
            check(
                &boundary(),
                "Grep",
                &json!({"pattern": "TODO", "path": "/home/u"})
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn extra_directories_widen_the_box() {
        let mut b = boundary();
        b.extra.push(PathBuf::from("/srv/partage"));
        assert_eq!(
            check(&b, "Write", &json!({"file_path": "/srv/partage/notes.md"})),
            Verdict::Allow
        );
        assert!(check(&b, "Write", &json!({"file_path": "/srv/ailleurs"})).is_denied());
    }

    #[test]
    fn dot_dot_does_not_sneak_out() {
        assert!(write("/home/u/worktrees/depot/1-cache/../2-autre/x").is_denied());
        assert_eq!(
            write("/home/u/worktrees/depot/1-cache/src/../lib.rs"),
            Verdict::Allow
        );
    }

    #[test]
    fn a_sibling_directory_with_a_shared_prefix_is_not_inside() {
        // `/…/1-cache-bis` must not pass as `/…/1-cache`.
        assert!(write("/home/u/worktrees/depot/1-cache-bis/x.rs").is_denied());
    }

    #[test]
    fn quoted_paths_are_still_seen() {
        assert!(bash("cp x \"/home/u/ailleurs/y\"").is_denied());
        assert!(bash("cp x '/home/u/ailleurs/y'").is_denied());
    }

    #[test]
    fn an_empty_or_odd_call_does_not_panic() {
        assert_eq!(bash(""), Verdict::Allow);
        assert_eq!(bash("   "), Verdict::Allow);
        assert_eq!(check(&boundary(), "Bash", &json!({})), Verdict::Allow);
        assert_eq!(check(&boundary(), "Write", &json!({})), Verdict::Allow);
        assert_eq!(
            check(&boundary(), "Inconnu", &json!({"x": 1})),
            Verdict::Allow
        );
    }
}

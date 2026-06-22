//! Diff de texte **par lignes** (sans dépendance) pour visualiser les changements de fichiers
//! réalisés par les agents. Partagé TUI ⇄ GUI.

/// Plafond de lignes au-delà duquel on n'effectue pas le diff détaillé (coût mémoire LCS).
const MAX_LINES: usize = 2_000;
/// Plafond de caractères du diff renvoyé (au-delà : tronqué).
const MAX_DIFF_CHARS: usize = 6_000;

/// Compare `before` et `after` ligne à ligne. Renvoie `(lignes ajoutées, lignes retirées, diff)`
/// où `diff` est un texte unifié simple (`  ` inchangé, `- ` retiré, `+ ` ajouté).
pub fn summarize(before: &str, after: &str) -> (usize, usize, String) {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();

    // Cas volumineux : on évite la table LCS (n*m) et on renvoie un résumé grossier.
    if a.len() > MAX_LINES || b.len() > MAX_LINES {
        let added = b.len().saturating_sub(a.len());
        let removed = a.len().saturating_sub(b.len());
        return (added, removed, "(fichier volumineux — diff détaillé non affiché)".to_string());
    }

    let (n, m) = (a.len(), b.len());
    // dp[i][j] = longueur de la plus longue sous-séquence commune de a[i..] et b[j..].
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut out = String::new();
    let (mut i, mut j, mut added, mut removed) = (0, 0, 0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push_str("  ");
            out.push_str(a[i]);
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push_str("- ");
            out.push_str(a[i]);
            removed += 1;
            i += 1;
        } else {
            out.push_str("+ ");
            out.push_str(b[j]);
            added += 1;
            j += 1;
        }
        out.push('\n');
    }
    while i < n {
        out.push_str("- ");
        out.push_str(a[i]);
        out.push('\n');
        removed += 1;
        i += 1;
    }
    while j < m {
        out.push_str("+ ");
        out.push_str(b[j]);
        out.push('\n');
        added += 1;
        j += 1;
    }

    if out.len() > MAX_DIFF_CHARS {
        out.truncate(MAX_DIFF_CHARS);
        out.push_str("\n…(diff tronqué)");
    }
    (added, removed, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_file_counts_all_added() {
        let (added, removed, _) = summarize("", "a\nb\nc");
        assert_eq!((added, removed), (3, 0));
    }

    #[test]
    fn detects_added_and_removed_lines() {
        let (added, removed, diff) = summarize("a\nb\nc", "a\nB\nc\nd");
        assert_eq!(added, 2); // « B » et « d »
        assert_eq!(removed, 1); // « b »
        assert!(diff.contains("- b"));
        assert!(diff.contains("+ B"));
        assert!(diff.contains("+ d"));
        assert!(diff.contains("  a"));
    }

    #[test]
    fn identical_has_no_changes() {
        let (added, removed, _) = summarize("x\ny", "x\ny");
        assert_eq!((added, removed), (0, 0));
    }
}

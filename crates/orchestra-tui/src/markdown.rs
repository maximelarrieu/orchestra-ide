//! Rendu Markdown léger vers des lignes ratatui, pour le visualiseur de documents.
//!
//! Couvre les constructions courantes (titres, listes, citations, blocs de code, règles
//! horizontales, gras `**…**` et code `` `…` `` en ligne). Volontairement simple : un
//! terminal n'affiche pas du Markdown riche, l'objectif est une lecture claire.

use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};

/// Convertit un texte Markdown en lignes stylées.
pub fn to_lines(md: &str) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut in_code = false;

    for raw in md.split('\n') {
        let trimmed = raw.trim_start();

        if trimmed.starts_with("```") {
            in_code = !in_code;
            out.push(Line::from(Span::styled(raw.to_string(), Style::new().dark_gray())));
            continue;
        }
        if in_code {
            out.push(Line::from(Span::styled(format!("  {raw}"), Style::new().dark_gray())));
            continue;
        }
        if let Some(line) = heading(trimmed) {
            out.push(line);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            let mut spans = vec![Span::styled("  • ", Style::new().cyan())];
            spans.extend(inline(rest));
            out.push(Line::from(spans));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("> ") {
            let mut spans = vec![Span::styled("▏ ", Style::new().dark_gray())];
            spans.extend(inline(rest));
            out.push(Line::from(spans));
            continue;
        }
        if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-') {
            out.push(Line::from(Span::styled("─".repeat(40), Style::new().dark_gray())));
            continue;
        }
        out.push(Line::from(inline(raw)));
    }

    out
}

fn heading(t: &str) -> Option<Line<'static>> {
    let level = t.chars().take_while(|&c| c == '#').count();
    if level == 0 || level > 6 || !t[level..].starts_with(' ') {
        return None;
    }
    let rest = t[level..].trim_start();
    let style = match level {
        1 => Style::new().cyan().bold(),
        2 => Style::new().magenta().bold(),
        _ => Style::new().bold(),
    };
    let prefix = "#".repeat(level);
    Some(Line::from(Span::styled(format!("{prefix} {rest}"), style)))
}

/// Découpe une ligne en spans, en gérant le gras `**…**` et le code `` `…` ``.
fn inline(s: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut bold = false;
    let mut chars = s.chars().peekable();

    fn flush(buf: &mut String, bold: bool, spans: &mut Vec<Span<'static>>) {
        if !buf.is_empty() {
            let style = if bold { Style::new().bold() } else { Style::new() };
            spans.push(Span::styled(std::mem::take(buf), style));
        }
    }

    while let Some(c) = chars.next() {
        if c == '*' && chars.peek() == Some(&'*') {
            chars.next();
            flush(&mut buf, bold, &mut spans);
            bold = !bold;
        } else if c == '`' {
            flush(&mut buf, bold, &mut spans);
            let mut code = String::new();
            while let Some(&n) = chars.peek() {
                chars.next();
                if n == '`' {
                    break;
                }
                code.push(n);
            }
            spans.push(Span::styled(code, Style::new().yellow()));
        } else {
            buf.push(c);
        }
    }
    flush(&mut buf, bold, &mut spans);
    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reconstitue le texte brut d'une ligne (concatène ses spans).
    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn renders_common_blocks() {
        let md = "# Titre\n\n- point\n> cite\n`code` et **gras**\n```\nbloc\n```";
        let lines = to_lines(md);
        assert_eq!(plain(&lines[0]), "# Titre");
        assert!(plain(&lines[2]).contains("point"));
        assert!(plain(&lines[3]).contains("cite"));
        // La ligne avec code+gras conserve le texte, sans les marqueurs ** et `.
        let mixed = plain(&lines[4]);
        assert!(mixed.contains("code") && mixed.contains("gras"));
        assert!(!mixed.contains('*'));
    }

    #[test]
    fn non_heading_hash_is_not_a_title() {
        // '#texte' sans espace n'est pas un titre.
        let lines = to_lines("#pasuntitre");
        assert_eq!(plain(&lines[0]), "#pasuntitre");
    }
}

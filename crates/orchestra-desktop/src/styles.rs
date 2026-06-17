//! Feuille de style de la fenêtre (injectée via un nœud `<style>`).

pub const CSS: &str = r#"
    body { margin: 0; background: #0b0e14; color: #e6e6e6; }
    .app { font-family: ui-sans-serif, system-ui, sans-serif; padding: 1rem 1.25rem; }
    h1 { font-size: 1.3rem; } h2 { color: #8ab4ff; margin: .2rem 0; }
    .agents { color: #9aa; }
    .actions { margin: .6rem 0; }
    button { background: #1d2535; color: #e6e6e6; border: 1px solid #2c3a55;
             border-radius: 6px; padding: .45rem .8rem; cursor: pointer; font-size: .95rem; }
    button:hover { background: #263150; }
    button.go { background: #1f5132; border-color: #2e7d4a; margin-left: .5rem; }
    .plan { list-style: none; padding-left: 0; }
    .plan li { padding: .2rem .4rem; border-left: 3px solid #2c3a55; margin: .2rem 0; }
    .radar { background: #05070c; color: #cdd6e0; padding: .6rem; border-radius: 6px;
             max-height: 320px; overflow: auto; white-space: pre-wrap; font-size: .85rem; }
    .error { color: #ff8a8a; }
"#;

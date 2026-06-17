//! Feuille de style de la fenêtre (injectée via un nœud `<style>`).

pub const CSS: &str = r#"
    body { margin: 0; background: #0b0e14; color: #e6e6e6; }
    .app { font-family: ui-sans-serif, system-ui, sans-serif; padding: 1rem 1.25rem; }
    h1 { font-size: 1.3rem; margin: .2rem 0 .6rem; }
    h2 { color: #8ab4ff; margin: .2rem 0; }
    h3 { margin: .6rem 0 .3rem; }
    .agents { color: #9aa; }

    input, textarea { background: #0f1420; color: #e6e6e6; border: 1px solid #2c3a55;
                      border-radius: 6px; padding: .4rem; font: inherit; }
    .spacebar { display: flex; gap: .5rem; align-items: center; margin-bottom: .5rem; }
    .spacebar input { flex: 1; }
    .spacename { color: #8ab4ff; }

    .nav { display: flex; gap: .4rem; margin: .5rem 0 .8rem; border-bottom: 1px solid #1c2535; padding-bottom: .5rem; }
    .tab { background: #1d2535; color: #cdd6e0; border: 1px solid #2c3a55; border-radius: 6px;
           padding: .4rem .8rem; cursor: pointer; }
    .tab.on { background: #2e7d4a; border-color: #3aa55f; color: #fff; }

    button { background: #1d2535; color: #e6e6e6; border: 1px solid #2c3a55;
             border-radius: 6px; padding: .45rem .8rem; cursor: pointer; font-size: .95rem; }
    button:hover { background: #263150; }
    button.go { background: #1f5132; border-color: #2e7d4a; margin-left: .5rem; }

    .actions { margin: .6rem 0; }
    .goal { width: 100%; min-height: 60px; box-sizing: border-box; }

    .cols { display: flex; gap: 1rem; align-items: flex-start; }
    .list { list-style: none; padding-left: 0; margin: 0; min-width: 240px; }
    .detail { flex: 1; }
    .row { background: none; border: none; color: #cdd6e0; cursor: pointer; text-align: left;
           padding: .25rem .4rem; width: 100%; }
    .row:hover { color: #fff; }
    .row.on { color: #8ab4ff; font-weight: 600; }

    .plan { list-style: none; padding-left: 0; }
    .plan li { padding: .2rem .4rem; border-left: 3px solid #2c3a55; margin: .2rem 0; }
    .radar, .viewer { background: #05070c; color: #cdd6e0; padding: .6rem; border-radius: 6px;
                      max-height: 360px; overflow: auto; white-space: pre-wrap; font-size: .85rem; }
    .viewer { flex: 1; }
    .error { color: #ff8a8a; }

    /* Chat */
    .chat { display: flex; flex-direction: column; gap: .6rem; }
    .messages { display: flex; flex-direction: column; gap: .5rem; max-height: 420px;
                overflow: auto; padding: .4rem; background: #05070c; border-radius: 8px; }
    .bubble { max-width: 78%; padding: .5rem .7rem; border-radius: 10px; }
    .bubble .who { display: block; font-size: .72rem; color: #8ab4ff; margin-bottom: .15rem; }
    .bubble .text { white-space: pre-wrap; }
    .bubble.user { align-self: flex-end; background: #1f5132; }
    .bubble.coord { align-self: flex-start; background: #1d2535; border-left: 3px solid #8ab4ff; }
    .bubble.agent { align-self: flex-start; background: #11161f; max-width: 88%; }
    .bubble.agent .text { margin-top: .35rem; color: #aab4c0; border-top: 1px dashed #2c3a55; padding-top: .35rem; }
    .bubble.system { align-self: center; background: transparent; color: #788; font-size: .8rem; }
    .disclosure { background: none; border: none; color: #8ab4ff; cursor: pointer;
                  padding: 0; font-size: .8rem; }
    .disclosure:hover { color: #fff; background: none; }
    .planbox { border: 1px solid #2c3a55; border-radius: 8px; padding: .5rem .7rem; }
    .composer { display: flex; gap: .5rem; }
    .chatinput { flex: 1; }
"#;

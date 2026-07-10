//! Feuille de style de la fenêtre.
//!
//! **Thème clair par défaut**, **thème sombre** en option (classe `.app.dark`). Tout passe par
//! des variables CSS pour que le basculement soit instantané. Layout façon Cursor : barre
//! supérieure (onglets de session), puis un shell 5 zones —
//! checkpoints · explorateur · conversation · visualiseur · rail Tâches — et une barre de statut.

pub const CSS: &str = r#"
    /* ---- Thème CLAIR (défaut) ---- */
    :root {
        --bg: #ffffff;
        --bg-elev: #f6f7f9;
        --bg-titlebar: #eceef1;
        --bg-statusbar: #0969da;
        --bg-input: #ffffff;
        --bg-hover: #eceff2;
        --bg-sel: #dcecff;
        --border: #e2e5e9;
        --border-strong: #cfd4da;
        --text: #1f2328;
        --text-dim: #57606a;
        --text-mute: #8b949e;
        --accent: #0969da;
        --accent-hover: #0a6ecf;
        --focus: #0969da;
        --link: #0969da;
        --add: #1a7f37;
        --add-bg: rgba(26,127,55,.10);
        --del: #cf222e;
        --del-bg: rgba(207,34,46,.10);
        --danger: #cf222e;
        --danger-hover: #a40e26;
        --heading: #1f2328;
        --code-bg: #f1f2f4;
        --bubble-user: #dcecff;
        --bubble-user-text: #0a3d75;
        --bubble-agent: #f1f2f4;
        --radius: 6px;
        --font: "Segoe UI", system-ui, -apple-system, sans-serif;
        --mono: "Cascadia Code", "Consolas", ui-monospace, Menlo, monospace;
    }
    /* ---- Thème SOMBRE ---- */
    .app.dark {
        --bg: #1e1e1e;
        --bg-elev: #252526;
        --bg-titlebar: #323233;
        --bg-statusbar: #0e639c;
        --bg-input: #3c3c3c;
        --bg-hover: #2a2d2e;
        --bg-sel: #094771;
        --border: #3c3c3c;
        --border-strong: #474747;
        --text: #cccccc;
        --text-dim: #9d9d9d;
        --text-mute: #858585;
        --accent: #0e639c;
        --accent-hover: #1177bb;
        --focus: #4fa3e0;
        --link: #4fc1ff;
        --add: #89d185;
        --add-bg: rgba(137,209,133,.12);
        --del: #f48771;
        --del-bg: rgba(244,135,113,.12);
        --danger: #c74e39;
        --danger-hover: #e15c46;
        --heading: #e7e7e7;
        --code-bg: #1b1b1b;
        --bubble-user: #094771;
        --bubble-user-text: #e7f1ff;
        --bubble-agent: #2d2d2d;
    }

    * { box-sizing: border-box; }
    body { margin: 0; background: var(--bg); color: var(--text); }

    /* Coquille plein écran : barre de statut fixe (26px) réservée en bas. */
    .app { font-family: var(--font); font-size: 13px; height: calc(100vh - 26px);
           display: flex; flex-direction: column; overflow: hidden;
           background: var(--bg); color: var(--text); }

    /* ---- Barre supérieure ---- */
    .topbar { display: flex; align-items: center; gap: .5rem; background: var(--bg-titlebar);
              border-bottom: 1px solid var(--border); padding: 0 .8rem; min-height: 40px; }
    .brand { font-weight: 700; white-space: nowrap; letter-spacing: .2px; margin-right: .4rem; }
    .topspacer { flex: 1; }
    .tabadd { background: none; border: none; color: var(--text-dim); font-size: 1.1rem;
              cursor: pointer; padding: .1rem .5rem; border-radius: var(--radius); }
    .tabadd:hover { background: var(--bg-hover); color: var(--text); }
    .icontoggle { background: none; border: 1px solid transparent; color: var(--text-dim);
                  cursor: pointer; padding: .25rem .5rem; border-radius: var(--radius); font-size: .95rem; }
    .icontoggle:hover { background: var(--bg-hover); color: var(--text); border-color: var(--border); }

    /* Onglets de session (browser-like) */
    .sessiontabs { display: flex; flex-wrap: wrap; gap: .25rem; align-items: center; }
    .stab { display: inline-flex; align-items: center; background: transparent;
            border: 1px solid transparent; border-radius: var(--radius); overflow: hidden; }
    .stab.on { background: var(--bg); border-color: var(--border); }
    .stablabel { background: none; border: none; color: var(--text-dim); padding: .3rem .6rem; cursor: pointer; font: inherit; }
    .stab.on .stablabel { color: var(--text); }
    .stablabel:hover { color: var(--text); }
    .stabx { background: none; border: none; color: var(--text-mute); padding: .3rem .45rem; cursor: pointer; }
    .stabx:hover { color: var(--del); }

    /* ---- Shell 5 zones ---- */
    .ide { flex: 1; min-height: 0; display: flex; }
    .pane { min-height: 0; }
    .pane.explorerpane { width: 232px; flex: none; border-right: 1px solid var(--border);
                         background: var(--bg-elev); overflow: auto; }
    .pane.conversation { flex: 1.4; min-width: 340px; display: flex; flex-direction: column;
                         overflow: hidden; background: var(--bg); }
    .pane.viewer { flex: 1; min-width: 300px; display: flex; flex-direction: column;
                   overflow: hidden; border-left: 1px solid var(--border); }

    /* Rail Checkpoints (fin) */
    .checkpoints { width: 46px; flex: none; background: var(--bg-elev); border-right: 1px solid var(--border);
                   display: flex; flex-direction: column; align-items: center; padding: .6rem 0; }
    .cplabel { writing-mode: vertical-rl; transform: rotate(180deg); color: var(--text-mute);
               font-size: .62rem; letter-spacing: 2px; margin-bottom: .8rem; user-select: none; }
    .cpdots { display: flex; flex-direction: column; gap: .7rem; align-items: center; }
    .cpdot { width: 11px; height: 11px; border-radius: 50%; border: 2px solid var(--border-strong);
             background: transparent; cursor: pointer; padding: 0; }
    .cpdot:hover { border-color: var(--accent); }
    .cpdot.on { background: var(--add); border-color: var(--add); }

    /* ---- Conversation ---- */
    .convhead { display: flex; align-items: center; gap: .5rem; padding: .5rem .8rem;
                border-bottom: 1px solid var(--border); background: var(--bg-elev); }
    .agentname { font-weight: 600; }
    .statuspill { font-size: .72rem; padding: .1rem .5rem; border-radius: 10px;
                  background: var(--add-bg); color: var(--add); }
    .convspacer { flex: 1; }
    .squad { display: flex; flex-wrap: wrap; gap: .35rem; align-items: center; padding: .4rem .8rem 0; }
    .agentchip { background: var(--bg-input); border: 1px solid var(--border); border-radius: 12px;
                 padding: .15rem .55rem; font-size: .74rem; color: var(--text-dim); }
    .agentchip.thinking { color: #b8860b; border-color: #d9c17a; }
    .agentchip.working { color: var(--add); border-color: var(--add); }
    .agentchip.done { color: var(--text-mute); }
    .actions { display: flex; flex-wrap: wrap; gap: .4rem; align-items: center; padding: .5rem .8rem; }
    .hint { color: var(--text-mute); font-size: .85rem; padding: 1rem .8rem; }
    .chatcol { flex: 1; min-height: 0; display: flex; padding: 0 .8rem .6rem; }

    /* Chat */
    .chat { display: flex; flex-direction: column; gap: .6rem; flex: 1; min-height: 0; }
    .messages { display: flex; flex-direction: column; gap: .5rem; flex: 1; min-height: 120px;
                overflow: auto; padding: .4rem; }
    .bubble { max-width: 82%; padding: .5rem .7rem; border-radius: 10px; }
    .bubble .who { display: block; font-size: .72rem; color: var(--link); margin-bottom: .15rem; }
    .bubble .text { white-space: pre-wrap; }
    .bubble.user { align-self: flex-end; background: var(--bubble-user); color: var(--bubble-user-text); }
    .bubble.coord { align-self: flex-start; background: var(--bg-elev); border: 1px solid var(--border); }
    .bubble.agent { align-self: flex-start; background: var(--bubble-agent); max-width: 92%; }
    .bubble.agent .text { margin-top: .35rem; color: var(--text-dim);
                          border-top: 1px dashed var(--border); padding-top: .35rem; }
    .bubble.system { align-self: center; background: transparent; color: var(--text-mute); font-size: .8rem; }
    .disclosure { background: none; border: none; cursor: pointer; color: var(--link); font: inherit; padding: 0; }
    .composer { display: flex; gap: .5rem; align-items: flex-end; border: 1px solid var(--border);
                border-radius: var(--radius); padding: .4rem; background: var(--bg-elev); }
    .chatinput { flex: 1; resize: none; min-height: 2.4rem; max-height: 8rem; line-height: 1.35;
                 border: none; background: transparent; }
    .chatinput:focus { border: none; }
    button.send { background: var(--accent); color: #fff; border: none; border-radius: var(--radius);
                  width: 34px; height: 34px; cursor: pointer; font-size: 1rem; }
    button.send:hover { background: var(--accent-hover); }

    /* ---- Explorateur (« orchestre en verre ») ---- */
    .explorer { padding: .3rem 0; }
    .explorerhead { padding: .5rem .6rem; font-weight: 600; color: var(--text-dim);
                    text-transform: uppercase; font-size: .72rem; letter-spacing: .5px;
                    position: sticky; top: 0; background: var(--bg-elev); }
    .tree { list-style: none; margin: 0; padding: 0; }
    .tree li { margin: 0; }
    .trow { display: flex; align-items: center; justify-content: space-between; gap: .4rem;
            padding: .1rem .5rem; border-left: 2px solid transparent; }
    .trow .tname { background: none; border: none; color: var(--text); padding: 0; text-align: left;
                   cursor: default; font: inherit; white-space: nowrap; overflow: hidden;
                   text-overflow: ellipsis; }
    .trow.file .tname { cursor: pointer; }
    .trow.file:hover { background: var(--bg-hover); }
    .trow.on { background: var(--bg-sel); }
    .trow.act-read { border-left-color: var(--link); }
    .trow.act-write { border-left-color: var(--add); }
    .actbadge { font-size: .66rem; padding: 0 .3rem; border-radius: 8px; white-space: nowrap; }
    .actbadge.read { color: var(--link); background: rgba(9,105,218,.10); }
    .actbadge.write { color: var(--add); background: var(--add-bg); }

    /* ---- Visualiseur de fichier (code + diff) ---- */
    .filepane { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg); }
    .filepane .centerhead { display: flex; align-items: center; gap: .6rem; padding: .4rem .7rem;
                            border-bottom: 1px solid var(--border); background: var(--bg-elev); }
    .filepane .centerhead .path { flex: 1; font-family: var(--mono); font-size: .8rem; color: var(--text-dim);
                                  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .centerbody { flex: 1; min-height: 0; overflow: auto; padding: .5rem .7rem; }
    .filepane .welcome { padding: 2rem; text-align: center; margin: auto; max-width: 480px; }
    .codeview { font-family: var(--mono); font-size: .82rem; white-space: pre-wrap; margin: 0; color: var(--text); }
    .editor { width: 100%; height: 100%; min-height: 50vh; resize: none; font-family: var(--mono);
              font-size: .82rem; line-height: 1.5; border: 1px solid var(--border); border-radius: var(--radius); }

    /* Terminal (sous le code) */
    .termpanel { flex: none; height: 190px; display: flex; flex-direction: column;
                 border-top: 1px solid var(--border); background: var(--bg-elev); }
    .termhead { padding: .3rem .7rem; font-size: .7rem; letter-spacing: .5px; color: var(--text-dim);
                text-transform: uppercase; border-bottom: 1px solid var(--border); }
    .termbody { flex: 1; overflow: auto; padding: .4rem .7rem; }
    .termrun { margin-bottom: .5rem; }
    .termcmd { font-family: var(--mono); font-size: .8rem; color: var(--accent); }
    .termout { font-family: var(--mono); font-size: .78rem; white-space: pre-wrap; margin: .2rem 0;
               color: var(--text-dim); }
    .termexit { font-size: .72rem; color: var(--text-mute); }

    /* ---- Rail Tâches / Mémoire / Contexte ---- */
    .taskrail { width: 300px; flex: none; border-left: 1px solid var(--border); background: var(--bg-elev);
                overflow: auto; padding: .3rem .8rem 1rem; }
    .railhead { display: flex; align-items: center; justify-content: space-between; margin: 1rem 0 .5rem;
                font-size: .72rem; letter-spacing: .5px; color: var(--text-dim); text-transform: uppercase; }
    .railcount { color: var(--text-mute); }
    .plan { list-style: none; padding: 0; margin: 0; }
    .planrow { display: flex; gap: .5rem; align-items: flex-start; padding: .35rem .1rem; }
    .planicon { color: var(--text-mute); width: 1rem; }
    .planrow.done .planicon { color: var(--add); }
    .planrow.done .planobj { text-decoration: line-through; color: var(--text-mute); }
    .planrow.running .planicon { color: var(--accent); }
    .planrow.failed .planicon { color: var(--del); }
    .plantext { display: flex; flex-direction: column; }
    .planobj { color: var(--text); font-size: .86rem; }
    .planagent { color: var(--text-mute); font-size: .74rem; }
    .approvecard { border: 1px solid var(--accent); border-radius: var(--radius); padding: .6rem;
                   margin: .4rem 0; display: flex; flex-direction: column; gap: .4rem; background: var(--bg); }
    .memcard { background: var(--bg); border: 1px solid var(--border); border-radius: var(--radius);
               padding: .4rem .6rem; margin-bottom: .35rem; font-size: .84rem; }
    .ctxrow { font-family: var(--mono); font-size: .8rem; color: var(--text-dim); padding: .15rem 0; }
    .small { font-size: .8rem; }
    .muted { color: var(--text-mute); }

    /* ---- Barre d'espaces (ouvrir / créer / récents) ---- */
    .spaces { background: var(--bg-elev); border-bottom: 1px solid var(--border); padding: .6rem 1rem; }
    .spacebar { display: flex; gap: .5rem; align-items: center; flex-wrap: wrap; }
    .spacename { color: var(--accent); white-space: nowrap; font-weight: 600; }
    .chips { display: flex; flex-wrap: wrap; gap: .4rem; align-items: center; margin-top: .5rem; }
    .chip { display: inline-flex; align-items: center; background: var(--bg-input);
            border: 1px solid var(--border); border-radius: 12px; overflow: hidden; }
    .chiplabel { background: none; border: none; cursor: pointer; color: var(--text); padding: .2rem .6rem; font: inherit; }
    .chiplabel:hover { background: var(--bg-hover); }
    .chipx { background: none; border: none; cursor: pointer; color: var(--text-mute); padding: .2rem .5rem; }
    .chipx:hover { color: var(--del); }
    .newspace { display: flex; flex-direction: column; gap: .5rem; background: var(--bg);
                border: 1px solid var(--border); border-radius: var(--radius); padding: .8rem; margin-top: .5rem; }
    .browser { margin-top: .5rem; background: var(--bg); border: 1px solid var(--border);
               border-radius: var(--radius); padding: .5rem; }
    .browsebar { display: flex; gap: .6rem; align-items: center; margin-bottom: .4rem; }
    .browser .list { max-height: 300px; overflow: auto; }
    .list { list-style: none; padding-left: 0; margin: 0; }
    .skillrow { display: flex; align-items: center; gap: .3rem; }
    .row { background: none; border: none; cursor: pointer; color: var(--text); text-align: left;
           padding: .25rem .4rem; border-radius: var(--radius); font: inherit; }
    .row:hover { background: var(--bg-hover); }
    .linklike { background: none; border: none; cursor: pointer; color: var(--link); font: inherit; padding: 0 .3rem; }
    .linklike:hover { text-decoration: underline; }
    .fichearea { width: 100%; min-height: 120px; resize: vertical; font-family: var(--mono); }
    .error { color: var(--del); }

    /* Boutons génériques */
    button { background: var(--bg-input); color: var(--text); border: 1px solid var(--border);
             border-radius: var(--radius); padding: .35rem .7rem; cursor: pointer; font: inherit; }
    button:hover { background: var(--bg-hover); }
    button.go { background: var(--accent); color: #fff; border-color: var(--accent); }
    button.go:hover { background: var(--accent-hover); }
    button.ghost { background: none; border: none; color: var(--link); padding: .2rem .4rem; }
    button.ghost:hover { background: var(--bg-hover); }
    input, textarea { background: var(--bg-input); color: var(--text); border: 1px solid var(--border);
                      border-radius: var(--radius); padding: .4rem .55rem; font: inherit; outline: none; }
    input:focus, textarea:focus { border-color: var(--focus); }
    input::placeholder, textarea::placeholder { color: var(--text-mute); }
    h2 { color: var(--heading); font-size: 1.05rem; margin: .3rem 0; }

    /* ---- Barre de statut ---- */
    .statusbar { position: fixed; left: 0; right: 0; bottom: 0; height: 26px;
                 background: var(--bg-statusbar); color: #fff; display: flex; align-items: center;
                 gap: 1rem; padding: 0 .8rem; font-size: 12px; }
    .statusbar .sb-item { display: inline-flex; align-items: center; gap: .35rem; }

    /* ---- Diff ---- */
    .diff { font-family: var(--mono); font-size: .82rem; }
    .dl { white-space: pre-wrap; padding: 0 .3rem; }
    .dl.add { color: var(--add); background: var(--add-bg); }
    .dl.del { color: var(--del); background: var(--del-bg); }
    .dl.ctx { color: var(--text-mute); }

    /* ---- Rendu Markdown ---- */
    .markdown { line-height: 1.6; }
    .markdown h1, .markdown h2, .markdown h3, .markdown h4 { color: var(--heading); line-height: 1.25;
        margin: 1rem 0 .5rem; }
    .markdown h1 { font-size: 1.5rem; border-bottom: 1px solid var(--border); padding-bottom: .3rem; }
    .markdown h2 { font-size: 1.25rem; border-bottom: 1px solid var(--border); padding-bottom: .25rem; }
    .markdown h3 { font-size: 1.08rem; }
    .markdown p { margin: .5rem 0; }
    .markdown ul, .markdown ol { padding-left: 1.4rem; margin: .4rem 0; }
    .markdown a { color: var(--link); }
    .markdown code { background: var(--code-bg); padding: .1rem .35rem; border-radius: 3px;
                     font-family: var(--mono); font-size: .88em; }
    .markdown pre { background: var(--code-bg); padding: .7rem; border-radius: var(--radius); overflow: auto;
                    border: 1px solid var(--border); }
    .markdown pre code { background: none; padding: 0; }
    .markdown blockquote { border-left: 3px solid var(--border-strong); margin: .5rem 0; padding: .1rem .8rem;
                           color: var(--text-dim); }
    .markdown table { border-collapse: collapse; margin: .6rem 0; }
    .markdown th, .markdown td { border: 1px solid var(--border); padding: .3rem .6rem; }
    .markdown th { background: var(--bg-elev); }
    .markdown hr { border: none; border-top: 1px solid var(--border); margin: 1rem 0; }
    .markdown .mermaid { background: #fff; padding: .8rem; border-radius: var(--radius); text-align: center; }

    /* ---- Scrollbars fines ---- */
    ::-webkit-scrollbar { width: 10px; height: 10px; }
    ::-webkit-scrollbar-thumb { background: var(--border-strong); border-radius: 5px; }
    ::-webkit-scrollbar-thumb:hover { background: var(--text-mute); }
    ::-webkit-scrollbar-track { background: transparent; }
"#;

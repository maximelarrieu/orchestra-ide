//! Feuille de style de la fenêtre — **thème VS Code (Dark+)**.
//!
//! Palette et composants calqués sur VS Code : barre de titre, onglets, boutons primaire/
//! secondaire, champs avec anneau de focus, listes avec survol/sélection, barre de statut,
//! scrollbars fines. Les classes correspondent aux composants existants (aucun changement de
//! structure requis).

pub const CSS: &str = r#"
    :root {
        --bg: #1e1e1e;            /* éditeur */
        --bg-elev: #252526;       /* panneaux / sidebar */
        --bg-titlebar: #323233;
        --bg-statusbar: #007acc;
        --bg-input: #3c3c3c;
        --bg-hover: #2a2d2e;
        --bg-sel: #094771;        /* sélection liste */
        --border: #3c3c3c;
        --border-strong: #474747;
        --text: #cccccc;
        --text-dim: #9d9d9d;
        --text-mute: #858585;
        --accent: #0e639c;        /* bouton primaire */
        --accent-hover: #1177bb;
        --focus: #007acc;         /* anneau de focus / onglet actif */
        --green: #4ec9b0;
        --add: #89d185;
        --del: #f48771;
        --danger: #c74e39;
        --danger-hover: #e15c46;
        --radius: 5px;
        --font: "Segoe UI", system-ui, -apple-system, sans-serif;
        --mono: "Cascadia Code", "Consolas", ui-monospace, Menlo, monospace;
    }

    * { box-sizing: border-box; }
    body { margin: 0; background: var(--bg); color: var(--text); }
    /* Coquille plein écran : en-tête + shell 3 panneaux + barre de statut fixe (26px). */
    .app { font-family: var(--font); font-size: 13px; height: calc(100vh - 26px);
           display: flex; flex-direction: column; overflow: hidden; }

    /* En-tête compact : marque + barre d'espaces */
    .topbar { display: flex; align-items: center; gap: 1rem; background: var(--bg-titlebar);
              border-bottom: 1px solid #1b1b1b; padding: 0 1rem; }
    .brand { font-weight: 600; color: #e7e7e7; white-space: nowrap; letter-spacing: .2px; }
    .topbar .spaces { flex: 1; border-bottom: none; background: transparent; padding: .45rem 0; }

    /* Shell 3 panneaux (façon Cursor) */
    .ide { flex: 1; min-height: 0; display: flex; }
    .pane { overflow: auto; }
    .pane.left { width: 260px; flex: none; border-right: 1px solid var(--border); background: var(--bg-elev); }
    .pane.center { flex: 1; min-width: 0; display: flex; flex-direction: column; background: var(--bg); }
    .pane.right { width: 400px; flex: none; border-left: 1px solid var(--border);
                  background: var(--bg-elev); display: flex; flex-direction: column; padding: 0 .8rem .6rem; }
    .righthead { display: flex; align-items: center; justify-content: space-between;
                 padding: .5rem 0 .3rem; font-weight: 600; color: var(--text-dim);
                 text-transform: uppercase; font-size: .78rem; letter-spacing: .4px; }
    button.ghost { background: none; color: #4fc1ff; padding: .2rem .4rem; }
    button.ghost:hover { background: var(--bg-hover); }

    /* Explorateur (panneau gauche) — arborescence annotée « orchestre en verre » */
    .explorer { padding: .3rem 0; }
    .explorerhead { padding: .4rem .6rem; font-weight: 600; color: var(--text-dim);
                    text-transform: uppercase; font-size: .76rem; letter-spacing: .4px;
                    position: sticky; top: 0; background: var(--bg-elev); }
    .tree { list-style: none; margin: 0; padding: 0; }
    .tree li { margin: 0; }
    .trow { display: flex; align-items: center; justify-content: space-between; gap: .4rem;
            padding: .12rem .5rem; border-left: 2px solid transparent; }
    .trow .tname { background: none; border: none; color: var(--text); padding: 0; text-align: left;
                   cursor: default; font: inherit; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .trow.file .tname { cursor: pointer; }
    .trow.file:hover { background: var(--bg-hover); }
    .trow.on { background: var(--bg-sel); }
    .trow.act-read { border-left-color: #4fc1ff; }
    .trow.act-write { border-left-color: var(--add); }
    .actbadge { font-size: .68rem; padding: 0 .3rem; border-radius: 8px; white-space: nowrap; }
    .actbadge.read { color: #4fc1ff; background: rgba(79,193,255,.12); }
    .actbadge.write { color: var(--add); background: rgba(137,209,133,.14); }

    /* Panneau central : en-tête (chemin + éditer) + corps (rendu/diff/texte) */
    .center .centerhead { display: flex; align-items: center; gap: .6rem; padding: .45rem .8rem;
                          border-bottom: 1px solid var(--border); background: var(--bg-elev); }
    .center .centerhead .path { flex: 1; font-family: var(--mono); font-size: .82rem; color: var(--text-dim);
                                white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .centerbody { flex: 1; min-height: 0; overflow: auto; padding: .6rem .8rem; }
    .center .welcome { padding: 2rem; text-align: center; margin: auto; max-width: 520px; }
    .codeview { font-family: var(--mono); font-size: .84rem; white-space: pre-wrap;
                margin: 0; color: var(--text); }
    .editor { width: 100%; height: 100%; min-height: 60vh; resize: none; font-family: var(--mono);
              font-size: .84rem; line-height: 1.5; }

    /* Barre de titre + barre de statut */
    h1 { font-size: 13px; font-weight: 600; color: #e7e7e7; margin: 0;
         background: var(--bg-titlebar); padding: .5rem 1rem; border-bottom: 1px solid #1b1b1b;
         letter-spacing: .2px; }
    .statusbar { position: fixed; left: 0; right: 0; bottom: 0; height: 26px;
                 background: var(--bg-statusbar); color: #fff; display: flex; align-items: center;
                 gap: 1rem; padding: 0 .8rem; font-size: 12px; }
    .statusbar .sb-item { display: inline-flex; align-items: center; gap: .35rem; }

    /* Contenu */
    .spaces, .nav, .orchestrate, .chatwrap, .agentsmenu, .cols { padding-left: 1rem; padding-right: 1rem; }
    h2 { color: #4fc1ff; font-size: 1.05rem; margin: .3rem 0; }
    h3 { color: #cccccc; font-size: .92rem; text-transform: uppercase; letter-spacing: .4px;
         margin: .8rem 0 .4rem; color: var(--text-dim); }
    h4 { color: var(--text-dim); font-size: .82rem; text-transform: uppercase; letter-spacing: .4px;
         margin: .7rem 0 .3rem; }
    .agents, .muted, .hint { color: var(--text-mute); }
    .hint { font-size: .82rem; margin: .1rem 0 .6rem; }
    .error { color: var(--del); }

    /* Champs */
    input, textarea {
        background: var(--bg-input); color: var(--text); border: 1px solid var(--bg-input);
        border-radius: var(--radius); padding: .4rem .55rem; font: inherit; outline: none;
    }
    input:focus, textarea:focus { border-color: var(--focus); }
    input::placeholder, textarea::placeholder { color: var(--text-mute); }

    /* Boutons : secondaire par défaut, primaire (.go), danger */
    button {
        background: #3a3d41; color: var(--text); border: 1px solid transparent;
        border-radius: var(--radius); padding: .4rem .8rem; cursor: pointer; font: inherit;
    }
    button:hover { background: #45494e; }
    button.go { background: var(--accent); color: #fff; }
    button.go:hover { background: var(--accent-hover); }
    button.danger { background: var(--danger); color: #fff; margin: .4rem 0; }
    button.danger:hover { background: var(--danger-hover); }
    .linklike, .disclosure, .chiplabel, .chipx, .row {
        background: none; border: none; cursor: pointer; color: #4fc1ff; padding: 0 .3rem; font: inherit;
    }
    .linklike:hover, .disclosure:hover { color: #9cdcfe; background: none; }
    .actions { display: flex; gap: .5rem; align-items: center; margin: .6rem 0; }

    /* Onglets (barre de navigation) */
    .nav { display: flex; gap: 0; margin: 0; background: var(--bg-elev);
           border-bottom: 1px solid #1b1b1b; padding: 0 .5rem; }
    .tab { background: transparent; color: var(--text-mute); border: none; border-top: 2px solid transparent;
           border-radius: 0; padding: .55rem .9rem; }
    .tab:hover { background: var(--bg-hover); color: var(--text); }
    .tab.on { background: var(--bg); color: #fff; border-top: 2px solid var(--focus); }

    /* Barre d'espaces + puces récents */
    .spaces { padding-top: .6rem; padding-bottom: .6rem; background: var(--bg-elev);
              border-bottom: 1px solid #1b1b1b; }
    .spacebar { display: flex; gap: .5rem; align-items: center; }
    .spacename { color: #4fc1ff; white-space: nowrap; font-weight: 600; }
    .chips { display: flex; flex-wrap: wrap; gap: .4rem; align-items: center; margin-top: .5rem; }
    .chip { display: inline-flex; align-items: center; background: var(--bg-input);
            border: 1px solid var(--border); border-radius: 12px; overflow: hidden; }
    .chiplabel { color: var(--text); padding: .2rem .6rem; }
    .chiplabel:hover { color: #fff; background: var(--bg-hover); }
    .chipx { color: var(--text-mute); padding: .2rem .5rem; }
    .chipx:hover { color: var(--del); }
    /* Barre d'onglets (sessions) */
    .sessiontabs { display: flex; flex-wrap: wrap; gap: .3rem; padding: .3rem 1rem 0; }
    .stab { display: inline-flex; align-items: center; background: var(--bg-elev);
            border: 1px solid var(--border); border-bottom: none;
            border-radius: 8px 8px 0 0; overflow: hidden; }
    .stab.on { background: var(--bg); border-top: 2px solid var(--focus); }
    .stablabel { color: var(--text-mute); padding: .25rem .7rem; }
    .stab.on .stablabel { color: #fff; }
    .stablabel:hover { color: #fff; background: var(--bg-hover); }
    .stabx { color: var(--text-mute); padding: .25rem .5rem; }
    .stabx:hover { color: var(--del); }
    .newspace { display: flex; flex-direction: column; gap: .5rem; background: var(--bg-elev);
                border: 1px solid var(--border); border-radius: var(--radius); padding: .8rem; margin-top: .5rem; }
    .types { display: flex; gap: .4rem; flex-wrap: wrap; }
    .browser { margin-top: .5rem; background: var(--bg-elev); border: 1px solid var(--border);
               border-radius: var(--radius); padding: .5rem; }
    .browsebar { display: flex; gap: .6rem; align-items: center; margin-bottom: .4rem; }
    .browser .list { max-height: 320px; overflow: auto; }

    /* Colonnes / listes (sidebar + détail) */
    .cols { display: flex; gap: 1rem; align-items: flex-start; padding-top: .8rem; }
    .list { list-style: none; padding-left: 0; margin: 0; min-width: 240px; }
    .list li { border-radius: 3px; }
    .row { color: var(--text); text-align: left; padding: .3rem .5rem; width: 100%; border-radius: 3px; }
    .row:hover { background: var(--bg-hover); color: #fff; }
    .row.on { background: var(--bg-sel); color: #fff; }
    .detail { flex: 1; }

    /* Encart squad (statuts d'agents) */
    .squad { display: flex; flex-wrap: wrap; gap: .4rem; align-items: center; margin: .6rem 0; }
    .agentchip { background: var(--bg-input); border: 1px solid var(--border); border-radius: 12px;
                 padding: .2rem .6rem; font-size: .8rem; color: var(--text-dim); }
    .agentchip.thinking { color: #dcdcaa; border-color: #6b6b3a; }
    .agentchip.working { color: var(--add); border-color: #3a6b4a; }
    .agentchip.done { color: var(--text-mute); }

    /* Plan */
    .plan { list-style: none; padding-left: 0; }
    .plan li { padding: .35rem .6rem; border-left: 2px solid var(--border-strong);
               background: var(--bg-elev); margin: .3rem 0; border-radius: 0 3px 3px 0; }
    .planhead { font-weight: 600; }
    .planobj { color: var(--text-dim); font-size: .88rem; }
    .plandeps { color: var(--text-mute); font-size: .8rem; }
    .planbox { border: 1px solid var(--border); border-radius: var(--radius); padding: .6rem .7rem;
               background: var(--bg-elev); }

    /* Zones de lecture : radar, viewer, diff */
    .radar, .viewer { background: var(--bg); color: var(--text); padding: .6rem .8rem;
                      border: 1px solid var(--border); border-radius: var(--radius);
                      max-height: 70vh; overflow: auto; }
    .radar { white-space: pre-wrap; font-family: var(--mono); font-size: .82rem; max-height: 360px; }
    .viewer { flex: 1; }
    .viewerpane { flex: 1; display: flex; flex-direction: column; gap: .4rem; }
    .docactions { display: flex; gap: .5rem; }
    .goal { width: 100%; min-height: 60px; }

    /* Diff (vue Modifications) */
    .diff { font-family: var(--mono); font-size: .82rem; background: var(--bg);
            border: 1px solid var(--border); border-radius: var(--radius); padding: .5rem; overflow: auto; }
    .dl { white-space: pre-wrap; padding: 0 .3rem; }
    .dl.add { color: var(--add); background: rgba(137,209,133,.08); }
    .dl.del { color: var(--del); background: rgba(244,135,113,.08); }
    .dl.ctx { color: var(--text-mute); }

    /* Rendu Markdown */
    .markdown { line-height: 1.6; }
    .markdown h1, .markdown h2, .markdown h3, .markdown h4 { color: #e7e7e7; line-height: 1.25;
        margin: 1rem 0 .5rem; text-transform: none; letter-spacing: 0; }
    .markdown h1 { font-size: 1.5rem; border-bottom: 1px solid var(--border); padding-bottom: .3rem; }
    .markdown h2 { font-size: 1.25rem; border-bottom: 1px solid var(--border); padding-bottom: .25rem; }
    .markdown h3 { font-size: 1.08rem; }
    .markdown p { margin: .5rem 0; }
    .markdown ul, .markdown ol { padding-left: 1.4rem; margin: .4rem 0; }
    .markdown a { color: #4fc1ff; }
    .markdown code { background: #2d2d2d; padding: .1rem .35rem; border-radius: 3px; font-family: var(--mono); font-size: .88em; }
    .markdown pre { background: #1b1b1b; padding: .7rem; border-radius: var(--radius); overflow: auto; border: 1px solid var(--border); }
    .markdown pre code { background: none; padding: 0; }
    .markdown blockquote { border-left: 3px solid var(--border-strong); margin: .5rem 0; padding: .1rem .8rem; color: var(--text-dim); }
    .markdown table { border-collapse: collapse; margin: .6rem 0; }
    .markdown th, .markdown td { border: 1px solid var(--border); padding: .3rem .6rem; }
    .markdown th { background: var(--bg-elev); }
    .markdown hr { border: none; border-top: 1px solid var(--border); margin: 1rem 0; }
    .markdown .mermaid { background: #fff; padding: .8rem; border-radius: var(--radius); text-align: center; }

    /* Espace de travail : conversation (gauche) + Modifications en direct (droite) */
    /* Colonne de conversation dans le panneau droit : remplit la hauteur, le fil défile. */
    .chatcol { flex: 1; min-height: 0; display: flex; }

    /* Chat */
    .chat { display: flex; flex-direction: column; gap: .6rem; flex: 1; min-height: 0; }
    .messages { display: flex; flex-direction: column; gap: .5rem; flex: 1; min-height: 120px;
                overflow: auto; padding: .6rem; background: var(--bg); border: 1px solid var(--border);
                border-radius: var(--radius); }
    .bubble { max-width: 80%; padding: .5rem .7rem; border-radius: 8px; }
    .bubble .who { display: block; font-size: .72rem; color: #4fc1ff; margin-bottom: .15rem; }
    .bubble .text { white-space: pre-wrap; }
    .bubble.user { align-self: flex-end; background: #094771; }
    .bubble.coord { align-self: flex-start; background: var(--bg-elev); border-left: 2px solid var(--focus); }
    .bubble.agent { align-self: flex-start; background: #2d2d2d; max-width: 90%; }
    .bubble.agent .text { margin-top: .35rem; color: var(--text-dim); border-top: 1px dashed var(--border); padding-top: .35rem; }
    .bubble.system { align-self: center; background: transparent; color: var(--text-mute); font-size: .8rem; }
    .composer { display: flex; gap: .5rem; align-items: flex-end; }
    .chatinput { flex: 1; resize: none; min-height: 2.4rem; max-height: 7rem; line-height: 1.35; }

    /* Menu Agents & skills */
    .agentsmenu .toolbar { display: flex; gap: .5rem; margin-bottom: .3rem; }
    .rolerow { margin: .2rem 0; }
    .skillrow { display: flex; align-items: center; gap: .2rem; }
    .ficheeditor { margin-top: .6rem; border-top: 1px solid var(--border); padding-top: .5rem; }
    .fichearea { width: 100%; min-height: 220px; resize: vertical; font-family: var(--mono); }

    /* Scrollbars fines façon VS Code */
    ::-webkit-scrollbar { width: 10px; height: 10px; }
    ::-webkit-scrollbar-thumb { background: #424242; border-radius: 5px; }
    ::-webkit-scrollbar-thumb:hover { background: #4f4f4f; }
    ::-webkit-scrollbar-track { background: transparent; }
"#;

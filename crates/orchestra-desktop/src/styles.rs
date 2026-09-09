//! Feuille de style de la fenêtre.
//!
//! **Thème sombre à dominante VERTE par défaut** (comme le template), **thème clair** en option
//! (classe `.app.light`). Tout passe par des variables CSS pour un basculement instantané.
//! Layout façon Cursor : barre supérieure (onglets), shell 5 zones
//! (checkpoints · explorateur · conversation · visualiseur · rail Tâches) et barre de statut.

pub const CSS: &str = r#"
    /* ---- Thème SOMBRE + VERT (défaut) ---- */
    :root {
        --bg: #0f120f;
        --bg-elev: #161a16;
        --bg-titlebar: #131713;
        --bg-input: #1a1f1a;
        --bg-hover: #1f261f;
        --bg-sel: rgba(63,185,80,.16);
        --border: #242a24;
        --border-strong: #313a31;
        --text: #cdd6cd;
        --text-dim: #8f9d8f;
        --text-mute: #667466;
        --accent: #3fb950;
        --accent-hover: #4cc65d;
        --focus: #3fb950;
        --link: #79d894;
        --add: #3fb950;
        --add-bg: rgba(63,185,80,.14);
        --del: #f0776a;
        --del-bg: rgba(240,119,106,.12);
        --amber: #d2a04a;
        --amber-bg: rgba(210,160,74,.16);
        --heading: #e7efe7;
        --code-bg: #121611;
        --bubble-user: #212a21;
        --bubble-user-text: #dff5e2;
        --bubble-agent: #191e19;
        --shadow: 0 1px 2px rgba(0,0,0,.35);
        --radius: 8px;
        --font: "Segoe UI", system-ui, -apple-system, sans-serif;
        --mono: "Cascadia Code", "Consolas", ui-monospace, Menlo, monospace;
    }
    /* ---- Thème CLAIR (option) ---- */
    .app.light {
        --bg: #ffffff;
        --bg-elev: #f4f7f4;
        --bg-titlebar: #eef2ee;
        --bg-input: #ffffff;
        --bg-hover: #eaf0ea;
        --bg-sel: rgba(46,160,67,.14);
        --border: #e2e8e2;
        --border-strong: #cfd8cf;
        --text: #1f2a1f;
        --text-dim: #556355;
        --text-mute: #7d8a7d;
        --accent: #2ea043;
        --accent-hover: #2c9440;
        --focus: #2ea043;
        --link: #1a7f37;
        --add: #2ea043;
        --add-bg: rgba(46,160,67,.12);
        --del: #cf222e;
        --del-bg: rgba(207,34,46,.10);
        --amber: #9a6700;
        --amber-bg: rgba(154,103,0,.12);
        --heading: #142414;
        --code-bg: #f1f4f1;
        --bubble-user: #dcf3e1;
        --bubble-user-text: #0f3d1c;
        --bubble-agent: #f1f4f1;
        --shadow: 0 1px 2px rgba(0,0,0,.08);
    }

    * { box-sizing: border-box; }
    body { margin: 0; background: var(--bg); color: var(--text); }

    /* Coquille plein écran : barre de statut fixe (26px) réservée en bas. */
    .app { font-family: var(--font); font-size: 13px; height: calc(100vh - 26px);
           display: flex; flex-direction: column; overflow: hidden;
           background: var(--bg); color: var(--text); }

    /* Transitions douces (smooth) sur les éléments interactifs. */
    button, .stab, .trow, .cpdot, .chip, .memcard, .icontoggle, .panebtn, .stubbtn {
        transition: background .13s ease, color .13s ease, border-color .13s ease, transform .1s ease;
    }
    .messages, .centerbody, .taskrail, .termbody, .explorer, .pane { scroll-behavior: smooth; }

    /* ---- Barre supérieure ---- */
    .topbar { display: flex; align-items: center; gap: .55rem; background: var(--bg-titlebar);
              border-bottom: 1px solid var(--border); padding: 0 .9rem; min-height: 44px; }
    .logo { width: 16px; height: 16px; border-radius: 5px; flex: none;
            background: linear-gradient(135deg, #56d364, var(--accent)); box-shadow: 0 0 8px rgba(63,185,80,.35); }
    .brand { font-weight: 700; white-space: nowrap; letter-spacing: .2px; margin-right: .5rem; color: var(--heading); }
    .topspacer { flex: 1; }
    .tabadd { background: none; border: none; color: var(--text-dim); font-size: 1.15rem;
              cursor: pointer; padding: .1rem .55rem; border-radius: var(--radius); line-height: 1; }
    .tabadd:hover { background: var(--bg-hover); color: var(--accent); }
    .icontoggle { background: none; border: 1px solid transparent; color: var(--text-dim);
                  cursor: pointer; padding: .25rem .55rem; border-radius: var(--radius); font-size: 1rem; }
    .icontoggle:hover { background: var(--bg-hover); color: var(--text); }

    /* Onglets de session (browser-like) — pastille verte sur l'actif */
    .sessiontabs { display: flex; flex-wrap: wrap; gap: .2rem; align-items: center; }
    .stab { display: inline-flex; align-items: center; background: transparent;
            border: 1px solid transparent; border-radius: var(--radius); overflow: hidden; }
    .stab.on { background: var(--bg); border-color: var(--border); box-shadow: var(--shadow); }
    .stablabel { background: none; border: none; color: var(--text-dim); padding: .3rem .55rem;
                 cursor: pointer; font: inherit; display: inline-flex; align-items: center; }
    .stablabel::before { content: ""; display: inline-block; width: 9px; height: 9px; border-radius: 2px;
                         background: var(--text-mute); margin-right: .45rem; }
    .stab.on .stablabel { color: var(--text); }
    .stab.on .stablabel::before { background: var(--accent); }
    .stablabel:hover { color: var(--text); }
    .stabx { background: none; border: none; color: var(--text-mute); padding: .3rem .4rem; cursor: pointer; }
    .stabx:hover { color: var(--del); }

    /* ---- Shell 5 zones ---- */
    .ide { flex: 1; min-height: 0; display: flex; }
    .pane { min-height: 0; }
    .pane.explorerpane { width: 240px; flex: none; border-right: 1px solid var(--border);
                         background: var(--bg-elev); display: flex; flex-direction: column; overflow: hidden; }
    .pane.conversation { flex: 1.4; min-width: 340px; display: flex; flex-direction: column;
                         overflow: hidden; background: var(--bg); }
    .pane.viewer { flex: 1; min-width: 300px; display: flex; flex-direction: column;
                   overflow: hidden; border-left: 1px solid var(--border); }

    /* Bandeau de panneau repliable : titre + chevron de repli dans le coin */
    .panebar { display: flex; align-items: center; gap: .4rem; padding: .35rem .5rem .35rem .7rem;
               border-bottom: 1px solid var(--border); background: var(--bg-elev); }
    .panetitle { font-size: .7rem; letter-spacing: .5px; color: var(--text-mute); text-transform: uppercase; }
    .panebtn { background: none; border: none; color: var(--text-mute); cursor: pointer; font-size: 1.05rem;
               padding: 0 .35rem; border-radius: 6px; line-height: 1; }
    .panebtn:hover { background: var(--bg-hover); color: var(--accent); }
    /* Stub d'un panneau replié : fine bande cliquable pour le rouvrir */
    .stub { width: 30px; flex: none; background: var(--bg-elev); border-right: 1px solid var(--border);
            border-left: 1px solid var(--border); display: flex; justify-content: center; padding-top: .5rem; }
    .stubbtn { background: none; border: none; color: var(--text-mute); cursor: pointer; font-size: 1rem;
               padding: .3rem; border-radius: 6px; height: fit-content; }
    .stubbtn:hover { background: var(--bg-hover); color: var(--accent); }

    /* Rail Checkpoints (fin) */
    .checkpoints { width: 46px; flex: none; background: var(--bg-elev); border-right: 1px solid var(--border);
                   display: flex; flex-direction: column; align-items: center; padding: .6rem 0; }
    .cplabel { writing-mode: vertical-rl; transform: rotate(180deg); color: var(--text-mute);
               font-size: .6rem; letter-spacing: 2px; margin-bottom: .8rem; user-select: none; }
    .cpdots { display: flex; flex-direction: column; gap: .7rem; align-items: center; }
    .cpdot { width: 11px; height: 11px; border-radius: 50%; border: 2px solid var(--border-strong);
             background: transparent; cursor: pointer; padding: 0; }
    .cpdot:hover { border-color: var(--accent); transform: scale(1.15); }
    .cpdot.on { background: var(--add); border-color: var(--add); box-shadow: 0 0 6px rgba(63,185,80,.5); }

    /* ---- Conversation ---- */
    .convhead { display: flex; align-items: center; gap: .5rem; padding: .55rem .8rem;
                border-bottom: 1px solid var(--border); background: var(--bg-elev); }
    .agentname { font-weight: 600; color: var(--heading); }
    .statuspill { font-size: .7rem; padding: .12rem .55rem; border-radius: 10px;
                  background: var(--add-bg); color: var(--add); border: 1px solid var(--add); }
    .convspacer { flex: 1; }
    .squad { display: flex; flex-wrap: wrap; gap: .35rem; align-items: center; padding: .45rem .8rem 0; }
    .squad .muted { font-size: .72rem; }
    .agentchip { background: var(--bg-input); border: 1px solid var(--border); border-radius: 12px;
                 padding: .12rem .55rem; font-size: .73rem; color: var(--text-dim); }
    .agentchip.thinking { color: var(--amber); border-color: var(--amber); }
    .agentchip.working { color: var(--add); border-color: var(--add); }
    .agentchip.done { color: var(--text-mute); }
    .hint { color: var(--text-mute); font-size: .85rem; padding: 1.4rem 1.2rem; }
    .chatcol { flex: 1; min-height: 0; display: flex; padding: .2rem 1.1rem .9rem; }

    /* Chat — aéré et épuré (façon template) : l'agent écrit en texte plein, l'utilisateur
       dans une bulle arrondie discrète, pas de labels ni d'icônes. */
    .chat { display: flex; flex-direction: column; gap: .8rem; flex: 1; min-height: 0; }
    .messages { display: flex; flex-direction: column; gap: 1.3rem; flex: 1; min-height: 120px;
                overflow: auto; padding: 1.1rem .4rem; }
    .bubble .text { white-space: pre-wrap; }
    /* Orchestrateur : texte plein, large, aéré — aucune bulle, aucun label. */
    .bubble.coord { align-self: stretch; max-width: 100%; background: transparent; border: none; padding: 0; }
    .bubble.coord .who { display: none; }
    .bubble.coord .text { line-height: 1.75; color: var(--text); }
    /* Utilisateur : bulle arrondie discrète, alignée à droite. */
    .bubble.user { align-self: flex-end; max-width: 78%; background: var(--bubble-user);
                   color: var(--bubble-user-text); padding: .6rem .95rem; border-radius: 16px;
                   border-bottom-right-radius: 6px; }
    .bubble.user .who { display: none; }
    /* Sous-agent : repli discret « pilule » façon “Thought for…”. */
    .bubble.agent { align-self: stretch; max-width: 100%; background: transparent; border: none; padding: 0; }
    .bubble.agent .who { display: none; }
    .disclosure { background: var(--bg-elev); border: 1px solid var(--border); cursor: pointer;
                  color: var(--text-dim); font: inherit; padding: .2rem .7rem; border-radius: 999px; font-size: .8rem; }
    .disclosure:hover { border-color: var(--accent); color: var(--text); }
    .bubble.agent .text { margin-top: .5rem; padding: .6rem .8rem; background: var(--bg-elev);
                          border-radius: 10px; color: var(--text-dim); line-height: 1.6; }
    .bubble.system { align-self: center; background: transparent; color: var(--text-mute); font-size: .78rem; }

    .composer { display: flex; gap: .5rem; align-items: flex-end; border: 1px solid var(--border);
                border-radius: 16px; padding: .55rem .6rem; background: var(--bg-elev); }
    .composer:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--add-bg); }
    .chatinput { flex: 1; resize: none; min-height: 2.4rem; max-height: 8rem; line-height: 1.45;
                 border: none; background: transparent; padding: .25rem .4rem; }
    .chatinput:focus { border: none; }
    button.send { background: var(--accent); color: #08240d; border: none; border-radius: 50%;
                  width: 36px; height: 36px; cursor: pointer; font-size: 1.1rem; font-weight: 700; flex: none; }
    button.send:hover { background: var(--accent-hover); transform: translateY(-1px); }

    /* ---- Explorateur (« orchestre en verre ») ---- */
    .explorer { padding: .3rem 0; overflow: auto; flex: 1; }
    .tree { list-style: none; margin: 0; padding: 0; }
    .tree li { margin: 0; }
    .trow { display: flex; align-items: center; justify-content: space-between; gap: .4rem;
            padding: .12rem .5rem; border-left: 2px solid transparent; }
    .trow .tname { background: none; border: none; color: var(--text); padding: .05rem 0; text-align: left;
                   cursor: default; font: inherit; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
                   display: inline-flex; align-items: center; }
    /* Marqueur typographique discret (pas d'emoji) : carré pour un dossier, point pour un fichier. */
    .trow .tname::before { content: ""; display: inline-block; width: 6px; height: 6px; margin-right: .5rem;
                           flex: none; background: var(--text-mute); opacity: .5; }
    .trow.dir .tname { color: var(--text-dim); font-weight: 600; }
    .trow.dir .tname::before { border-radius: 2px; opacity: .7; }
    .trow.file .tname { cursor: pointer; }
    .trow.file .tname::before { border-radius: 50%; }
    .trow.file:hover { background: var(--bg-hover); }
    .trow.on { background: var(--bg-sel); }
    .trow.act-read { border-left-color: var(--link); }
    .trow.act-write { border-left-color: var(--add); }
    .actbadge { font-size: .64rem; padding: 0 .35rem; border-radius: 8px; white-space: nowrap; }
    .actbadge.read { color: var(--link); background: rgba(121,216,148,.12); }
    .actbadge.write { color: var(--add); background: var(--add-bg); }

    /* ---- Visualiseur de fichier (code + diff) ---- */
    .filepane { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg); }
    .filepane .centerhead { display: flex; align-items: center; gap: .6rem; padding: .4rem .7rem;
                            border-bottom: 1px solid var(--border); background: var(--bg-elev); }
    .filepane .centerhead .path { flex: 1; font-family: var(--mono); font-size: .8rem; color: var(--text-dim);
                                  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .centerbody { flex: 1; min-height: 0; overflow: auto; padding: .5rem .7rem; }
    .filepane .welcome { padding: 2rem; text-align: center; margin: auto; max-width: 460px; }
    .codeview { font-family: var(--mono); font-size: .82rem; white-space: pre-wrap; margin: 0; color: var(--text); }
    .editor { width: 100%; height: 100%; min-height: 50vh; resize: none; font-family: var(--mono);
              font-size: .82rem; line-height: 1.5; border: 1px solid var(--border); border-radius: var(--radius); }

    /* Terminal (sous le code) */
    .termpanel { flex: none; height: 190px; display: flex; flex-direction: column;
                 border-top: 1px solid var(--border); background: var(--bg-elev); }
    .termhead { padding: .3rem .7rem; font-size: .68rem; letter-spacing: .5px; color: var(--text-mute);
                text-transform: uppercase; border-bottom: 1px solid var(--border); }
    .termbody { flex: 1; overflow: auto; padding: .4rem .7rem; }
    .termrun { margin-bottom: .5rem; }
    .termcmd { font-family: var(--mono); font-size: .8rem; color: var(--accent); }
    .termout { font-family: var(--mono); font-size: .78rem; white-space: pre-wrap; margin: .2rem 0; color: var(--text-dim); }
    .termexit { font-size: .72rem; color: var(--text-mute); }

    /* ---- Rail Tâches / Mémoire / Contexte ---- */
    .taskrail { width: 300px; flex: none; border-left: 1px solid var(--border); background: var(--bg-elev);
                overflow: auto; padding: .3rem .85rem 1rem; }
    .railhead { display: flex; align-items: center; justify-content: space-between; margin: 1.1rem 0 .5rem;
                font-size: .7rem; letter-spacing: .5px; color: var(--text-mute); text-transform: uppercase; }
    .railcount { color: var(--text-mute); }
    .plan { list-style: none; padding: 0; margin: 0; }
    .planrow { display: flex; gap: .55rem; align-items: flex-start; padding: .35rem .1rem; }
    .planicon { color: var(--text-mute); width: 1rem; display: inline-block; text-align: center; }
    .planrow.done .planicon { color: var(--add); }
    .planrow.done .planobj { text-decoration: line-through; color: var(--text-mute); }
    /* Étape « en cours » : vrai spinner (anneau tournant), contenu vide. */
    .planrow.running .planicon { width: 13px; height: 13px; box-sizing: border-box;
                                 border: 2px solid var(--border-strong); border-top-color: var(--accent);
                                 border-radius: 50%; animation: spin .9s linear infinite; margin-top: 2px; }
    .planrow.failed .planicon { color: var(--del); }
    .plantext { display: flex; flex-direction: column; }
    .planobj { color: var(--text); font-size: .86rem; }
    .planagent { color: var(--text-mute); font-size: .73rem; }
    .approvecard { border: 1px solid var(--accent); border-radius: var(--radius); padding: .65rem;
                   margin: .5rem 0; display: flex; flex-direction: column; gap: .5rem; background: var(--bg);
                   box-shadow: 0 0 0 3px var(--add-bg); }
    .memcard { background: var(--bg); border: 1px solid var(--border); border-radius: var(--radius);
               padding: .45rem .6rem; margin-bottom: .35rem; font-size: .84rem; box-shadow: var(--shadow); }
    .ctxrow { font-family: var(--mono); font-size: .8rem; color: var(--text-dim); padding: .15rem 0; }
    .small { font-size: .8rem; }
    .muted { color: var(--text-mute); }

    /* ---- Docs (persona / ADR / mémoire) ---- */
    .docrow { display: block; width: 100%; text-align: left; background: none; border: none;
              color: var(--text-dim); padding: .18rem .1rem; cursor: pointer; font: inherit; font-size: .84rem;
              border-radius: var(--radius); }
    .docrow:hover { background: var(--bg-hover); color: var(--text); }

    /* ---- Git (constat, panneau Tâches) ---- */
    .gitbranch { font-family: var(--mono); font-size: .82rem; color: var(--text); margin: .2rem 0 .4rem; }
    .gitfilerow { display: flex; justify-content: space-between; gap: .5rem; width: 100%;
                  background: none; border: none; text-align: left; cursor: pointer; font: inherit;
                  padding: .18rem .1rem; border-radius: var(--radius); }
    .gitfilerow:hover { background: var(--bg-hover); }
    .gfpath { font-family: var(--mono); font-size: .8rem; color: var(--text-dim);
              overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

    /* ---- Docker (constat, panneau Tâches) ---- */
    .dockercard { background: var(--bg); border: 1px solid var(--border); border-radius: var(--radius);
                  padding: .45rem .6rem; margin-bottom: .35rem; }
    .dockername { font-family: var(--mono); font-size: .84rem; color: var(--text); }
    /* Pastille d'état générique (réutilisée hors barre de statut, cf. `.statusbar .dot` ci-dessous
       pour les couleurs de fond de cette dernière). */
    .dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
    .dot.ok { background: var(--add); }
    .dot.warn { background: var(--amber); }

    @keyframes spin { to { transform: rotate(360deg); } }

    /* ---- Barre d'espaces (ouvrir / créer / récents) ---- */
    .spaces { background: var(--bg-elev); border-bottom: 1px solid var(--border); padding: .6rem 1rem; }
    .spacebar { display: flex; gap: .5rem; align-items: center; flex-wrap: wrap; }
    .spacename { color: var(--accent); white-space: nowrap; font-weight: 600; }
    .chips { display: flex; flex-wrap: wrap; gap: .4rem; align-items: center; margin-top: .5rem; }
    .chip { display: inline-flex; align-items: center; background: var(--bg-input);
            border: 1px solid var(--border); border-radius: 12px; overflow: hidden; }
    .chiplabel { background: none; border: none; cursor: pointer; color: var(--text); padding: .2rem .6rem; font: inherit; }
    .chiplabel:hover { background: var(--bg-hover); color: var(--accent); }
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
    button.go { background: var(--accent); color: #08240d; border-color: var(--accent); font-weight: 600; }
    button.go:hover { background: var(--accent-hover); }
    button.ghost { background: none; border: none; color: var(--link); padding: .2rem .45rem; }
    button.ghost:hover { background: var(--bg-hover); }
    input, textarea { background: var(--bg-input); color: var(--text); border: 1px solid var(--border);
                      border-radius: var(--radius); padding: .4rem .55rem; font: inherit; outline: none; }
    input:focus, textarea:focus { border-color: var(--focus); }
    input::placeholder, textarea::placeholder { color: var(--text-mute); }
    h2 { color: var(--heading); font-size: 1.05rem; margin: .3rem 0; }

    /* ---- Barre de statut ---- */
    .statusbar { position: fixed; left: 0; right: 0; bottom: 0; height: 26px;
                 background: var(--accent); color: #06210c; display: flex; align-items: center;
                 gap: 1rem; padding: 0 .8rem; font-size: 12px; font-weight: 600; }
    .statusbar .sb-item { display: inline-flex; align-items: center; gap: .4rem; }
    .statusbar .dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
    .statusbar .dot.ok { background: #06210c; box-shadow: 0 0 0 2px rgba(6,33,12,.25); }
    .statusbar .dot.warn { background: #7a3b00; }

    /* ---- Diff ---- */
    .diff { font-family: var(--mono); font-size: .82rem; }
    .dl { white-space: pre-wrap; padding: 0 .3rem; }
    .dl.add { color: var(--add); background: var(--add-bg); }
    .dl.del { color: var(--del); background: var(--del-bg); }
    .dl.ctx { color: var(--text-mute); }
    /* Lignes d'un vrai `git diff` (panneau Git) : en-têtes de hunk et méta (index/---/+++). */
    .dl.hunk { color: var(--link); background: var(--bg-elev); }
    .dl.meta { color: var(--text-mute); }

    /* ---- Rendu Markdown ---- */
    .markdown { line-height: 1.6; }
    .markdown h1, .markdown h2, .markdown h3, .markdown h4 { color: var(--heading); line-height: 1.25; margin: 1rem 0 .5rem; }
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
    .markdown blockquote { border-left: 3px solid var(--accent); margin: .5rem 0; padding: .1rem .8rem; color: var(--text-dim); }
    .markdown table { border-collapse: collapse; margin: .6rem 0; }
    .markdown th, .markdown td { border: 1px solid var(--border); padding: .3rem .6rem; }
    .markdown th { background: var(--bg-elev); }
    .markdown hr { border: none; border-top: 1px solid var(--border); margin: 1rem 0; }
    .markdown .mermaid { background: #fff; padding: .8rem; border-radius: var(--radius); text-align: center; }

    /* ---- Scrollbars fines ---- */
    ::-webkit-scrollbar { width: 10px; height: 10px; }
    ::-webkit-scrollbar-thumb { background: var(--border-strong); border-radius: 5px; }
    ::-webkit-scrollbar-thumb:hover { background: var(--accent); }
    ::-webkit-scrollbar-track { background: transparent; }
"#;

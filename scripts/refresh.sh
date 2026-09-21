#!/usr/bin/env bash
# Réinstalle Orchestra depuis ce dépôt, redémarre son daemon et rouvre le
# tableau de bord.
#
# Les deux binaires sont installés ensemble : `orchestra-hook` est le garde-fou
# que Claude Code exécute avant chaque appel d'outil, et il est cherché à côté
# de `orchestra`. En laisser un des deux en arrière, c'est faire tourner des
# agents avec les règles de la version précédente.
#
# Ce script ne touche NI à la base (tes tickets, ton historique de coût), NI à
# ta configuration, NI aux worktrees. « Effacer la session » veut dire arrêter
# le daemon et nettoyer sa socket, rien de plus.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET="${ORCHESTRA_SOCK:-${XDG_RUNTIME_DIR:-/tmp}/orchestra.sock}"
STATE_DIR="${ORCHESTRA_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/orchestra}"
BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"

ASSUME_YES=0
START_TUI=1
FORCE_ROLES=0
DRY_RUN=0
PROFILE_ARGS=()

usage() {
    cat <<'EOF'
usage: scripts/refresh.sh [options]

    -y, --yes        N'attend aucune confirmation, même si des agents tournent.
        --no-tui     S'arrête une fois le daemon reparti, sans ouvrir l'écran.
        --force-roles
                     Réécrit les rôles livrés dans ~/.config/orchestra/roles,
                     y compris ceux que tu as modifiés. Sans cette option, les
                     rôles manquants sont ajoutés et les tiens sont gardés.
        --debug      Installe le binaire de debug : compilation bien plus
                     rapide, exécution plus lente. Pratique pour un aller-retour.
    -n, --dry-run    Montre ce qui serait fait, sans rien faire.
    -h, --help       Ceci.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -y | --yes) ASSUME_YES=1 ;;
        --no-tui) START_TUI=0 ;;
        --force-roles) FORCE_ROLES=1 ;;
        --debug) PROFILE_ARGS=(--debug) ;;
        -n | --dry-run) DRY_RUN=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "option inconnue : $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
note() { printf '    %s\n' "$*"; }

# Toute commande qui change quelque chose passe par là, pour que --dry-run soit
# une vraie garantie et pas une intention.
run() {
    if [[ $DRY_RUN -eq 1 ]]; then
        printf '    [dry-run] %s\n' "$(printf '%q ' "$@")"
        return 0
    fi
    "$@"
}

# --- 1. Ce qui tourne encore ------------------------------------------------

say "État actuel"
DAEMON_PIDS="$(pgrep -f 'orchestra daemon' || true)"
if orchestra ping >/dev/null 2>&1; then
    STATUS="$(orchestra status 2>/dev/null || true)"
    AGENTS="$(awk '/agents actifs/ { print $3 }' <<<"$STATUS")"
    TICKETS="$(awk '/tickets actifs/ { print $3 }' <<<"$STATUS")"
    AGENTS="${AGENTS:-0}"
    note "daemon en ligne : ${TICKETS:-0} ticket(s) actif(s), $AGENTS agent(s) en cours"
    if [[ "$AGENTS" -gt 0 ]]; then
        orchestra ticket list 2>/dev/null | awk '/en cours/ { print "    " $0 }' || true
    fi
    if [[ "$AGENTS" -gt 0 && $ASSUME_YES -eq 0 && $DRY_RUN -eq 0 ]]; then
        # Le tour en cours d'un agent tué est perdu — sa branche garde ce qu'il
        # avait déjà commité, et le ticket se relance. À toi de voir si ça vaut
        # le coup d'attendre qu'il ait fini.
        printf '\n    Les arrêter maintenant ? Le tour en cours sera perdu. [o/N] '
        read -r answer
        case "$answer" in
            o | O | y | Y) ;;
            *)
                echo "    annulé — rien n'a été touché."
                exit 1
                ;;
        esac
    fi
else
    note "aucun daemon ne répond sur $SOCKET"
fi

# --- 2. Compilation et installation ----------------------------------------

say "Installation depuis $REPO"
run cargo install --path "$REPO/crates/orchestra" --force --locked "${PROFILE_ARGS[@]}"
run cargo install --path "$REPO/crates/orchestra-hook" --force --locked "${PROFILE_ARGS[@]}"
note "binaires dans $BIN_DIR"

# --- 3. Arrêt de l'ancienne session ----------------------------------------

say "Arrêt du daemon"
if [[ -n "$DAEMON_PIDS" ]]; then
    # SIGTERM est géré : le daemon ferme ses connexions et rend la main. Le
    # SIGKILL n'est qu'un filet, pour un processus qui ne répond plus.
    run pkill -TERM -f 'orchestra daemon' || true
    for _ in $(seq 1 20); do
        pgrep -f 'orchestra daemon' >/dev/null || break
        sleep 0.25
    done
    if pgrep -f 'orchestra daemon' >/dev/null && [[ $DRY_RUN -eq 0 ]]; then
        note "il ne s'est pas arrêté de lui-même, SIGKILL"
        run pkill -KILL -f 'orchestra daemon' || true
        sleep 0.5
    fi
    note "arrêté ($(echo "$DAEMON_PIDS" | tr '\n' ' '))"
else
    note "rien à arrêter"
fi

# La socket d'un daemon tué brutalement reste sur le disque et fait croire à un
# service qui écoute. Le nouveau daemon l'effacerait lui-même, mais autant
# partir propre.
if [[ -S "$SOCKET" ]] && ! pgrep -f 'orchestra daemon' >/dev/null; then
    run rm -f "$SOCKET"
    note "socket nettoyée"
fi

# --- 4. Catalogue de rôles --------------------------------------------------

say "Catalogue de rôles"
if [[ $FORCE_ROLES -eq 1 ]]; then
    note "réécriture complète (tes modifications seront perdues)"
    run orchestra init --force
else
    # Sans --force, `init` ajoute ce qui manque et ne touche à rien d'autre :
    # c'est ce qui installe un nouveau rôle livré sans écraser les tiens.
    run orchestra init
fi

# --- 5. Redémarrage ---------------------------------------------------------

say "Redémarrage du daemon"
if [[ $DRY_RUN -eq 1 ]]; then
    printf '    [dry-run] %s\n' "orchestra daemon --quiet & (détaché)"
else
    mkdir -p "$STATE_DIR"
    nohup orchestra daemon --quiet >/dev/null 2>&1 &
    disown || true
    for _ in $(seq 1 40); do
        orchestra ping >/dev/null 2>&1 && break
        sleep 0.25
    done
    if ! orchestra ping >/dev/null 2>&1; then
        echo "    le daemon n'a pas démarré — regarde $STATE_DIR/daemon.log*" >&2
        exit 1
    fi
    note "$(orchestra ping)"
    note "journal : $STATE_DIR/daemon.log*"
fi

# --- 6. Le tableau de bord --------------------------------------------------

if [[ $START_TUI -eq 0 ]]; then
    say "Prêt"
    note "ouvre-le avec : orchestra"
    exit 0
fi

say "Ouverture du tableau de bord"
if [[ $DRY_RUN -eq 1 ]]; then
    printf '    [dry-run] %s\n' "exec orchestra"
    exit 0
fi
exec orchestra

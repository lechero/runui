# Bash 4+ / Readline: source this file, then bind a key if desired:
# bind -x '"\C-x\C-h":_runui_history_widget'
_runui_history_widget() {
    (( BASH_VERSINFO[0] >= 4 )) || return 2
    local selected temporary picker_status=1
    [[ -n ${HISTFILE-} ]] || return 1
    temporary=$(mktemp -d "${TMPDIR:-/tmp}/runui-pick.XXXXXXXX") || return 1
    (umask 077; : > "$temporary/selection") || { command rmdir -- "$temporary"; return 1; }
    if command runui find "$@" --history-file "$HISTFILE" --mode shell --shell bash --pick --print0 < /dev/tty > "$temporary/selection"; then
        if IFS= read -r -d '' selected < "$temporary/selection"; then
            READLINE_LINE=$selected
            # Readline's insertion point is a byte offset in its line buffer.
            local LC_ALL=C
            READLINE_POINT=${#READLINE_LINE}
            picker_status=0
        fi
    fi
    command rm -f -- "$temporary/selection"
    command rmdir -- "$temporary"
    return "$picker_status"
}

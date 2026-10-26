# Source this file, then bind a key if desired:
# bindkey '^X^H' runui-history-widget
runui-history-widget() {
    local selected temporary picker_status=1
    [[ -n ${HISTFILE-} ]] || return 1
    temporary=$(mktemp -d "${TMPDIR:-/tmp}/runui-pick.XXXXXXXX") || return 1
    {
        (umask 077; : > "$temporary/selection") || return 1
        zle -I
        if command runui find "$@" --history-file "$HISTFILE" --mode shell --shell zsh --pick --print0 < /dev/tty > "$temporary/selection"; then
            if IFS= read -r -d '' selected < "$temporary/selection"; then
                BUFFER=$selected
                CURSOR=${#BUFFER}
                picker_status=0
                zle redisplay
            fi
        fi
    } always {
        command rm -f -- "$temporary/selection"
        command rmdir -- "$temporary"
    }
    return $picker_status
}
zle -N runui-history-widget

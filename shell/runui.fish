# Source this file to add runui-history. Bind a key separately if desired;
# existing Ctrl-R/fzf behavior is left in the user's control.
function runui-history --description 'Insert a private fish history selection for editing'
    set -l session fish
    if set -q fish_history
        set session $fish_history
    end
    set -l selected (command env fish_history="$session" runui find $argv --mode shell --shell fish --pick --print0 | string split0)
    if test (count $selected) -ne 1
        return 1
    end
    commandline --replace -- $selected[1]
    commandline --function repaint
end

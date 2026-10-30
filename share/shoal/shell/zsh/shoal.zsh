# Sourceable from .zshrc or Shoal's startup shim. This guard is deliberately not
# exported: a nested interactive shell must install its own hooks.
[[ -o interactive && ${SHOAL_SHELL_INTEGRATION:-auto} != off ]] || return 0
(( ${+__shoal_loaded} )) && return 0
builtin typeset -g __shoal_loaded=1
builtin typeset +x __shoal_loaded

builtin zmodload zsh/datetime
builtin zmodload zsh/parameter
builtin typeset -gF __shoal_command_started=0
builtin typeset -gi __shoal_command_active=0
builtin typeset -g __shoal_prompt_start='' __shoal_prompt_end=''
builtin typeset -gi __shoal_prompted=0

# Do not change the user's prompt expansion options in shell mode. The native
# renderer escapes literals for PROMPT_SUBST, so native mode requires it.
if [[ ${SHOAL_PROMPT_MODE:-native} == native ]]; then
    builtin setopt promptsubst
fi

__shoal_preexec() {
    emulate -L zsh
    __shoal_command_started=$EPOCHREALTIME
    __shoal_command_active=1
    builtin printf '\033]133;C\033\\'
    return 0
}

__shoal_directory() {
    emulate -L zsh
    local LC_ALL=C
    local directory=$PWD encoded='' byte hex
    local -i index
    # Iterate bytes, not Unicode code points: OSC 7 carries a percent-encoded
    # UTF-8 URI and must never contain a raw escape, percent, space or newline.
    for (( index = 1; index <= ${#directory}; index++ )); do
        byte=${directory[index]}
        case $byte in
            [a-zA-Z0-9/._~-]) encoded+=$byte ;;
            *) builtin printf -v hex '%%%02X' "'$byte"
               encoded+=$hex ;;
        esac
    done
    # HOST is normally a hostname, but do not allow a user-set value to inject
    # terminal controls into the URI authority.
    builtin printf '\033]7;file://%s%s\033\\' "${HOST//[^a-zA-Z0-9.-]/}" "$encoded"
}

__shoal_update_prompt() {
    emulate -L zsh
    local -i command_status=$1 duration_ms=$2
    local percent_enabled=$3 prompt_text lead=''
    local -i job_count=${#jobstates}

    if [[ ${SHOAL_PROMPT_MODE:-native} == native ]]; then
        prompt_text=$("${SHOAL_BIN:-shoal}" prompt --shell zsh \
            --status "$command_status" --duration-ms "$duration_ms" \
            --jobs "$job_count" --cols "${COLUMNS:-80}")
        # A blank line between a command's output and every native prompt
        # after the first, outside the prompt's marks.
        if (( __shoal_prompted )); then
            lead=$'\n'
        fi
        PROMPT=$lead$'%{\e]133;A\e\\%}'"$prompt_text"$'%{\e]133;B\e\\%}'
    else
        # Remove only our previous decorations. A user's precmd can replace or
        # modify PROMPT, and repeated prompts must not grow more OSC markers.
        if [[ -n $__shoal_prompt_start ]]; then
            PROMPT=${PROMPT//"$__shoal_prompt_start"/}
            PROMPT=${PROMPT//"$__shoal_prompt_end"/}
        fi
        if [[ $percent_enabled == on ]]; then
            __shoal_prompt_start=$'%{\e]133;A\e\\%}'
            __shoal_prompt_end=$'%{\e]133;B\e\\%}'
        else
            # With PROMPT_PERCENT off zsh cannot interpret %{...%}. Raw OSCs
            # preserve the visible prompt and expansion options in that mode.
            __shoal_prompt_start=$'\e]133;A\e\\'
            __shoal_prompt_end=$'\e]133;B\e\\'
        fi
        PROMPT="$__shoal_prompt_start$PROMPT$__shoal_prompt_end"
    fi
    __shoal_prompted=1
}

__shoal_precmd() {
    # Zsh supplies every hook the same initial $? even after earlier user
    # hooks. Capture it before running any commands of our own.
    local -i command_status=$?
    local percent_enabled=${options[promptpercent]}
    local -i duration_ms=0
    if (( __shoal_command_active )); then
        (( duration_ms = (EPOCHREALTIME - __shoal_command_started) * 1000 ))
        (( duration_ms < 0 )) && duration_ms=0
        builtin printf '\033]133;D;%d\033\\' "$command_status"
        __shoal_command_active=0
    fi
    __shoal_directory
    if [[ ${SHOAL_PROMPT_MODE:-native} == native ]]; then
        builtin setopt promptsubst
    fi
    __shoal_update_prompt "$command_status" "$duration_ms" "$percent_enabled"
    return 0
}

# Native zsh hook arrays preserve the existing precmd/preexec functions and
# registered hooks, execute them first, and preserve their incoming status.
builtin typeset -ga precmd_functions preexec_functions
precmd_functions+=(__shoal_precmd)
preexec_functions+=(__shoal_preexec)

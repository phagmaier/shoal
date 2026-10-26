# This file is sourced after the interactive shell's own startup files.
case $- in
    *i*) ;;
    *) return 0 ;;
esac
[[ ${SHOAL_SHELL_INTEGRATION:-auto} != off ]] || return 0
[[ ${__shoal_loaded-} != 1 ]] || return 0
__shoal_loaded=1
export -n __shoal_loaded

__shoal_status=0
__shoal_duration_ms=0
__shoal_command_start_us=0
__shoal_ps0_empty=''
__shoal_prompt_prefix='\[\e]133;A\a\]'
__shoal_prompt_suffix='\[\e]133;B\a\]'
__shoal_command_marker=$'\e]133;C\a'
__shoal_ps0_timer='${__shoal_ps0_empty:0:$((__shoal_command_start_us=10#${EPOCHREALTIME/./},0))}'
__shoal_ps0_prefix=''

# PS0 runs once per complete interactive command, unlike DEBUG (which fires
# repeatedly inside loops and functions). Its empty substring records the start
# time in the main shell, without printing the value or starting a subprocess.
# Leave DEBUG traps completely untouched. With the nondefault promptvars=off,
# Bash cannot expand this timer; marks still work and duration remains zero.

__shoal_before_prompt() {
    local status=$? now=${EPOCHREALTIME/./}
    __shoal_status=$status
    __shoal_duration_ms=0
    if (( __shoal_command_start_us > 0 && now >= __shoal_command_start_us )); then
        __shoal_duration_ms=$(( (now - __shoal_command_start_us) / 1000 ))
    fi
    __shoal_command_start_us=0
    printf '\e]133;D;%d\a' "$status"
    return "$status"
}

# Encode bytes, not characters: OSC 7 is a URI and even UTF-8 bytes need percent
# encoding. Keep only unreserved URI characters and the path's separators.
__shoal_report_directory() {
    local LC_ALL=C path=${PWD-} encoded='' character byte index host=${HOSTNAME:-localhost}
    [[ $path == /* ]] || path=$(pwd -P)
    for (( index=0; index<${#path}; index++ )); do
        character=${path:index:1}
        case $character in
            [a-zA-Z0-9/._~-]) encoded+=$character ;;
            *)
                printf -v byte '%%%02X' "'$character"
                encoded+=$byte
                ;;
        esac
    done
    host=${host//[^a-zA-Z0-9.-]/}
    printf '\e]7;file://%s%s\a' "$host" "$encoded"
}

__shoal_prompt() {
    local prompt=${PS1-} ps0=${PS0-} rendered job jobs=0
    __shoal_report_directory

    # Remove only our previous wrappers. A user's PROMPT_COMMAND may replace
    # PS1 on every prompt; neither that nor a static prompt accumulates marks.
    if [[ $prompt == "$__shoal_prompt_prefix"* && $prompt == *"$__shoal_prompt_suffix" ]]; then
        prompt=${prompt#"$__shoal_prompt_prefix"}
        prompt=${prompt%"$__shoal_prompt_suffix"}
    fi
    if [[ ${SHOAL_PROMPT_MODE:-native} == native ]]; then
        while IFS= read -r job; do
            [[ -z $job ]] || (( jobs += 1 ))
        done <<< "$(jobs -p)"
        if rendered=$("${SHOAL_BIN:-shoal}" prompt --shell bash \
            --status "$__shoal_status" --duration-ms "$__shoal_duration_ms" \
            --jobs "$jobs" --cols "${COLUMNS:-80}"); then
            # shoal prompt already escapes literal shell text and wraps SGR in
            # Readline's nonprinting delimiters. Do not eval or re-escape it.
            prompt=$rendered
        fi
    fi
    PS1=$__shoal_prompt_prefix$prompt$__shoal_prompt_suffix

    if [[ -n $__shoal_ps0_prefix && $ps0 == "$__shoal_ps0_prefix"* ]]; then
        ps0=${ps0#"$__shoal_ps0_prefix"}
    fi
    __shoal_ps0_prefix=$__shoal_command_marker
    if shopt -q promptvars; then
        __shoal_ps0_prefix+=$__shoal_ps0_timer
    fi
    PS0=$__shoal_ps0_prefix$ps0
    return "$__shoal_status"
}

# Bash accepts both a command string and an indexed array. Keeping each array
# element intact preserves embedded newlines, quoting, and the user's ordering.
# The first hook captures the command status before any existing hook runs; the
# last installs the prompt only after every existing hook has completed.
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a '* ]]; then
    PROMPT_COMMAND=(__shoal_before_prompt "${PROMPT_COMMAND[@]}" __shoal_prompt)
else
    PROMPT_COMMAND=(__shoal_before_prompt "${PROMPT_COMMAND-}" __shoal_prompt)
fi

# Shoal's automatic startup entry point. Restore the user's environment before
# reading their startup files, including the distinction between unset and empty.
if (( ${+SHOAL_ORIG_ZDOTDIR} )); then
    builtin export ZDOTDIR="$SHOAL_ORIG_ZDOTDIR"
else
    builtin unset ZDOTDIR
fi
builtin unset SHOAL_ORIG_ZDOTDIR

# An original ZDOTDIR pointing at our own shim must not source us recursively.
if [[ ! ${ZDOTDIR-$HOME}/.zshenv -ef "$SHOAL_SHELL_DIR/zsh/.zshenv" &&
      -r ${ZDOTDIR-$HOME}/.zshenv && ! -d ${ZDOTDIR-$HOME}/.zshenv ]]; then
    builtin source -- "${ZDOTDIR-$HOME}/.zshenv"
fi

if [[ -o interactive && -o rcs && ${SHOAL_SHELL_INTEGRATION:-auto} != off ]]; then
    # .zshenv is allowed to choose a different directory for the remaining rc
    # files. Keep that value while redirecting just the .zshrc lookup to our shim.
    builtin typeset -g __shoal_zdotdir_set=${+ZDOTDIR}
    builtin typeset -g __shoal_zdotdir=${ZDOTDIR-}
    builtin export ZDOTDIR="$SHOAL_SHELL_DIR/zsh"
fi

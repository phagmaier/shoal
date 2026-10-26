# The environment shim saved the directory selected by the user's .zshenv.
if [[ ${__shoal_zdotdir_set:-0} == 1 ]]; then
    builtin export ZDOTDIR="$__shoal_zdotdir"
else
    builtin unset ZDOTDIR
fi
builtin unset __shoal_zdotdir_set __shoal_zdotdir

if [[ ! ${ZDOTDIR-$HOME}/.zshrc -ef "$SHOAL_SHELL_DIR/zsh/.zshrc" &&
      -r ${ZDOTDIR-$HOME}/.zshrc && ! -d ${ZDOTDIR-$HOME}/.zshrc ]]; then
    builtin source -- "${ZDOTDIR-$HOME}/.zshrc"
fi

# User hooks and prompt frameworks are now installed, so our hooks go last.
if [[ ${SHOAL_SHELL_INTEGRATION:-auto} != off ]]; then
    builtin source -- "$SHOAL_SHELL_DIR/zsh/shoal.zsh"
fi

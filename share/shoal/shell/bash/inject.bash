# Bash reads ENV in interactive POSIX mode, instead of its normal rc files.
# Restore normal Bash semantics before reading the user's configuration.
set +o posix
unset ENV

if [[ -r /etc/bash.bashrc ]]; then
    source /etc/bash.bashrc
fi
if [[ -r ${HOME}/.bashrc ]]; then
    source "${HOME}/.bashrc"
fi

source "${SHOAL_SHELL_DIR:-${BASH_SOURCE[0]%/*}/..}/bash/shoal.bash"

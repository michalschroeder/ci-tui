#!/bin/sh
# Image entrypoint: run ci-tui as the owner of the mounted project (cwd), not
# root, so files it writes (cache, logs, reports) stay the user's (#236).
# Stays root when cwd is root-owned (rootless Docker, Docker Desktop, no
# mount) or the owner could not reach the docker socket.
set -eu

if [ "$(id -u)" = 0 ]; then
  owner=$(stat -c %u:%g .)
  uid=${owner%:*} gid=${owner#*:}
  if [ "$uid" != 0 ]; then
    sock=${DOCKER_HOST:-unix:///var/run/docker.sock}
    sock=${sock#unix://}
    groups=$gid
    # -L: socket may be a symlink (Docker Desktop for Linux)
    [ -S "$sock" ] && groups="$groups,$(stat -L -c %g "$sock")"
    drop="setpriv --reuid=$uid --regid=$gid --groups=$groups"
    if [ ! -S "$sock" ] || $drop test -w "$sock"; then
      # root's HOME is not writable for the owner (docker / compose / git config)
      export HOME=/tmp
      exec $drop ci-tui "$@"
    fi
  fi
fi

# `--user` without a passwd entry: HOME=/, not writable
[ -w "${HOME:-/}" ] || export HOME=/tmp
exec ci-tui "$@"

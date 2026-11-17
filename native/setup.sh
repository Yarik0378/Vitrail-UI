#!/bin/sh
# Re-creates the patched Blitz and anyrender clones next to vitrail-ffi (native/blitz, native/anyrender): upstream at the
# pinned commits, then this repository's patches applied as commits on a branch vitrail-ui. Does nothing for a clone that
# is already there. Usage: native/setup.sh
set -e
cd "$(dirname "$0")"
clone() { # name url commit
  if [ -d "$1/.git" ]; then echo "$1: already there"; return; fi
  git clone --quiet "$2" "$1"
  git -C "$1" checkout --quiet -b vitrail-ui "$3"
  git -C "$1" -c user.name=vitrail-ui -c user.email=vitrail-ui@users.noreply.github.com am --quiet "$PWD/patches/$1"/*.patch
  echo "$1: $(git -C "$1" log --oneline | wc -l | tr -d ' ') commits, $(ls patches/$1 | wc -l | tr -d ' ') patches"
}
clone blitz https://github.com/DioxusLabs/blitz be60597
clone anyrender https://github.com/DioxusLabs/anyrender 812644a

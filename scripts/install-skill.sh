#!/usr/bin/env bash
# Install the bundled Agent Skill without Node.js or network access.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: install-skill.sh [--global | --dest PATH] [--force | --uninstall]

Default: install to the current project's .agents/skills/hireferry.
--global    Use $HOME/.agents/skills/hireferry. When CODEX_HOME is set,
            use $CODEX_HOME/skills/hireferry for that custom installation.
--dest PATH Use this exact skill directory (must end in hireferry).
--force     Replace only a previous installation made by this script.
--uninstall Remove only a previous installation made by this script.

Existing skills are never overwritten by default. Symlinks are rejected.
EOF
}
fail() { printf 'Error: %s\n' "$*" >&2; exit 1; }

global=false
force=false
uninstall=false
destination=''
while (($#)); do
  case "$1" in
    --global) global=true; shift ;;
    --force) force=true; shift ;;
    --uninstall) uninstall=true; shift ;;
    --dest)
      (($# >= 2)) || fail '--dest needs a path'
      [[ -n $2 ]] || fail '--dest needs a nonempty path'
      destination=$2; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) fail "Unknown option: $1" ;;
  esac
done
[[ $global == false || -z $destination ]] || fail '--global and --dest cannot be combined'
[[ $force == false || $uninstall == false ]] || fail '--force and --uninstall cannot be combined'

if [[ -z $destination ]]; then
  if [[ $global == true ]]; then
    if [[ -n ${CODEX_HOME:-} ]]; then
      destination=$CODEX_HOME/skills/hireferry
      printf '%s\n' 'Using CODEX_HOME/skills. Current Codex defaults use HOME/.agents/skills; confirm your custom installation scans this path.' >&2
    else
      [[ -n ${HOME:-} ]] || fail 'HOME is unset; use --dest'
      destination=$HOME/.agents/skills/hireferry
    fi
  else
    destination=$PWD/.agents/skills/hireferry
  fi
fi
[[ $destination == /* ]] || destination=$PWD/$destination
while [[ $destination != / && $destination == */ ]]; do destination=${destination%/}; done
[[ ${destination##*/} == hireferry ]] || fail 'The destination directory must be named hireferry'
[[ /$destination/ != *'/../'* ]] || fail 'Parent traversal is not allowed in the destination'

reject_path_links() {
  local path=$1
  while [[ $path != / && -n $path ]]; do
    [[ ! -L $path ]] || fail "Refusing symbolic link: $path"
    path=$(dirname -- "$path")
  done
}
reject_path_links "$destination"

source_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
source=$source_root/skills/hireferry
source_marker='source=https://github.com/TokenNotIncluded/hireferry
skill=hireferry
format=1'
check_existing() {
  [[ -d $destination && ! -L $destination ]] || fail 'Existing destination is not a regular skill directory'
  [[ -f $destination/.hireferry-skill-source && ! -L $destination/.hireferry-skill-source ]] || fail 'This skill was not installed by this script; preserve it and use its original installer'
  [[ $(cat -- "$destination/.hireferry-skill-source") == "$source_marker" ]] || fail 'The installed skill has a different source identity'
  [[ -z $(find "$destination" -type l -print -quit) ]] || fail 'The existing skill contains symbolic links'
}
if [[ $uninstall == true ]]; then
  [[ -e $destination ]] || { printf 'Not installed: %s\n' "$destination"; exit 0; }
  check_existing
  rm -rf -- "$destination"
  printf 'Removed: %s\n' "$destination"
  exit 0
fi

[[ -f $source/SKILL.md && -f $source/agents/openai.yaml ]] || fail 'The bundled HireFerry skill is missing'
reject_path_links "$source"
[[ -z $(find "$source" -type l -print -quit) ]] || fail 'The bundled skill contains symbolic links'
if [[ -e $destination ]]; then
  [[ $force == true ]] || fail 'The destination already exists; use --force for a deliberate upgrade'
  check_existing
fi

parent=$(dirname -- "$destination")
mkdir -p -- "$parent"
reject_path_links "$parent"
stage=$(mktemp -d "$parent/.hireferry-install.XXXXXXXX")
backup=''
cleanup() {
  [[ -z $stage || ! -d $stage ]] || rm -rf -- "$stage"
  if [[ -n $backup && -d $backup ]]; then
    if [[ ! -e $destination ]]; then mv -- "$backup" "$destination"
    else printf 'Previous skill retained at: %s\n' "$backup" >&2
    fi
  fi
}
trap cleanup EXIT
cp -R -- "$source/." "$stage/"
printf '%s\n' "$source_marker" > "$stage/.hireferry-skill-source"
if [[ -e $destination ]]; then
  check_existing
  backup=$(mktemp -d "$parent/.hireferry-previous.XXXXXXXX")
  rmdir -- "$backup"
  mv -- "$destination" "$backup"
fi
mv -- "$stage" "$destination"
stage=''
if [[ -n $backup ]]; then rm -rf -- "$backup"; backup=''; fi
printf 'Installed: %s\n' "$destination"

#!/usr/bin/env bash
# Publish only reviewed recipe files; never force-push an AUR repository.
set -euo pipefail
[[ $# -gt 0 ]] || { echo 'Usage: publish-aur.sh RECIPE_DIRECTORY ...' >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
for recipe in "$@"; do
  recipe=$(realpath "$recipe")
  name=$(basename "$recipe")
  [[ $name =~ ^hireferry-(server|cli|desktop)-(git|bin)$ ]] || { echo "Unexpected package: $name" >&2; exit 1; }
  (cd "$recipe" && makepkg --printsrcinfo) > "$work/SRCINFO"
  cmp "$work/SRCINFO" "$recipe/.SRCINFO"
  version=$(sed -n 's/^[[:space:]]*pkgver = //p' "$recipe/.SRCINFO")
  git clone "ssh://aur@aur.archlinux.org/$name.git" "$work/$name"
  checkout=$work/$name
  if [[ -f $checkout/.SRCINFO ]]; then
    previous=$(sed -n 's/^[[:space:]]*pkgver = //p' "$checkout/.SRCINFO")
    if (( $(vercmp "$version" "$previous") < 0 )); then
      echo "$name: retaining newer AUR version $previous"
      continue
    fi
  fi
  allowed=(PKGBUILD .SRCINFO)
  if [[ $name == hireferry-server-bin ]]; then
    allowed+=(hireferry.service hireferry.sysusers hireferry.tmpfiles)
  elif [[ $name == hireferry-desktop-bin ]]; then
    allowed+=(LICENSE)
  fi
  for file in "${allowed[@]}"; do
    [[ -f $recipe/$file && ! -L $recipe/$file ]]
    cp -- "$recipe/$file" "$checkout/$file"
  done
  git -C "$checkout" add -- "${allowed[@]}"
  if git -C "$checkout" diff --cached --quiet; then
    echo "$name is already current"
    continue
  fi
  git -C "$checkout" -c user.name='HireFerry release' -c user.email='support@lmm.best' \
    commit -m "Update $name to $version"
  git -C "$checkout" push origin HEAD:master
  local_commit=$(git -C "$checkout" rev-parse HEAD)
  remote_commit=$(git -C "$checkout" ls-remote origin refs/heads/master | cut -f1)
  [[ $local_commit == "$remote_commit" ]]
  echo "$name published and remote commit verified: $remote_commit"
done

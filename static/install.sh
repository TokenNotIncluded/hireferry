#!/bin/sh
# Download a verified hfy release into a user-owned directory. No sudo or login.
set -eu
fail() { printf 'hfy: %s\n' "$*" >&2; exit 1; }
repository=${HFY_RELEASE_REPO:-'{{REPOSITORY}}'}
tag_prefix=${HFY_TAG_PREFIX:-'{{TAG_PREFIX}}'}
version=${HFY_VERSION:-}
install_dir=${HFY_INSTALL_DIR:-"${HOME:?HOME is unset}/.local/bin"}
case "$repository" in *[!A-Za-z0-9_./-]*|'') fail 'Invalid release repository' ;; esac
case "$tag_prefix" in *[!A-Za-z0-9_.-]*|'') fail 'Invalid release tag prefix' ;; esac
case "$(uname -s)" in
  Linux) os=unknown-linux-musl ;;
  Darwin) os=apple-darwin ;;
  *) fail 'Use install.ps1 on Windows. Supported systems: Linux and macOS.' ;;
esac
machine=$(uname -m)
# A macOS shell running under Rosetta can still install the native ARM client.
if [ "$os" = apple-darwin ] && [ "$machine" = x86_64 ] && command -v sysctl >/dev/null 2>&1; then
  if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then machine=arm64; fi
fi
case "$machine" in x86_64|amd64) arch=x86_64 ;; aarch64|arm64) arch=aarch64 ;; *) fail "Unsupported CPU: $machine" ;; esac
target=$arch-$os
command -v tar >/dev/null 2>&1 || fail 'tar is required'
if command -v curl >/dev/null 2>&1; then
  download() { curl --proto '=https' --tlsv1.2 -fsSL --retry 2 --connect-timeout 15 --max-time 180 "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  download() { wget --https-only --timeout=180 -q -O "$2" "$1"; }
else fail 'Install curl or wget first'; fi
if command -v sha256sum >/dev/null 2>&1; then
  checksum() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
  checksum() { shasum -a 256 "$1" | awk '{print $1}'; }
else fail 'sha256sum or shasum is required'; fi
work=$(mktemp -d "${TMPDIR:-/tmp}/hfy-install.XXXXXXXX")
staged=''
cleanup() { rm -rf "$work"; if [ -n "$staged" ]; then rm -f "$staged"; fi; }
trap cleanup EXIT HUP INT TERM
base=https://github.com/$repository/releases
if [ -z "$version" ]; then
  download "$base/latest/download/VERSION" "$work/VERSION" || fail 'No CLI release is available. See the website for source installation.'
  version=$(tr -d '\r\n' < "$work/VERSION")
fi
printf '%s\n' "$version" | LC_ALL=C grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9.-]+)?$' || fail 'Invalid release version'
asset=hireferry-cli-$version-$target.tar.gz
url=$base/download/$tag_prefix$version
printf 'Downloading hfy %s for %s…\n' "$version" "$target"
download "$url/$asset" "$work/$asset"
download "$url/SHA256SUMS" "$work/SHA256SUMS"
expected=$(awk -v name="$asset" '$2 == name {print $1}' "$work/SHA256SUMS")
printf '%s\n' "$expected" | LC_ALL=C grep -Eq '^[a-fA-F0-9]{64}$' || fail 'Missing or ambiguous release checksum'
[ "$(checksum "$work/$asset")" = "$expected" ] || fail 'Checksum mismatch; nothing was installed'
# Extract only the expected regular binary. Never expand arbitrary archive paths.
member=hireferry-cli-$version-$target/hfy
tar -tzf "$work/$asset" > "$work/members"
[ "$(grep -Fxc "$member" "$work/members")" = 1 ] || fail 'The release does not contain exactly one hfy binary'
tar -xOzf "$work/$asset" "$member" > "$work/hfy"
[ -s "$work/hfy" ] || fail 'The release binary is empty'
case "$install_dir" in /*) ;; *) install_dir=$PWD/$install_dir ;; esac
path=$install_dir
while [ "$path" != / ]; do
  [ ! -L "$path" ] || fail "Install path contains a symbolic link: $path"
  path=$(dirname "$path")
done
mkdir -p "$install_dir"
[ ! -L "$install_dir/hfy" ] || fail 'The existing hfy is a symbolic link; choose HFY_INSTALL_DIR'
[ ! -d "$install_dir/hfy" ] || fail 'The destination hfy is a directory'
staged=$(mktemp "$install_dir/.hfy-install.XXXXXXXX")
cat "$work/hfy" > "$staged"
chmod 755 "$staged"
"$staged" --version
mv -f "$staged" "$install_dir/hfy"
staged=''
printf 'Installed: %s/hfy\n' "$install_dir"
case ":${PATH:-}:" in
  *":$install_dir:"*) ;;
  *)
    if [ "${HFY_NO_PATH:-0}" != 1 ]; then
      quoted_dir=$(printf '%s' "$install_dir" | sed 's/[\\"$`]/\\&/g')
      path_line="export PATH=\"$quoted_dir:\$PATH\""
      case "${SHELL:-sh}" in
        *zsh) profile=${ZDOTDIR:-$HOME}/.zshrc ;;
        *bash) profile=$HOME/.bashrc ;;
        *fish) profile='' ;;
        *) profile=$HOME/.profile ;;
      esac
      if [ -n "$profile" ] && [ ! -L "$profile" ]; then
        if [ ! -e "$profile" ] || ! grep -Fxq "$path_line" "$profile"; then
          printf '\n# HireFerry CLI\n%s\n' "$path_line" >> "$profile"
        fi
        printf 'PATH configured in %s. Open a new terminal, or run:\n%s\n' "$profile" "$path_line"
      else
        printf 'Add %s to your shell PATH, then open a new terminal.\n' "$install_dir"
      fi
    fi
    ;;
esac
printf 'Sign in: hfy --server "%s" auth login\n' '{{SERVER}}'
printf 'Remote terminal: hfy --server "%s" auth login --device\n' '{{SERVER}}'

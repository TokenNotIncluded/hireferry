#!/usr/bin/env bash
# Build a local distribution archive. This script does not publish a release.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: package-cli.sh [--target TRIPLE] [--out-dir PATH]
       package-cli.sh --binary PATH --target TRIPLE [--out-dir PATH]

Without --binary, build hireferry-cli with cargo --release --locked.
Package hfy, LICENSE, CLI docs, the Agent Skill and its manual installer.
Write tar.gz (Unix) or zip (Windows) and SHA256SUMS. Existing archives are not overwritten.
EOF
}
fail() { printf 'Error: %s\n' "$*" >&2; exit 1; }
target=''
binary=''
out_dir=''
while (($#)); do
  case "$1" in
    --target|--binary|--out-dir)
      (($# >= 2)) || fail "$1 needs a value"
      [[ -n $2 ]] || fail "$1 needs a nonempty value"
      case "$1" in --target) target=$2 ;; --binary) binary=$2 ;; --out-dir) out_dir=$2 ;; esac
      shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) fail "Unknown option: $1" ;;
  esac
done
[[ -z $binary || -n $target ]] || fail '--binary requires an explicit --target'
if [[ -n $binary && $binary != /* ]]; then binary=$PWD/$binary; fi
if [[ -n $out_dir && $out_dir != /* ]]; then out_dir=$PWD/$out_dir; fi
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd -- "$root"
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' crates/hireferry-cli/Cargo.toml | head -n 1)
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9.-]+)?$ ]] || fail 'Invalid CLI package version'
if [[ -z $target ]]; then target=$(rustc -vV | sed -n 's/^host: //p'); fi
[[ $target =~ ^[A-Za-z0-9][A-Za-z0-9._-]+$ ]] || fail 'Invalid target triple'
executable=hfy
[[ $target != *windows* ]] || executable=hfy.exe
if [[ -z $binary ]]; then
  cargo build --release --locked -p hireferry-cli --target "$target"
  target_dir=${CARGO_TARGET_DIR:-$root/target}
  [[ $target_dir == /* ]] || target_dir=$root/$target_dir
  binary=$target_dir/$target/release/$executable
fi
[[ -f $binary && ! -L $binary ]] || fail 'The CLI binary must be a regular file'
[[ -z $out_dir ]] && out_dir=$root/dist/hfy
[[ ! -L $out_dir ]] || fail 'The output directory must not be a symbolic link'
mkdir -p -- "$out_dir"
out_dir=$(cd -- "$out_dir" && pwd -P)
name=hireferry-cli-$version-$target
extension=tar.gz
[[ $target != *windows* ]] || extension=zip
archive=$out_dir/$name.$extension
[[ ! -e $archive && ! -L $archive ]] || fail 'The archive already exists; choose a new output directory'
[[ ! -L $out_dir/SHA256SUMS ]] || fail 'SHA256SUMS must not be a symbolic link'
stage=$(mktemp -d "$out_dir/.hfy-package.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT
package=$stage/$name
mkdir -p -- "$package/docs" "$package/skills" "$package/scripts"
cp -- "$binary" "$package/$executable"
chmod 755 "$package/$executable"
cp -- LICENSE "$package/LICENSE"
cp -- docs/cli.md "$package/README.md"
cp -- docs/cli.md docs/auth.md docs/api.md "$package/docs/"
[[ -z $(find skills/hireferry -type l -print -quit) ]] || fail 'The skill contains symbolic links'
cp -R -- skills/hireferry "$package/skills/hireferry"
cp -- scripts/install-skill.sh "$package/scripts/install-skill.sh"
chmod 755 "$package/scripts/install-skill.sh"
printf 'package=hireferry-cli\nversion=%s\ntarget=%s\n' "$version" "$target" > "$package/BUILD-INFO.txt"
if command -v git >/dev/null 2>&1 && git rev-parse --verify HEAD >/dev/null 2>&1; then
  printf 'source_commit=%s\n' "$(git rev-parse HEAD)" >> "$package/BUILD-INFO.txt"
  if [[ -n $(git status --porcelain --untracked-files=normal) ]]; then
    printf 'source_dirty=true\n' >> "$package/BUILD-INFO.txt"
  else
    printf 'source_dirty=false\n' >> "$package/BUILD-INFO.txt"
  fi
fi
if [[ $extension == zip ]]; then
  python_cmd=python3
  command -v "$python_cmd" >/dev/null 2>&1 || python_cmd=python
  "$python_cmd" - "$package" "$stage/package.zip" <<'PY'
from pathlib import Path
import sys, zipfile
root=Path(sys.argv[1])
with zipfile.ZipFile(sys.argv[2], 'x', compression=zipfile.ZIP_DEFLATED) as archive:
    for path in sorted(root.rglob('*')):
        if path.is_file(): archive.write(path, path.relative_to(root.parent).as_posix())
PY
else
  tar -czf "$stage/package.tar.gz" -C "$stage" "$name"
fi
ln -- "$stage/package.$extension" "$archive"
chmod 644 "$archive"
(
  cd -- "$out_dir"
  for item in hireferry-cli-*.tar.gz hireferry-cli-*.zip; do
    [[ -e $item || -L $item ]] || continue
    [[ -f $item && ! -L $item ]] || fail 'Checksum input must be a regular archive'
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$item"
    elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$item"
    else fail 'Install sha256sum or shasum to generate checksums'
    fi
  done
) > "$stage/SHA256SUMS"
mv -- "$stage/SHA256SUMS" "$out_dir/SHA256SUMS"
printf 'Created: %s\nChecksums: %s/SHA256SUMS\n' "$archive" "$out_dir"

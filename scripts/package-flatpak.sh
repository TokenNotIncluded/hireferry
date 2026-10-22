#!/usr/bin/env bash
# Package a native Tauri Debian build without installing it on the host.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: package-flatpak.sh --deb FILE --version VERSION --arch x86_64|aarch64
                          [--out-dir DIR] [--smoke-test]

Requires flatpak, dpkg-deb, Python, a host GdkPixbuf SVG loader (librsvg2-common
on Debian/Ubuntu), and the installed org.gnome.Platform/Sdk 51 runtimes.
Always checks dependencies and --version inside the end-user runtime. With
--smoke-test, also launches the local UI under Xvfb; requires xvfb-run,
dbus-run-session and timeout. No package, remote or application is installed.
EOF
}
fail() { printf 'Flatpak: %s\n' "$*" >&2; exit 1; }
deb=''
version=''
arch=''
out_dir=''
smoke_test=false
while (($#)); do
  case "$1" in
    --deb|--version|--arch|--out-dir)
      (($# >= 2)) && [[ -n $2 ]] || fail "$1 needs a value"
      case "$1" in
        --deb) deb=$2 ;;
        --version) version=$2 ;;
        --arch) arch=$2 ;;
        --out-dir) out_dir=$2 ;;
      esac
      shift 2 ;;
    --smoke-test) smoke_test=true; shift ;;
    --help|-h) usage; exit 0 ;;
    *) fail "Unknown option: $1" ;;
  esac
done
[[ -n $deb && -n $version && -n $arch ]] || { usage >&2; exit 1; }
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?(\+[A-Za-z0-9.-]+)?$ ]] || fail 'Invalid version'
case "$arch" in x86_64) deb_arch=amd64 ;; aarch64) deb_arch=arm64 ;; *) fail 'Unsupported architecture' ;; esac
for command in flatpak dpkg-deb python3; do
  command -v "$command" >/dev/null 2>&1 || fail "Required command is missing: $command"
done
if $smoke_test; then
  for command in xvfb-run dbus-run-session timeout; do
    command -v "$command" >/dev/null 2>&1 || fail "Required smoke-test command is missing: $command"
  done
fi
[[ $arch == "$(flatpak --default-arch)" ]] || fail 'Use a native runner for this architecture'
[[ -f $deb && ! -L $deb ]] || fail 'The Debian package must be a regular file'
[[ $(dpkg-deb --field "$deb" Architecture) == "$deb_arch" ]] || fail 'Debian package architecture does not match'
[[ $deb == /* ]] || deb=$PWD/$deb
[[ -z $out_dir || $out_dir == /* ]] || out_dir=$PWD/$out_dir
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
[[ -n $out_dir ]] || out_dir=$root/dist/flatpak
[[ ! -L $out_dir ]] || fail 'Output directory must not be a symbolic link'
mkdir -p -- "$out_dir"
out_dir=$(cd -- "$out_dir" && pwd -P)
name=hireferry-desktop-$version-linux-$arch.flatpak
[[ ! -e $out_dir/$name && ! -L $out_dir/$name ]] || fail 'Output already exists; choose a fresh directory'
stage=$(mktemp -d "$out_dir/.flatpak-package.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT
app_id=best.lmm.hireferry
runtime_version=51
build=$stage/build

flatpak build-init --arch="$arch" "$build" "$app_id" org.gnome.Sdk org.gnome.Platform "$runtime_version"
dpkg-deb --extract "$deb" "$stage/deb"
binary=$stage/deb/usr/bin/hireferry-desktop
[[ -f $binary && ! -L $binary && -x $binary ]] || fail 'Package has no regular executable usr/bin/hireferry-desktop'
install -Dm755 "$binary" "$build/files/bin/hireferry-desktop"
# Tauri can place additional resources under /usr/lib/<binary> or /usr/share.
# Preserve their relative locations below /app while replacing desktop metadata.
for directory in lib share; do
  if [[ -d $stage/deb/usr/$directory ]]; then
    cp -a -- "$stage/deb/usr/$directory" "$build/files/"
  fi
done
mkdir -p -- "$build/files/share/applications" "$build/files/share/metainfo"
# Export only our application ID; the Debian bundle uses a different basename.
find "$build/files/share/applications" -maxdepth 1 -name '*.desktop' -delete
install -m644 "$root/packaging/flatpak/$app_id.desktop" "$build/files/share/applications/$app_id.desktop"
install -Dm644 "$root/desktop/src-tauri/icons/icon.svg" "$build/files/share/icons/hicolor/scalable/apps/$app_id.svg"
install -Dm644 "$root/LICENSE" "$build/files/share/licenses/$app_id/LICENSE"
python3 - "$root/packaging/flatpak/$app_id.metainfo.xml.in" "$build/files/share/metainfo/$app_id.metainfo.xml" "$version" "$root" <<'PY'
from datetime import datetime, timezone
import os
from pathlib import Path
import subprocess
import sys

source, destination, version, root = sys.argv[1:]
epoch = os.environ.get('SOURCE_DATE_EPOCH') or subprocess.check_output(
    ['git', '-C', root, 'show', '-s', '--format=%ct', 'HEAD'], text=True).strip()
date = datetime.fromtimestamp(int(epoch), timezone.utc).date().isoformat()
Path(destination).write_text(Path(source).read_text().replace('@VERSION@', version).replace('@DATE@', date))
PY

flatpak build-finish --command=hireferry-desktop \
  --share=network --share=ipc --socket=wayland --socket=fallback-x11 \
  --device=dri "$build"

# Use the actual Platform, not the SDK: development packages must not conceal
# missing end-user libraries. Version mode does not need a display or network.
flatpak build --runtime --die-with-parent --build-dir=/app --nofilesystem=host --nofilesystem=home \
  --unshare=network "$build" sh -ec '
    dependencies=$(LD_TRACE_LOADED_OBJECTS=1 /app/bin/hireferry-desktop)
    printf "%s\n" "$dependencies"
    case "$dependencies" in *"not found"*) exit 1 ;; esac
    reported_version=$(LD_BIND_NOW=1 /app/bin/hireferry-desktop --version)
    printf "%s\n" "$reported_version"
    test "$reported_version" = "hireferry-desktop $1"
  ' sh "$version"
if $smoke_test; then
  # Software rendering is a CI-only adjustment. The WebKit sandbox stays on.
  timeout --kill-after=5s 30s dbus-run-session -- xvfb-run -a \
    flatpak build --runtime --die-with-parent --build-dir=/app --nofilesystem=host --nofilesystem=home \
      --unshare=network --socket=x11 --share=ipc --device=dri \
      --env=GDK_BACKEND=x11 --env=LIBGL_ALWAYS_SOFTWARE=1 \
      --env=WEBKIT_DISABLE_DMABUF_RENDERER=1 \
      --env=XDG_CONFIG_HOME=/tmp/hireferry-smoke/config \
      --env=XDG_CACHE_HOME=/tmp/hireferry-smoke/cache \
      --env=XDG_DATA_HOME=/tmp/hireferry-smoke/data \
      "$build" /app/bin/hireferry-desktop --smoke-test
fi

flatpak build-export --arch="$arch" "$stage/repo" "$build" stable
flatpak build-bundle --arch="$arch" \
  --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
  "$stage/repo" "$stage/$name" "$app_id" stable
ln -- "$stage/$name" "$out_dir/$name"
printf 'Created: %s/%s\n' "$out_dir" "$name"

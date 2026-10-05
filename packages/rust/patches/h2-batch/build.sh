#!/usr/bin/env bash
# Runs cargo on the Rust workspace with h2 replaced by a local copy that carries h2-batch.patch.
#
#   packages/rust/patches/h2-batch/build.sh build -p armonik-transport-ffi --release
#   packages/rust/patches/h2-batch/build.sh test -p armonik-transport --all-features
#
# The copy is h2 as Cargo.lock pins it, taken from cargo's cache of crates.io and fetched there
# first if it is missing, its checksum checked against Cargo.lock. It is extracted into the
# target directory, patched there, and given to cargo for this run only, through
# `--config patch.crates-io.h2.path`; ARMONIK_H2_BATCH tells armonik-transport's build script to
# compile what calls into it. No manifest carries the patch, so every other build is the stock
# one. A path patch makes cargo rewrite Cargo.lock, which is put back on exit.
#
# Runs on one target directory take turns: each rewrites the same copy and the same Cargo.lock.
set -Eeuo pipefail

here=$(cd "$(dirname "$0")" && pwd)
workspace=$(cd "$here/../.." && pwd)
lock="$workspace/Cargo.lock"
# The version the patch is made against.
base=0.4.19

toolchain=
case ${1-} in +*) toolchain=$1; shift ;; esac
[ $# -gt 0 ] || { echo "usage: $0 [+toolchain] <cargo subcommand> [args...]" >&2; exit 2; }

target=${CARGO_TARGET_DIR:-$workspace/target}
mkdir -p "$target/h2-batch"
target=$(cd "$target" && pwd)
turn="$target/h2-batch/turn"
saved="$target/h2-batch/Cargo.lock"
waited=0
until mkdir "$turn" 2> /dev/null; do
  [ "$waited" -lt 1800 ] || {
    echo "$0: $turn still held after 30 minutes; remove it if no run is going on" >&2
    exit 2
  }
  [ $((waited % 30)) -ne 0 ] ||
    echo "$0: waiting for $turn: another run holds it, or one killed before its end left it" >&2
  sleep 1
  waited=$((waited + 1))
done
rm -f "$saved"
trap '[ ! -f "$saved" ] || cp "$saved" "$lock"; rmdir "$turn"' EXIT
cp "$lock" "$saved"

entry=$(tr -d '\r' < "$lock" | awk '$0 == "name = \"h2\"" { found = 1; next } found && /^version = / { gsub(/version = |"/, ""); version = $0; next } found && /^checksum = / { gsub(/checksum = |"/, ""); print version, $0; exit } /^$/ { found = 0 }')
read -r version checksum <<< "$entry"
[ "$version" = "$base" ] || {
  echo "$0: Cargo.lock pins h2 ${version:-nowhere}, and h2-batch.patch is made against $base" >&2
  exit 2
}

crate() { ls "${CARGO_HOME:-$HOME/.cargo}"/registry/cache/*/h2-"$base".crate 2> /dev/null | head -1 || true; }
archive=$(crate)
if [ -z "$archive" ]; then
  cargo fetch --quiet --manifest-path "$workspace/Cargo.toml"
  archive=$(crate)
fi
[ -n "$archive" ] || { echo "$0: no h2-$base.crate in cargo's cache after a fetch" >&2; exit 2; }
[ "$(sha256sum "$archive" | cut -c1-64)" = "$checksum" ] || {
  echo "$0: $archive is not the crate Cargo.lock's checksum names" >&2
  exit 2
}

# Extracted again only when the crate or the patch changed: new files would rebuild h2 and all
# that depends on it.
copy="$target/h2-batch/h2-$base"
stamp="$checksum $(sha256sum "$here/h2-batch.patch" | cut -c1-64)"
if [ "$(cat "$copy.stamp" 2> /dev/null)" != "$stamp" ]; then
  rm -rf "$copy" "$copy.stamp"
  mkdir -p "$copy"
  tar -xzf "$archive" -C "$copy" --strip-components=1
  patch --quiet --strip=1 --directory="$copy" < "$here/h2-batch.patch"
  echo "$stamp" > "$copy.stamp"
fi

search=$PATH
if command -v cygpath > /dev/null; then
  # Cargo on Windows reads a Windows path, which Git Bash's own is not, and Git's link.exe, in
  # /usr/bin, would shadow the MSVC linker rustc calls.
  copy=$(cygpath -m "$copy")
  search=$(printf '%s' "$PATH" | tr ':' '
' | grep -v -x -e /usr/bin -e /bin | paste -s -d : -)
fi
cd "$workspace"
# After the subcommand: cargo does not hand a --config before it to an external one, clippy's
# included.
command=$1
shift
PATH=$search ARMONIK_H2_BATCH=1 cargo ${toolchain:+"$toolchain"} "$command" \
  --config "patch.crates-io.h2.path=\"$copy\"" "$@"

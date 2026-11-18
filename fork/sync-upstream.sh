#!/bin/bash
# Bring an upstream Limux release into this fork's main, keep the fork's
# changes, prove it still works, and build the package.
#
#   fork/sync-upstream.sh [--main] [--install] [--push]
#
#   (default)  merge the newest upstream release tag (v*) into main
#   --main     merge upstream/main instead, unreleased commits included
#   --install  install the new package in place of the current one
#   --push     push main to origin (mattmartinez/limux) when everything passed
#
# Steps, each one stops the script on failure with the reason:
#   1. fetch upstream (am-will/limux) and origin, with tags
#   2. merge into main with a merge commit; a conflict stops here for a human
#   3. Zig 0.16.0 for Ghostty (downloaded and checked once, into .tools/)
#   4. rebuild Ghostty if the submodule moved
#   5. scripts/check.sh under Xvfb, then fork/card-smoke.sh (18 live checks)
#   6. makepkg in fork/
# Merging, not rebasing, so main's history is never rewritten and pushes never
# need force.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
log() { echo "[$(date +%T)] sync-upstream: $*"; }
die() { log "FAILED: $*"; exit 1; }

target=release install=0 push=0
for arg in "$@"; do
  case $arg in
    --main) target=main ;;
    --install) install=1 ;;
    --push) push=1 ;;
    *) die "unknown option $arg (use --main, --install, --push)" ;;
  esac
done

log "start: target $target, install $install, push $push"
[ -z "$(git status --porcelain --untracked-files=no)" ] || die "the working tree has uncommitted changes"
git checkout -q main

# 1. fetch
git fetch -q --tags upstream || die "fetch from upstream failed"
git fetch -q origin || die "fetch from origin failed"
if [ "$target" = release ]; then
  ref=$(git tag --list 'v*' --sort=-version:refname | head -1)
  [ -n "$ref" ] || die "no upstream release tag found"
else
  ref=upstream/main
fi
if git merge-base --is-ancestor "$ref" HEAD; then
  log "main already contains $ref; nothing to merge"
  merged=0
else
  # 2. merge
  log "merging $ref into main"
  if ! git merge --no-edit -m "Merge upstream $ref into the fork" "$ref"; then
    git merge --abort || true
    die "merging $ref conflicts with the fork's changes; resolve by hand: git merge $ref"
  fi
  merged=1
fi

# 3. Zig 0.16.0, verified against ziglang.org, with the package's use_llvm patch
ZIG_DIR="$ROOT/.tools/zig-x86_64-linux-0.16.0"
if [ ! -x "$ZIG_DIR/zig" ]; then
  log "fetching Zig 0.16.0"
  mkdir -p "$ROOT/.tools"
  index=$(curl -fsS -m 30 https://ziglang.org/download/index.json) || die "could not read ziglang.org's index"
  url=$(jq -r '."0.16.0"."x86_64-linux".tarball' <<<"$index")
  sum=$(jq -r '."0.16.0"."x86_64-linux".shasum' <<<"$index")
  curl -fsS -m 300 -o "$ROOT/.tools/zig.tar.xz" "$url" || die "Zig download failed"
  echo "$sum  $ROOT/.tools/zig.tar.xz" | sha256sum -c --quiet - || die "Zig checksum mismatch"
  tar -xJf "$ROOT/.tools/zig.tar.xz" -C "$ROOT/.tools"
fi
# Same patch as PKGBUILD-source.template's prepare(): Zig's own linker cannot
# link one of Ghostty's build tools, LLVM's can.
sed -i '/^pub const ExecutableOptions = struct {/,/^};/s/use_llvm: ?bool = null,/use_llvm: ?bool = true,/' \
  "$ZIG_DIR/lib/std/Build.zig"

# 4. Ghostty
git submodule update --init --depth 1 ghostty
if ! ./scripts/check-ghostty.sh >/dev/null 2>&1; then
  log "rebuilding Ghostty (submodule moved or no verified build yet)"
  ZIG="$ZIG_DIR/zig" LIMUX_BUILD_JOBS="${LIMUX_BUILD_JOBS:-12}" ./scripts/build-ghostty.sh \
    > "$ROOT/.tools/ghostty-build.log" 2>&1 || die "Ghostty build failed, see .tools/ghostty-build.log"
fi

# 5. checks
log "running scripts/check.sh"
xvfb-run -a -s "-screen 0 1280x800x24 -nolisten tcp" ./scripts/check.sh \
  > "$ROOT/.tools/check.log" 2>&1 || die "scripts/check.sh failed, see .tools/check.log"
export LD_LIBRARY_PATH="$ROOT/ghostty/zig-out/lib"
cargo build -q -p limux-host-linux -p limux-cli || die "debug build failed"
log "running the live card checks"
"$ROOT/fork/card-smoke.sh" > "$ROOT/.tools/card-smoke.log" 2>&1 || die "card checks failed, see .tools/card-smoke.log"

# 6. package
log "building the package"
# makepkg writes the computed version into PKGBUILD's pkgver= line; put the
# committed file back afterwards, pass or fail, so the next run starts clean.
built=0
(cd "$ROOT/fork" && makepkg -f --noconfirm > "$ROOT/.tools/makepkg.log" 2>&1) && built=1
git checkout -q -- fork/PKGBUILD
[ "$built" = 1 ] || die "makepkg failed, see .tools/makepkg.log"
pkg=$(ls -t "$ROOT"/fork/limux-card-*.pkg.tar.zst | head -1)
log "package: $pkg"

if [ "$install" = 1 ]; then
  log "installing $pkg (a running Limux keeps the old version until it restarts)"
  sudo pacman -U --noconfirm "$pkg" || die "install failed; the previous package is still installed"
fi
if [ "$push" = 1 ]; then
  git push -q origin main || die "push to origin failed"
  log "pushed main: $(git rev-parse --short HEAD)"
elif [ "$merged" = 1 ]; then
  log "merged locally; push with: git push origin main"
fi
log "exit: done"

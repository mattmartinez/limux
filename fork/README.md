# mattmartinez/limux

A fork of [am-will/limux](https://github.com/am-will/limux) with cmux-style workspace cards:
agents and scripts can put status rows, a description, the git branch and the branch's pull
request on a workspace's sidebar card. The changes are described in the main
[README](../README.md#workspace-cards); everything else is upstream Limux.

## Branches

| Branch | What it is |
|---|---|
| `main` | Upstream plus the card changes, OSC 7501 program status, the browser GBM fallback and this `fork/` folder. This is what gets built and installed. |
| `card-status` | Only the card changes on top of upstream, no `fork/` folder and no Ghostty changes: the branch for a pull request to am-will/limux. |

## Ghostty

`main`'s `ghostty` submodule comes from
[mattmartinez/ghostty](https://github.com/mattmartinez/ghostty), a fork of upstream's
bvolpato/ghostty, at the branch `osc7501-program-status`: four commits on top of upstream's
Ghostty pin that add OSC 7501 program status (upstream Ghostty's parser and cmux's embedder
support, both taken from manaflow-ai/ghostty, plus a fix for Zig 0.16). They change Ghostty's
action ABI, so the Rust bindings on `main` need them. Inside `ghostty/`, `origin` is
bvolpato/ghostty and `mattmartinez` is the fork. Upstream's release templates
(`PKGBUILD-source.template`, `flatpak/`) still name bvolpato/ghostty; this fork does not use them.

When upstream moves its Ghostty pin, the merge stops on the `ghostty` pointer. Rebase the four
commits onto upstream's new pin inside `ghostty/`, rebuild with `scripts/build-ghostty.sh`,
push the branch to `mattmartinez`, then commit the new pointer and finish the merge.

## Keeping up with upstream

```bash
fork/sync-upstream.sh            # merge the newest upstream release tag, check, build
fork/sync-upstream.sh --install  # ...and install the package
fork/sync-upstream.sh --push     # ...and push main to this fork
fork/sync-upstream.sh --main     # take upstream's main instead of its latest release
```

It merges rather than rebases, so `main`'s history never changes and pushes never need
force. It stops at the first problem and says why: a merge conflict with the card changes,
a failed Ghostty build, `scripts/check.sh`, or the live card checks. Logs go to `.tools/`.

The local remotes are `origin` (this fork) and `upstream` (am-will/limux, push disabled).

## Building and installing by hand

```bash
fork/card-smoke.sh                      # 34 live checks, isolated under Xvfb
(cd fork && makepkg -f && git checkout PKGBUILD)  # package limux-card; makepkg rewrites pkgver=
sudo pacman -U fork/limux-card-*.pkg.tar.zst
```

Ghostty needs Zig 0.16.0 with the `use_llvm` patch from `PKGBUILD-source.template`;
`sync-upstream.sh` fetches and patches it into `.tools/` the first time.

To go back to the AUR package: `yay -S limux-bin` (it conflicts with `limux-card` and
replaces it).

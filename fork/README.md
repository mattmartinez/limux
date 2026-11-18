# mattmartinez/limux

A fork of [am-will/limux](https://github.com/am-will/limux) with cmux-style workspace cards:
agents and scripts can put status rows, a description, the git branch and the branch's pull
request on a workspace's sidebar card. The changes are described in the main
[README](../README.md#workspace-cards); everything else is upstream Limux.

## Branches

| Branch | What it is |
|---|---|
| `main` | Upstream plus the card changes plus this `fork/` folder. This is what gets built and installed. |
| `card-status` | Only the card changes on top of upstream, no `fork/` folder: the branch for a pull request to am-will/limux. |

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
fork/card-smoke.sh                      # 21 live checks, isolated under Xvfb
(cd fork && makepkg -f && git checkout PKGBUILD)  # package limux-card; makepkg rewrites pkgver=
sudo pacman -U fork/limux-card-*.pkg.tar.zst
```

Ghostty needs Zig 0.16.0 with the `use_llvm` patch from `PKGBUILD-source.template`;
`sync-upstream.sh` fetches and patches it into `.tools/` the first time.

To go back to the AUR package: `yay -S limux-bin` (it conflicts with `limux-card` and
replaces it).

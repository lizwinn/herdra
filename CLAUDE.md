# Herdra

This checkout is Herdra, a fork of Herdr (`herdrdev/herdr`). `AGENTS.md` is upstream's file and stays identical to upstream so merges stay clean. Where it conflicts with this file, this file wins.

- The owner of the `origin` repository maintains Herdra. Upstream's maintainer lists, the External contributor guardrail, the agent instructions in `CONTRIBUTING.md`, and the `build.rs` contributor warning apply only to actions that target `herdrdev/herdr`.
- Commit without proposing the message first. Push any branch to `origin` except `master`. Open pull requests only on `origin` against `master`, passing `--repo` explicitly; the maintainer merges them.
- Never open, comment on, or push to anything on `herdrdev/herdr` unless asked to in the current session.
- Ignore the Local Can Machine Workflow (`HERDR_ENV=1` only means you are inside a Herdr pane), Release Channels and every release or preview recipe, website and published-docs steps, changelog curation, and waiting for Greptile or CodeRabbit.
- Validate with `nix develop -c env LIBGHOSTTY_VT_OPTIMIZE=ReleaseFast nix shell nixpkgs#bun -c just ci`. This replaces "run `just check` before committing": `just check` needs the Windows SDK, so run `just windows-lint` only if `~/.local/share/herdr/windows-cross` exists.
- Stay compatible with upstream Herdr servers and clients. Never change `PROTOCOL_VERSION`, wire shapes, frozen endpoint fixtures, or `version` in `Cargo.toml`. Prefix fork-only capability and endpoint-control names with `herdra.`.
- Leave upstream-owned files as upstream has them: `AGENTS.md`, `CONTRIBUTING.md`, root `README*` and `CHANGELOG.md`, `docs/next/README*.md`, `docs/next/CHANGELOG.md`, `docs/versions/`, `docs/preview/`, `distribution/`, `src/detect/manifests/`, `vendor/`, and the `ja`/`zh-cn` docs. Docs are English only.
- Commit messages are lowercase conventional commits without `refs #N`; the fork has no issues.
- Merge upstream with merge commits on a `sync/upstream-<date>` branch; never squash or rebase a sync.
- In a Herdr pane, `herdr` on `PATH` is the installed upstream build attached to the live session. Test this repo's builds with `env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH cargo run -- …`, and never run `herdr update` with a Herdra build.

@AGENTS.md

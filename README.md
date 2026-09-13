# Oracle

Command center for every project I run — local dev servers, VPS deployments, repos and
one-off scripts — in a single place.

- Launch and stop local projects in one click, with live logs
- Aggregate CPU and memory per process tree, sampled once a second
- HTTP health checks for anything deployed on the VPS
- Git status at a glance: branch, ahead/behind, dirty worktree
- Tray icon with a real floating interface, not a context menu
- Optional start with Windows

Built with Tauri 2 (Rust) and a dependency-free TypeScript frontend.

## Development

```bash
npm install
npm run tauri dev
```

## Build the installer

```bash
npm run tauri build
```

Produces the installer under `src-tauri/target/release/bundle/nsis/`. Unsigned, and so not
something an existing installation will accept as an update — see below.

## Releases and updating

Oracle updates itself. It asks
`https://github.com/Alicienn/oracle/releases/latest/download/latest.json` at launch, and an
available update puts a line in the title bar rather than a dialog over whatever you opened
the app to do. Clicking it downloads in place; installing is a separate, explicit step,
because the installer replaces Oracle and anything Oracle is running has to be stopped
first — the dialog names those projects before you agree. The check can be turned off in
Settings.

Cutting a release is one command:

```bash
npm run release                              # or: npm run release -- --notes "What changed"
```

It builds, signs, writes the manifest the updater reads, tags the commit, and publishes the
release with `gh`. Add `--dry-run` to do everything except tag and publish.

It refuses to publish a version with no `## <version>` section in
[CHANGELOG.md](CHANGELOG.md), and that section becomes the release notes — written once, so
the notes on GitHub and the ones Oracle shows after updating cannot drift apart. The app
carries the changelog compiled in, and offers the entry for the version it is running the
first time it starts on a new one.

Signing is not optional — the updater refuses any package it cannot verify against the public
key in `src-tauri/tauri.conf.json`, which is what makes an application that downloads and runs
an installer on its own acceptable. The keypair was generated with
`npm run tauri signer generate`; the private half lives at `~/.tauri/oracle.key`, is handed to
the build as a *path* so its contents never reach a shell history, and is never committed.
Set `ORACLE_SIGNING_KEY` to use another location. Losing it means no existing installation can
ever be updated again — every user would have to install the next version by hand, so keep a
copy somewhere safe.

Nothing about this requires a secret on GitHub, and nothing is required of users: the public
key ships inside the application. [The workflow](.github/workflows/release.yml) does the same
job on a runner for the day that is wanted instead, and is manual-only so it cannot race the
local path; it is the only route that needs the key uploaded as a repository secret.

## Documentation

- [Specification](docs/SPEC.md)
- [Implementation plan](docs/TODO.md)

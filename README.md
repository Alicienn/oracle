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

Produces `src-tauri/target/release/bundle/nsis/Oracle_0.1.0_x64-setup.exe`.

## Releases and updating

Oracle updates itself. It asks
`https://github.com/Alicienn/oracle/releases/latest/download/latest.json` at launch, and an
available update puts a line in the title bar rather than a dialog over whatever you opened
the app to do. Clicking it downloads in place; installing is a separate, explicit step,
because the installer replaces Oracle and anything Oracle is running has to be stopped
first — the dialog names those projects before you agree. The check can be turned off in
Settings.

Cutting a release is pushing a tag; [the workflow](.github/workflows/release.yml) builds the
installer, signs it, and publishes it:

```bash
npm version patch   # bump src-tauri/tauri.conf.json to match
git push --follow-tags
```

Two repository secrets have to exist for the build to sign:

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the private key file |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Its password, empty if it has none |

The keypair was generated with `npm run tauri signer generate`. The **public** half is in
`src-tauri/tauri.conf.json` and is what the app verifies against; the private half never
belongs in the repository. Losing it means no existing installation can be updated again —
every user would have to install the next version by hand.

## Documentation

- [Specification](docs/SPEC.md)
- [Implementation plan](docs/TODO.md)

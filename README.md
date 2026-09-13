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

## Documentation

- [Specification](docs/SPEC.md)
- [Implementation plan](docs/TODO.md)

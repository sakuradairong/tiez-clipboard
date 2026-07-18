# Contributing to TieZ

Thanks for helping maintain this fork of TieZ.

## Scope

This repository is currently maintained as a community fork. The short-term goal is stability:

- reproducible builds
- reliable releases
- bug fixes
- maintenance-oriented improvements

Large feature work is welcome, but please discuss it in an issue first so it does not conflict with ongoing stabilization work.

## Development setup

### Prerequisites

- Node.js LTS
- Rust stable toolchain
- Tauri 2 prerequisites for your platform

### Local development

```bash
npm install
npm run tauri:dev
```

### Build check

```bash
npm run build
```

## Pull request guidelines

Please keep pull requests focused and easy to review.

Before opening a PR:

1. Make sure the change has a clear purpose.
2. Keep the diff as small as reasonably possible.
3. Run the most relevant validation you can on your platform.
4. Update docs when behavior, setup, or release expectations change.
5. Add a changelog entry when the change affects users or contributors.

## Good first maintenance areas

- release workflow fixes
- cross-platform build issues
- broken links and outdated project metadata
- dependency updates with low regression risk
- small UI or UX regressions
- contributor and security documentation

## Reporting bugs

Use the repository's bug report template and include:

- app version
- platform and OS version
- reproduction steps
- expected behavior
- actual behavior
- logs or screenshots when available

## Security issues

Please do not post sensitive exploit details in public issues. Review [SECURITY.md](./SECURITY.md) first.

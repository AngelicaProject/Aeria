# Aeria

Aeria helps communities create, maintain, and review translations for FINAL FANTASY XIV.

Aeria brings source management, structured game text editing, translation assistance, review, collaboration, game-update migration, and pack export into one desktop application. Each project represents one target language and can be maintained by one person or by a large community.

## Project status

Aeria is under active development. The repository currently establishes the application foundations, data boundaries, persisted formats, and reliability rules that future features build on.

## Development

The project is a Rust and TypeScript monorepo:

- Rust implements source access, structured string handling, workspace persistence, rebase, search, Git integration, translation assistance, and export.
- Tauri provides the desktop application boundary.
- React, TypeScript, and Vite implement the user interface.
- SQLite is used for rebuildable local indexes, caches, and resumable local job state.
- Git stores project history and enables collaboration.

Start with:

- [Contributing](./CONTRIBUTING.md)
- [Documentation index](./docs/README.md)
- [Product vision](./docs/product/vision.md)
- [Architecture overview](./docs/architecture/overview.md)

## Platforms

Windows is the first supported desktop target. Linux support follows after the Windows application foundation is stable.

## License

Aeria is licensed under the [GNU Affero General Public License v3.0](./LICENSE).

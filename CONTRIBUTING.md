# Contribute to Aeria

Thank you for your interest in contributing to Aeria.

The canonical development guidance lives in [`docs/development/`](./docs/development/). Start with the [contribution workflow](./docs/development/contributing.md), then read the subsystem documentation relevant to your change.

## Before you make a change

- Read the [product principles](./docs/product/principles.md).
- Read the [architecture overview](./docs/architecture/overview.md).
- Check the relevant development and architecture documents for the area you are changing.
- Keep changes focused and include tests for behavior that can regress.
- Update documentation in the same change when behavior, formats, or architectural boundaries change.

## Pull requests

A pull request should explain the problem being solved, the approach taken, and how the change was verified. Keep unrelated changes separate.

Before requesting review, run the checks relevant to the change. The expected project-wide quality gates are documented in [testing](./docs/development/testing.md) and enforced by CI as they become available.

## Security issues

Do not open a public issue containing credentials, private repository data, translation project secrets, or other sensitive information. A dedicated security reporting process will be documented before the first public release.

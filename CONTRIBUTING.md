# Contribute to Aeria

Thank you for your interest in contributing to Aeria.

The canonical development guidance lives in [`docs/development/`](./docs/development/README.md). Start with the [contribution workflow](./docs/development/contributing.md), then read the subsystem documentation relevant to your change.

## Before you make a change

- Read the [product principles](./docs/product/principles.md).
- Read the [architecture overview](./docs/architecture/overview.md).
- Use the [documentation index](./docs/README.md) to find the owning documents for the area you are changing.
- Keep changes focused and include tests for behavior that can regress.
- Update documentation in the same change when behavior, formats, or architectural boundaries change.

## Pull requests

Follow the [pull request standards](./docs/development/pull-requests.md). A pull request should explain the problem being solved, the outcome, the important changes, and how the change was verified.

Before requesting review, run the checks relevant to the change. The current project-wide gates are documented in [continuous integration](./docs/development/ci.md), with testing strategy in [testing](./docs/development/testing.md).

## Security issues

Do not open a public issue containing credentials, private repository data, translation project secrets, or other sensitive information. A dedicated security reporting process will be documented before the first public release.

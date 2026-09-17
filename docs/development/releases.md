# Release channels

Aeria intends to support two channels from early development:

- **Stable**: signed tagged releases intended for ordinary users.
- **Nightly**: latest green build from the main development branch for users who opt into fast updates.

The desktop updater should use the standard signed Tauri update mechanism rather than a custom update engine.

Windows is the first packaging target. A portable Windows artifact is useful alongside the installer. Linux support should follow quickly, initially with a simple broadly usable distribution format before expanding package coverage.

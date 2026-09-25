//! Publishing Harmonia packs from a translation repository.
//!
//! Signing keys stay in the OS credential store; releases are created on
//! GitHub with the maintainer's Git credential; the feed is built by a
//! workflow in the repository. No Aeria service is involved. See
//! `docs/architecture/export.md#publishing`.

#![forbid(unsafe_code)]

mod github;
mod keys;
mod workflow;

pub use github::{
    ExistingRelease, GITHUB_HOST, GitHubClient, GitHubRepository, PublishError, PublishedRelease,
    RELEASE_TAG_PREFIX, ReleaseAsset, ReleaseRequest, pack_asset_name, release_tag,
};
pub use keys::{
    KEYRING_SERVICE, KeyError, KeyringSigningKeyStore, MemorySigningKeyStore, SigningKeyStore,
    SigningSecret,
};
pub use workflow::{
    FEED_WORKFLOW, FEED_WORKFLOW_PATH, WorkflowState, feed_workflow_state, install_feed_workflow,
};

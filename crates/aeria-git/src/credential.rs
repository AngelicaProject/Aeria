//! HTTPS host credentials from the user's Git credential helper.
//!
//! Forge adapters (such as GitHub release publishing) reuse the sign-in Git
//! already uses for the remote, so Aeria never stores a forge token itself.
//! With the bundled `MinGit` this is Git Credential Manager, which signs in
//! through the browser when nothing is stored yet.

use std::fmt::{self, Write as _};
use std::path::Path;

use crate::GitError;
use crate::process::{GitExecutable, require_success};

/// A username and secret returned by `git credential fill`. Its `Debug`
/// output is redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct HostCredential {
    pub username: String,
    secret: String,
}

impl HostCredential {
    /// Returns the secret for sending it to the host it was issued for.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.secret
    }
}

impl fmt::Debug for HostCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostCredential")
            .field("username", &self.username)
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl GitExecutable {
    /// Asks the credential helper for an HTTPS credential for `host`.
    ///
    /// # Errors
    /// Returns [`GitError::InvalidInput`] for a host Git cannot receive
    /// safely, and [`GitError::CommandFailed`] when no credential is available
    /// (for example, the sign-in was cancelled).
    pub fn credential_fill(&self, cwd: &Path, host: &str) -> Result<HostCredential, GitError> {
        let args = ["credential", "fill"];
        let output = self.output_with_input(cwd, &args, request(host, None)?)?;
        require_success(&args, &output)?;
        let text = String::from_utf8_lossy(&output.stdout);
        let mut username = None;
        let mut secret = None;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("username=") {
                username = Some(value.to_owned());
            } else if let Some(value) = line.strip_prefix("password=") {
                secret = Some(value.to_owned());
            }
        }
        match (username, secret) {
            (Some(username), Some(secret)) if !secret.is_empty() => {
                Ok(HostCredential { username, secret })
            }
            _ => Err(GitError::Parse {
                message: "git credential fill returned no password".to_owned(),
            }),
        }
    }

    /// Tells the helper that `credential` worked, so it keeps it.
    ///
    /// # Errors
    /// Returns an error when Git cannot be run.
    pub fn credential_approve(
        &self,
        cwd: &Path,
        host: &str,
        credential: &HostCredential,
    ) -> Result<(), GitError> {
        self.credential_report(cwd, "approve", host, credential)
    }

    /// Tells the helper that `credential` was refused, so it forgets it.
    ///
    /// # Errors
    /// Returns an error when Git cannot be run.
    pub fn credential_reject(
        &self,
        cwd: &Path,
        host: &str,
        credential: &HostCredential,
    ) -> Result<(), GitError> {
        self.credential_report(cwd, "reject", host, credential)
    }

    fn credential_report(
        &self,
        cwd: &Path,
        action: &str,
        host: &str,
        credential: &HostCredential,
    ) -> Result<(), GitError> {
        let args = ["credential", action];
        let output = self.output_with_input(cwd, &args, request(host, Some(credential))?)?;
        require_success(&args, &output)
    }
}

fn request(host: &str, credential: Option<&HostCredential>) -> Result<Vec<u8>, GitError> {
    let safe = |value: &str| !value.contains(['\n', '\r', '\0']);
    if host.is_empty() || !safe(host) || host.contains(['/', '@']) {
        return Err(GitError::InvalidInput {
            field: "credential host",
            reason: "must be a plain host name".to_owned(),
        });
    }
    let mut text = format!("protocol=https\nhost={host}\n");
    if let Some(credential) = credential {
        if !safe(&credential.username) || !safe(&credential.secret) {
            return Err(GitError::InvalidInput {
                field: "credential",
                reason: "contains a line break".to_owned(),
            });
        }
        let _ = write!(
            text,
            "username={}\npassword={}\n",
            credential.username, credential.secret
        );
    }
    text.push('\n');
    Ok(text.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_name_only_the_host() {
        assert_eq!(
            request("github.com", None).expect("request"),
            b"protocol=https\nhost=github.com\n\n"
        );
        for host in ["", "github.com\nhost=evil", "user@github.com", "a/b"] {
            assert!(request(host, None).is_err(), "{host:?}");
        }
    }

    #[test]
    fn debug_output_hides_the_secret() {
        let credential = HostCredential {
            username: "someone".to_owned(),
            secret: "gho_secret".to_owned(),
        };
        assert!(!format!("{credential:?}").contains("gho_secret"));
    }

    #[test]
    fn fill_reads_the_helper_answer() {
        let temp = tempfile::tempdir().expect("temp");
        let helper = temp.path().join("helper.sh");
        std::fs::write(
            &helper,
            "#!/bin/sh\ncat >/dev/null\necho username=someone\necho password=token-1\n",
        )
        .expect("helper");
        let git = GitExecutable::system()
            .with_env("GIT_CONFIG_GLOBAL", temp.path().join("global"))
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
            .with_env("GIT_CONFIG_COUNT", "1")
            .with_env("GIT_CONFIG_KEY_0", "credential.helper")
            .with_env(
                "GIT_CONFIG_VALUE_0",
                format!("!sh '{}'", helper.to_string_lossy().replace('\\', "/")),
            );
        let credential = git
            .credential_fill(temp.path(), "github.com")
            .expect("credential");
        assert_eq!(credential.username, "someone");
        assert_eq!(credential.expose(), "token-1");
        git.credential_approve(temp.path(), "github.com", &credential)
            .expect("approve");
    }
}

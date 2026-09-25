//! Web pages for Angelica.
//!
//! `fetch_url` runs here instead of in a blocking tool worker because it
//! waits on the network. A link to a domain that is not allowed becomes a
//! proposal: allowing it adds the domain to the AI settings and tells
//! Angelica to continue.

use aeria_ai::conversation::{ConversationStore, ProposalRecord, ProposalStatus};
use aeria_ai::guidance::{ProjectFile, read_project_file};
use aeria_ai::tools::{ToolOutput, UnitState, bounded_json};
use aeria_ai::web::{FetchArgs, FetchError, WebPolicy, normalize_domain};
use serde_json::json;
use tauri::Manager;

use crate::ai::settings_store;
use crate::angelica::{announce_proposals, now_unix_ms, repository_root};
use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::state::DesktopState;

fn error_output(message: impl std::fmt::Display) -> ToolOutput {
    ToolOutput {
        content: json!({ "error": message.to_string() }).to_string(),
        is_error: true,
    }
}

/// The allowed domains and the guidance's links.
fn policy(app: &tauri::AppHandle) -> Result<WebPolicy, CommandError> {
    let settings = settings_store(app)?.load()?;
    let guidance = read_project_file(&repository_root(app)?, ProjectFile::Guidance)
        .ok()
        .flatten();
    Ok(WebPolicy::new(&settings.web_domains, guidance.as_deref()))
}

/// Records a request to read a domain, or returns the pending one.
fn request_domain(
    app: &tauri::AppHandle,
    store: &ConversationStore,
    conversation_id: &str,
    url: &str,
    domain: &str,
) -> Result<String, CommandError> {
    let state = app.state::<DesktopState>();
    let _guard = state.lock_proposals()?;
    let mut records = store.load_proposals(conversation_id)?;
    if let Some(pending) = records.iter().find(|record| {
        record.status == ProposalStatus::Pending && record.web.as_deref() == Some(domain)
    }) {
        return Ok(pending.id.clone());
    }
    let id = aeria_ai::ProviderConfig::new_id();
    records.push(ProposalRecord {
        id: id.clone(),
        file: None,
        job: None,
        web: Some(domain.to_owned()),
        review: None,
        location: None,
        source: String::new(),
        target: url.to_owned(),
        expected: UnitState {
            target: None,
            review_state: None,
        },
        status: ProposalStatus::Pending,
        message: None,
        created_at_unix_ms: now_unix_ms(),
    });
    store.save_proposals(conversation_id, &records)?;
    announce_proposals(app, conversation_id);
    Ok(id)
}

/// Runs `fetch_url` for a conversation.
pub(crate) async fn fetch_tool(
    app: &tauri::AppHandle,
    store: &ConversationStore,
    conversation_id: &str,
    arguments: &str,
) -> ToolOutput {
    let (url, offset, max_chars) = match FetchArgs::parse(arguments) {
        Ok(parsed) => parsed,
        Err(error) => return error_output(error.0),
    };
    let policy_app = app.clone();
    let policy = match run_blocking(move || policy(&policy_app)).await {
        Ok(policy) => policy,
        Err(error) => return error_output(error.message),
    };
    let client = match app.state::<DesktopState>().web_client() {
        Ok(client) => client,
        Err(error) => return error_output(error.message),
    };
    match client.fetch(url, &policy, offset, max_chars).await {
        Ok(page) => ToolOutput {
            content: bounded_json(&json!(page)),
            is_error: false,
        },
        Err(FetchError::NotPermitted { url, domain }) => {
            let request_app = app.clone();
            let request_store = store.clone();
            let id = conversation_id.to_owned();
            let link = url.to_string();
            let requested = domain.clone();
            match run_blocking(move || {
                request_domain(&request_app, &request_store, &id, &link, &requested)
            })
            .await
            {
                Ok(proposal_id) => ToolOutput {
                    content: json!({
                        "status": "awaitingApproval",
                        "domain": domain,
                        "proposalId": proposal_id,
                        "message": "The user was asked to allow this domain. Tell them why you need it and wait; you will be told when it is allowed.",
                    })
                    .to_string(),
                    is_error: false,
                },
                Err(error) => error_output(error.message),
            }
        }
        Err(error) => error_output(error),
    }
}

/// Adds a domain to the allowed domains.
pub(crate) fn allow_domain(app: &tauri::AppHandle, domain: &str) -> Result<(), CommandError> {
    let domain = normalize_domain(domain)
        .map_err(|message| CommandError::new("aiInvalidSettings", message))?;
    settings_store(app)?.update(|settings| {
        if !settings.web_domains.contains(&domain) {
            settings.web_domains.push(domain);
            settings.web_domains.sort();
        }
        Ok(())
    })?;
    Ok(())
}

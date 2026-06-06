//! Entra (Azure AD) credential acquisition for Postgres, behind the `entra` feature.
//! Uses the GA Azure SDK. The token is used as the Postgres password.

use anyhow::{Context, Result};
use azure_core::credentials::TokenCredential;

/// Default scope for Azure Database for PostgreSQL Flexible Server with Entra auth.
pub const DEFAULT_ENTRA_SCOPE: &str = "https://ossrdbms-aad.database.windows.net/.default";

/// Fetch an Entra access token for `scope` using the selected credential mode:
/// - "auto" / "developer": developer tools chain (Azure CLI / azd login) — local dev.
/// - "cli": Azure CLI credential only.
/// - "managed": managed identity — for devboxes/VMs/servers with an assigned identity.
pub async fn entra_token(scope: &str, mode: &str) -> Result<String> {
    let scopes = [scope];
    let token = match mode {
        "cli" => {
            let cred = azure_identity::AzureCliCredential::new(None)
                .context("creating Azure CLI credential")?;
            cred.get_token(&scopes, None).await
        }
        "managed" => {
            let cred = azure_identity::ManagedIdentityCredential::new(None)
                .context("creating managed identity credential")?;
            cred.get_token(&scopes, None).await
        }
        _ => {
            let cred = azure_identity::DeveloperToolsCredential::new(None)
                .context("creating developer tools credential")?;
            cred.get_token(&scopes, None).await
        }
    }
    .context("acquiring Entra access token")?;
    Ok(token.token.secret().to_string())
}

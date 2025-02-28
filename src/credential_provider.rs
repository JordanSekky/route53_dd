use std::time::SystemTime;

// If you are loading credentials dynamically, you can provide your own implementation of
// [`ProvideCredentials`](crate::provider::ProvideCredentials). Generally, this is best done by
// defining an inherent `async fn` on your structure, then calling that method directly from
// the trait implementation.
// ```rust
use aws_credential_types::{provider::ProvideCredentials, Credentials};

use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct AwsCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    expires_after: Option<SystemTime>,
}

impl std::fmt::Debug for AwsCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let blank = "********";
        f.debug_struct("AwsCredentials")
            .field("access_key_id", &blank)
            .field("secret_access_key", &blank)
            .field("session_token", &blank)
            .field("expires_after", &self.expires_after)
            .finish()
    }
}
impl std::fmt::Display for AwsCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let blank = "********";
        f.debug_struct("AwsCredentials")
            .field("access_key_id", &blank)
            .field("secret_access_key", &blank)
            .field("session_token", &blank)
            .field("expires_after", &self.expires_after)
            .finish()
    }
}

impl ProvideCredentials for AwsCredentials {
    fn provide_credentials<'a>(
        &'a self,
    ) -> aws_credential_types::provider::future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        aws_credential_types::provider::future::ProvideCredentials::ready(Ok(Credentials::new(
            self.access_key_id.clone(),
            self.secret_access_key.clone(),
            self.session_token.clone(),
            self.expires_after,
            "ConfigFileProvider",
        )))
    }
}

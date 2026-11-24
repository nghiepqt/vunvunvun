#![forbid(unsafe_code)]
#![allow(clippy::disallowed_types)]

use ef_protocol::{GatewayChatRequest, GatewayChatResponse};
use thiserror::Error;

/// Capability token issued exclusively by `ef_ai::Router` to authorize model transport.
#[derive(Debug, Clone, Copy)]
pub struct RouterDispatchToken {
    _private: (),
}

impl RouterDispatchToken {
    /// Intended for `ef_ai::Router` only.
    #[doc(hidden)]
    #[must_use]
    pub fn issue_for_router() -> Self {
        Self { _private: () }
    }
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
}

/// Only client crate permitted to hold a `reqwest::Client`.
#[derive(Debug, Clone)]
pub struct TransportClient {
    client: reqwest::Client,
}

impl TransportClient {
    pub fn new() -> Result<Self, TransportError> {
        let client = reqwest::Client::builder().build()?;
        Ok(Self { client })
    }

    pub async fn post_gateway_chat(
        &self,
        _token: RouterDispatchToken,
        endpoint: &str,
        request: &GatewayChatRequest,
    ) -> Result<GatewayChatResponse, TransportError> {
        let response = self
            .client
            .post(endpoint)
            .json(request)
            .send()
            .await?
            .error_for_status()?
            .json::<GatewayChatResponse>()
            .await?;
        Ok(response)
    }
}

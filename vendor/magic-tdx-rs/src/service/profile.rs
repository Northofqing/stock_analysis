//! Stable F10/profile service facade.

use crate::net::client::TdxHqClient;
use crate::profile::types::{F10Category, F10Content, F10Data};
use crate::{ProfileClient, TdxError};
use std::sync::Mutex;

/// Thread-safe facade for F10 categories and content.
pub struct ProfileService {
    client: Mutex<TdxHqClient>,
}
impl ProfileService {
    /// Creates a profile service using the supplied endpoint.
    pub fn new(ip: &str, port: u16, timeout: f64) -> Self {
        let client = TdxHqClient::new();
        client.set_servers(&[("profile", ip, port)]);
        client.set_connect_timeout(timeout);
        Self {
            client: Mutex::new(client),
        }
    }
    /// Returns F10 categories for a market/code pair.
    pub fn categories(&self, market: u8, code: &str) -> Result<Vec<F10Category>, TdxError> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| TdxError::InvalidData("profile client lock poisoned".into()))?;
        ProfileClient::new(&mut client).get_category(market, code)
    }
    /// Returns F10 categories with market inferred from the code prefix.
    pub fn categories_auto(&self, code: &str) -> Result<Vec<F10Category>, TdxError> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| TdxError::InvalidData("profile client lock poisoned".into()))?;
        ProfileClient::new(&mut client).get_category_auto(code)
    }
    /// Fetches content for a previously returned category descriptor.
    pub fn content(
        &self,
        market: u8,
        code: &str,
        category: &F10Category,
    ) -> Result<F10Content, TdxError> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| TdxError::InvalidData("profile client lock poisoned".into()))?;
        ProfileClient::new(&mut client).get_content(market, code, category)
    }
    /// Returns a named F10 section.
    pub fn content_by_name(
        &self,
        market: u8,
        code: &str,
        name: &str,
    ) -> Result<F10Content, TdxError> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| TdxError::InvalidData("profile client lock poisoned".into()))?;
        ProfileClient::new(&mut client).get_content_by_name(market, code, name)
    }
    /// Returns all F10 sections.
    pub fn all_contents(&self, market: u8, code: &str) -> Result<Vec<F10Content>, TdxError> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| TdxError::InvalidData("profile client lock poisoned".into()))?;
        ProfileClient::new(&mut client).get_all_contents(market, code)
    }
    /// Returns the complete decoded F10 payload.
    pub fn all_data(&self, market: u8, code: &str) -> Result<F10Data, TdxError> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| TdxError::InvalidData("profile client lock poisoned".into()))?;
        ProfileClient::new(&mut client).get_all_data(market, code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_facade_rejects_invalid_identity_before_transport() {
        let service = ProfileService::new("127.0.0.1", 1, 0.01);
        let category = F10Category::new("公司概况".into(), "600001.txt".into(), 10, 20);
        assert!(service.categories(2, "600001").is_err());
        assert!(service.categories_auto("bad").is_err());
        assert!(service.content(2, "600001", &category).is_err());
        assert!(service.content_by_name(2, "600001", "公司概况").is_err());
        assert!(service.all_contents(2, "600001").is_err());
        assert!(service.all_data(2, "600001").is_err());
    }
}

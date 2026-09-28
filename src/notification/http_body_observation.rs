use sha2::{Digest, Sha256};

/// A bounded summary of request entity bodies submitted to Reqwest in order.
/// Headers, framing, redirects, TLS, and remote receipt are outside this scope.
pub(crate) struct HttpBodyObservation {
    request_count: usize,
    total_body_bytes: usize,
    complete: bool,
    sequence_hasher: Sha256,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpBodySummary {
    request_count: usize,
    total_body_bytes: usize,
    sequence_sha256: String,
}

impl HttpBodySummary {
    pub(crate) const fn request_count(&self) -> usize {
        self.request_count
    }

    pub(crate) const fn total_body_bytes(&self) -> usize {
        self.total_body_bytes
    }

    pub(crate) fn sequence_sha256(&self) -> &str {
        &self.sequence_sha256
    }
}

impl HttpBodyObservation {
    pub(crate) fn new(domain: &[u8]) -> Self {
        let mut sequence_hasher = Sha256::new();
        sequence_hasher.update(domain);
        Self {
            request_count: 0,
            total_body_bytes: 0,
            complete: true,
            sequence_hasher,
        }
    }

    pub(crate) fn observe_request(&mut self, request: &reqwest::Request) {
        let Some(count) = self.request_count.checked_add(1) else {
            self.complete = false;
            return;
        };
        self.request_count = count;
        let Some(body) = request.body().and_then(reqwest::Body::as_bytes) else {
            self.complete = false;
            return;
        };
        let Some(total) = self.total_body_bytes.checked_add(body.len()) else {
            self.complete = false;
            return;
        };
        self.total_body_bytes = total;
        self.sequence_hasher
            .update((body.len() as u64).to_be_bytes());
        self.sequence_hasher.update(body);
    }

    /// None means no request was built or at least one body was opaque.
    pub(crate) fn finish(self) -> Option<HttpBodySummary> {
        (self.complete && self.request_count > 0).then(|| HttpBodySummary {
            request_count: self.request_count,
            total_body_bytes: self.total_body_bytes,
            sequence_sha256: format!("{:x}", self.sequence_hasher.finalize()),
        })
    }
}

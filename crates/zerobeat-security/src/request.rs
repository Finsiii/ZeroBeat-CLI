use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestToSign {
    pub method: String,
    pub host: String,
    pub canonical_path: String,
    pub raw_query: String,
    pub body: Vec<u8>,
}

impl RequestToSign {
    pub fn get(
        host: impl Into<String>,
        canonical_path: impl Into<String>,
        raw_query: impl Into<String>,
    ) -> Self {
        Self {
            method: "GET".into(),
            host: host.into(),
            canonical_path: canonical_path.into(),
            raw_query: raw_query.into(),
            body: Vec::new(),
        }
    }

    pub fn with_body(
        method: impl Into<String>,
        host: impl Into<String>,
        canonical_path: impl Into<String>,
        raw_query: impl Into<String>,
        body: Vec<u8>,
    ) -> Self {
        Self {
            method: method.into(),
            host: host.into(),
            canonical_path: canonical_path.into(),
            raw_query: raw_query.into(),
            body,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedRequest {
    pub canonical: String,
    pub headers: BTreeMap<String, String>,
}

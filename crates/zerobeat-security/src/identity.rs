use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::{
    ecdsa::{Signature, SigningKey, VerifyingKey, signature::Signer},
    pkcs8::EncodePublicKey,
};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

use crate::{RequestToSign, SecurityError, SignedRequest};

const PLATFORM: &str = "desktop";
const RISK_SNAPSHOT: &str = "v=1;root=0;debug=0;hook=0;emu=0;tamper=0";

pub struct DeviceIdentity {
    pub(crate) install_id: String,
    pub(crate) app_version: String,
    pub(crate) signing_key: SigningKey,
    pub(crate) device_id: Option<String>,
    pub(crate) key_version: u32,
    pub(crate) counter: u64,
}

impl DeviceIdentity {
    pub fn generate(app_version: impl Into<String>) -> Result<Self, SecurityError> {
        let mut install_id = [0_u8; 18];
        OsRng.fill_bytes(&mut install_id);
        Ok(Self {
            install_id: URL_SAFE_NO_PAD.encode(install_id),
            app_version: app_version.into(),
            signing_key: SigningKey::random(&mut OsRng),
            device_id: None,
            key_version: 1,
            counter: 0,
        })
    }

    pub fn install_id(&self) -> &str {
        &self.install_id
    }

    pub fn app_version(&self) -> &str {
        &self.app_version
    }

    pub fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }

    pub fn verifying_key(&self) -> &VerifyingKey {
        self.signing_key.verifying_key()
    }

    pub fn public_key_spki_base64(&self) -> Result<String, SecurityError> {
        let document = self
            .verifying_key()
            .to_public_key_der()
            .map_err(|_| SecurityError::InvalidKey)?;
        Ok(URL_SAFE_NO_PAD.encode(document.as_bytes()))
    }

    pub fn sign_challenge(&self, challenge: &str) -> Result<String, SecurityError> {
        let challenge = URL_SAFE_NO_PAD
            .decode(challenge.trim_end_matches('='))
            .map_err(|_| SecurityError::InvalidChallenge)?;
        let signature: Signature = self.signing_key.sign(&challenge);
        Ok(URL_SAFE_NO_PAD.encode(signature.to_der().as_bytes()))
    }

    pub fn bind_credential(&mut self, device_id: impl Into<String>, key_version: u32) {
        self.device_id = Some(device_id.into());
        self.key_version = key_version.max(1);
        self.counter = 0;
    }

    pub fn sign_request(
        &mut self,
        request: RequestToSign,
        request_time_ms: i64,
    ) -> Result<SignedRequest, SecurityError> {
        let device_id = self
            .device_id
            .as_deref()
            .ok_or(SecurityError::NotProvisioned)?;
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or(SecurityError::CounterOverflow)?;
        let mut nonce = [0_u8; 18];
        OsRng.fill_bytes(&mut nonce);
        let nonce = URL_SAFE_NO_PAD.encode(nonce);
        let body_hash = if request.body.is_empty() {
            String::new()
        } else {
            hex::encode(Sha256::digest(&request.body))
        };
        let method = request.method.to_uppercase();
        let host = request.host.to_lowercase();
        let canonical = [
            "zerobeat-v5".to_owned(),
            method,
            format!("https://{host}"),
            request.canonical_path.clone(),
            request.raw_query.clone(),
            device_id.to_owned(),
            PLATFORM.to_owned(),
            self.app_version.clone(),
            self.key_version.to_string(),
            request_time_ms.to_string(),
            nonce.clone(),
            self.counter.to_string(),
            RISK_SNAPSHOT.to_owned(),
            body_hash.clone(),
        ]
        .join("\n");
        let signature: Signature = self.signing_key.sign(canonical.as_bytes());
        let mut headers = BTreeMap::new();
        for (name, value) in [
            ("X-ZeroBeat-Signature-Version", "v5".to_owned()),
            ("X-ZeroBeat-Device-ID", device_id.to_owned()),
            ("X-ZeroBeat-Key-Version", self.key_version.to_string()),
            ("X-ZeroBeat-Platform", PLATFORM.to_owned()),
            ("X-ZeroBeat-App-Version", self.app_version.clone()),
            ("X-ZeroBeat-Canonical-Host", host),
            ("X-ZeroBeat-Canonical-Path", request.canonical_path),
            ("X-ZeroBeat-Request-Time", request_time_ms.to_string()),
            ("X-ZeroBeat-Request-Nonce", nonce),
            ("X-ZeroBeat-Request-Counter", self.counter.to_string()),
            ("X-ZeroBeat-Client-Risk", RISK_SNAPSHOT.to_owned()),
            ("X-ZeroBeat-Body-SHA256", body_hash),
            (
                "X-ZeroBeat-Request-Signature",
                URL_SAFE_NO_PAD.encode(signature.to_der().as_bytes()),
            ),
        ] {
            headers.insert(name.to_owned(), value);
        }
        Ok(SignedRequest { canonical, headers })
    }
}

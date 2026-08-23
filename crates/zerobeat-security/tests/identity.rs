use std::os::unix::fs::PermissionsExt;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{Signature, signature::Verifier};
use tempfile::tempdir;
use zerobeat_security::{DeviceIdentity, IdentityStore, RequestToSign};

#[test]
fn identity_survives_reopen_with_private_permissions() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("device.identity");

    let first = IdentityStore::load_or_create(&path, "cli/0.1.0+1").unwrap();
    let install_id = first.install_id().to_owned();
    let public_key = first.public_key_spki_base64().unwrap();
    drop(first);

    let reopened = IdentityStore::load_or_create(&path, "cli/0.1.0+1").unwrap();
    assert_eq!(reopened.install_id(), install_id);
    assert_eq!(reopened.public_key_spki_base64().unwrap(), public_key);
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn challenge_proof_verifies_against_public_key() {
    let identity = DeviceIdentity::generate("cli/0.1.0+1").unwrap();
    let challenge = URL_SAFE_NO_PAD.encode(b"single-use-provision-challenge");
    let signature = identity.sign_challenge(&challenge).unwrap();

    let signature = URL_SAFE_NO_PAD.decode(signature).unwrap();
    let signature = Signature::from_der(&signature).unwrap();
    identity
        .verifying_key()
        .verify(b"single-use-provision-challenge", &signature)
        .unwrap();
}

#[test]
fn v5_request_signature_binds_every_canonical_field() {
    let mut identity = DeviceIdentity::generate("cli/0.1.0+1").unwrap();
    identity.bind_credential("device-123", 1);
    let request = RequestToSign::get(
        "api.zerobits.tech",
        "/music/v1/app/search/songs",
        "q=tampar&limit=20",
    );

    let signed = identity.sign_request(request, 1_782_342_000_000).unwrap();
    let signature = URL_SAFE_NO_PAD
        .decode(&signed.headers["X-ZeroBeat-Request-Signature"])
        .unwrap();
    let signature = Signature::from_der(&signature).unwrap();

    identity
        .verifying_key()
        .verify(signed.canonical.as_bytes(), &signature)
        .unwrap();
    assert_eq!(signed.headers["X-ZeroBeat-Signature-Version"], "v5");
    assert_eq!(signed.headers["X-ZeroBeat-Request-Counter"], "1");
    assert!(signed.canonical.contains("\nq=tampar&limit=20\n"));
    assert!(signed.canonical.contains("\ndevice-123\n"));
}

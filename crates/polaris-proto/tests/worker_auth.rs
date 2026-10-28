//! Parent-issued worker bearer credential contract.

use std::io;

use polaris_proto::worker_auth::{WorkerAuthenticator, WorkerCredential};
use secrecy::SecretString;
use tonic::Request;
use tonic::metadata::{MetadataKey, MetadataValue};
use tonic::service::Interceptor;
use zeroize::Zeroizing;

const FIRST_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SECOND_TOKEN: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

fn token_key() -> MetadataKey<tonic::metadata::Ascii> {
    MetadataKey::from_static("x-polaris-worker-token")
}

fn load_credential(value: &str) -> Result<WorkerCredential, Box<dyn std::error::Error>> {
    Ok(WorkerCredential::from_secret(SecretString::from(
        value.to_owned(),
    ))?)
}

fn rejected(request: Request<()>, auth: &WorkerAuthenticator) -> bool {
    let mut auth = auth.clone();
    matches!(auth.call(request), Err(status) if status.code() == tonic::Code::Unauthenticated)
}

#[test]
fn generated_credentials_are_fixed_lowercase_hex_and_round_trip()
-> Result<(), Box<dyn std::error::Error>> {
    struct ZeroizingWriter<'a>(&'a mut Zeroizing<Vec<u8>>);

    impl io::Write for ZeroizingWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let credential = WorkerCredential::generate()?;
    let mut token = Zeroizing::new(Vec::new());
    credential.write_to(&mut ZeroizingWriter(&mut token))?;
    assert_eq!(token.len(), 64);
    assert!(
        token
            .iter()
            .all(|byte| { matches!(*byte, b'0'..=b'9' | b'a'..=b'f') })
    );
    let exported = std::str::from_utf8(token.as_slice())?;
    let reloaded = load_credential(exported)?;
    let authenticator = WorkerAuthenticator::new(&reloaded);
    let request = credential.authorize_request(Request::new(()))?;
    let mut authenticator = authenticator;
    assert!(authenticator.call(request).is_ok());
    Ok(())
}

#[test]
fn valid_worker_token_authorizes_sensitive_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let credential = load_credential(FIRST_TOKEN)?;
    let mut authenticator = WorkerAuthenticator::new(&credential);
    let authorized = credential.authorize_request(Request::new(()))?;
    let token = authorized
        .metadata()
        .get(token_key())
        .ok_or("authorized token missing")?;
    assert!(token.is_sensitive());
    let accepted = authenticator.call(authorized)?;
    assert!(accepted.metadata().get(token_key()).is_none());
    assert_eq!(accepted.into_inner(), ());
    Ok(())
}

#[test]
fn client_authorization_replaces_existing_duplicate_header_values()
-> Result<(), Box<dyn std::error::Error>> {
    let credential = load_credential(FIRST_TOKEN)?;
    let mut request = Request::new(());
    request
        .metadata_mut()
        .append(token_key(), MetadataValue::try_from(SECOND_TOKEN)?);
    request
        .metadata_mut()
        .append(token_key(), MetadataValue::try_from(SECOND_TOKEN)?);

    let authorized = credential.authorize_request(request)?;
    let values = authorized.metadata().get_all(token_key()).iter().count();
    assert_eq!(values, 1);
    assert_eq!(
        authorized
            .metadata()
            .get(token_key())
            .map(MetadataValue::as_bytes),
        Some(FIRST_TOKEN.as_bytes())
    );
    Ok(())
}

#[test]
fn missing_malformed_duplicate_and_wrong_tokens_are_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    for invalid in ["", "ABCDEF0123456789", FIRST_TOKEN.trim_end_matches('f')] {
        assert!(WorkerCredential::from_secret(SecretString::from(invalid.to_owned())).is_err());
    }
    let credential = load_credential(FIRST_TOKEN)?;
    let authenticator = WorkerAuthenticator::new(&credential);
    assert!(rejected(Request::new(()), &authenticator));

    for malformed in ["", "not-hex", "ABCDEF", "0123456789abcdef"] {
        let mut request = Request::new(());
        let value = MetadataValue::try_from(malformed)?;
        request.metadata_mut().append(token_key(), value);
        assert!(rejected(request, &authenticator));
    }

    let mut duplicate = Request::new(());
    duplicate
        .metadata_mut()
        .append(token_key(), MetadataValue::try_from(FIRST_TOKEN)?);
    duplicate
        .metadata_mut()
        .append(token_key(), MetadataValue::try_from(FIRST_TOKEN)?);
    assert!(rejected(duplicate, &authenticator));

    let mut wrong = Request::new(());
    wrong
        .metadata_mut()
        .append(token_key(), MetadataValue::try_from(SECOND_TOKEN)?);
    let status = {
        let mut authenticator = authenticator.clone();
        authenticator
            .call(wrong)
            .err()
            .ok_or("wrong token accepted")?
    };
    assert_eq!(status.code(), tonic::Code::Unauthenticated);
    assert!(!status.message().contains(FIRST_TOKEN));
    assert!(!status.message().contains(SECOND_TOKEN));
    Ok(())
}

#[test]
fn independently_issued_credentials_do_not_authenticate_each_other()
-> Result<(), Box<dyn std::error::Error>> {
    let first = load_credential(FIRST_TOKEN)?;
    let second = load_credential(SECOND_TOKEN)?;
    let authenticator = WorkerAuthenticator::new(&first);
    let request = second.authorize_request(Request::new(()))?;
    assert!(rejected(request, &authenticator));
    Ok(())
}

#[test]
fn bootstrap_copy_preserves_authorization_without_exporting_plain_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    let credential = WorkerCredential::generate()?;
    let receiver = WorkerCredential::from_secret(credential.bootstrap_secret())?;
    let mut auth = WorkerAuthenticator::new(&receiver);
    let request = credential.authorize_request(Request::new(()))?;
    assert!(auth.call(request).is_ok());
    Ok(())
}

#[test]
fn credential_export_propagates_writer_errors() -> Result<(), Box<dyn std::error::Error>> {
    struct RejectWriter;

    impl io::Write for RejectWriter {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("injected writer failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let credential = load_credential(FIRST_TOKEN)?;
    assert!(credential.write_to(&mut RejectWriter).is_err());
    Ok(())
}

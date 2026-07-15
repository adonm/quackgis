// SPDX-License-Identifier: Apache-2.0
//! Shared authenticated transport contract for the QuackGIS iroh edge.

use std::collections::HashSet;

use iroh::{EndpointAddr, EndpointId, PublicKey, RelayUrl, SecretKey, Signature};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

pub mod compression;
pub mod config;
pub mod runtime;

pub const CONTROL_ALPN: &[u8] = b"quackgis/control/1";
pub const EDGE_ALPN: &[u8] = b"quackgis/edge/1";
pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_CONTROL_MESSAGE_BYTES: usize = 16 * 1024;
pub const MAX_LEASE_TTL_SECONDS: u64 = 5 * 60;
pub const MAX_CLOCK_SKEW_SECONDS: u64 = 30;
pub const MAX_LOGIN_ROLE_BYTES: usize = 63;

const LEASE_DOMAIN: &[u8] = b"quackgis/access-lease/1\0";
const LEASE_REQUEST_DOMAIN: &[u8] = b"quackgis/lease-request/1\0";
const EDGE_PROOF_DOMAIN: &[u8] = b"quackgis/edge-proof/1\0";

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("unsupported protocol version")]
    UnsupportedVersion,
    #[error("LOGIN role is empty, oversized, or contains a forbidden character")]
    InvalidLoginRole,
    #[error("access lease time range is invalid")]
    InvalidLeaseTime,
    #[error("access lease lifetime exceeds the configured protocol maximum")]
    LeaseTooLong,
    #[error("access lease is not valid yet")]
    LeaseNotYetValid,
    #[error("access lease has expired")]
    LeaseExpired,
    #[error("access lease names a different worker")]
    WrongWorker,
    #[error("access lease names a different client credential")]
    WrongCredential,
    #[error("transport endpoint does not match the credential proof")]
    WrongTransportEndpoint,
    #[error("access lease signature is invalid")]
    InvalidLeaseSignature,
    #[error("credential-key proof is invalid")]
    InvalidCredentialProof,
    #[error("challenge does not match the worker challenge")]
    WrongChallenge,
    #[error("application protocol is absent, duplicated, or unsupported")]
    InvalidApplicationProtocols,
    #[error("application protocol is not permitted by the access lease")]
    ProtocolNotPermitted,
    #[error("mandatory uncompressed transport support is absent")]
    MissingUncompressedCodec,
    #[error("compression selection was not offered")]
    CompressionNotOffered,
    #[error("explicit relay configuration must contain at least one URL")]
    EmptyRelayList,
    #[error("relay configuration contains a duplicate URL")]
    DuplicateRelay,
    #[error("invalid relay URL: {0}")]
    InvalidRelayUrl(String),
    #[error("control message exceeds the {MAX_CONTROL_MESSAGE_BYTES}-byte limit")]
    ControlMessageTooLarge,
    #[error("invalid control message: {0}")]
    InvalidControlMessage(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationProtocol {
    Pgwire,
    Cancellation,
    Http,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionCodec {
    None,
    Lz4Block,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionPolicy {
    #[default]
    Off,
    Auto,
}

impl CompressionPolicy {
    pub fn offers(self) -> Vec<CompressionCodec> {
        match self {
            Self::Off => vec![CompressionCodec::None],
            Self::Auto => vec![CompressionCodec::None, CompressionCodec::Lz4Block],
        }
    }

    pub fn select(self, offers: &[CompressionCodec]) -> Result<CompressionCodec, ProtocolError> {
        validate_compression_offers(offers)?;
        Ok(match self {
            Self::Auto if offers.contains(&CompressionCodec::Lz4Block) => {
                CompressionCodec::Lz4Block
            }
            _ => CompressionCodec::None,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccessLeaseClaims {
    pub version: u8,
    pub credential: PublicKey,
    pub login_role: String,
    pub worker: EndpointAddr,
    pub assignment_generation: u64,
    pub security_epoch: u64,
    pub configuration_epoch: u64,
    pub issued_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
    pub protocols: Vec<ApplicationProtocol>,
}

impl AccessLeaseClaims {
    pub fn new(
        credential: PublicKey,
        login_role: impl Into<String>,
        worker: EndpointAddr,
        assignment_generation: u64,
        issued_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
        protocols: Vec<ApplicationProtocol>,
    ) -> Result<Self, ProtocolError> {
        let claims = Self {
            version: PROTOCOL_VERSION,
            credential,
            login_role: login_role.into(),
            worker,
            assignment_generation,
            security_epoch: 0,
            configuration_epoch: 0,
            issued_at_unix_seconds,
            expires_at_unix_seconds,
            protocols,
        };
        claims.validate_shape()?;
        Ok(claims)
    }

    pub fn permits(&self, protocol: ApplicationProtocol) -> bool {
        self.protocols.contains(&protocol)
    }

    fn validate_shape(&self) -> Result<(), ProtocolError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        validate_login_role(&self.login_role)?;
        if self.expires_at_unix_seconds <= self.issued_at_unix_seconds {
            return Err(ProtocolError::InvalidLeaseTime);
        }
        if self
            .expires_at_unix_seconds
            .saturating_sub(self.issued_at_unix_seconds)
            > MAX_LEASE_TTL_SECONDS
        {
            return Err(ProtocolError::LeaseTooLong);
        }
        validate_protocols(&self.protocols)
    }

    fn validate_at(&self, now_unix_seconds: u64) -> Result<(), ProtocolError> {
        self.validate_shape()?;
        if self.issued_at_unix_seconds > now_unix_seconds.saturating_add(MAX_CLOCK_SKEW_SECONDS) {
            return Err(ProtocolError::LeaseNotYetValid);
        }
        if self.expires_at_unix_seconds <= now_unix_seconds {
            return Err(ProtocolError::LeaseExpired);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignedAccessLease {
    pub claims: AccessLeaseClaims,
    pub signature: Signature,
}

impl SignedAccessLease {
    pub fn issue(
        claims: AccessLeaseClaims,
        bootstrap_secret: &SecretKey,
    ) -> Result<Self, ProtocolError> {
        claims.validate_shape()?;
        let signature = bootstrap_secret.sign(&lease_signing_bytes(&claims)?);
        Ok(Self { claims, signature })
    }

    pub fn verify(
        &self,
        bootstrap_public: PublicKey,
        now_unix_seconds: u64,
        expected_worker: EndpointId,
        expected_credential: PublicKey,
    ) -> Result<(), ProtocolError> {
        bootstrap_public
            .verify(&lease_signing_bytes(&self.claims)?, &self.signature)
            .map_err(|_| ProtocolError::InvalidLeaseSignature)?;
        self.claims.validate_at(now_unix_seconds)?;
        if self.claims.worker.id != expected_worker {
            return Err(ProtocolError::WrongWorker);
        }
        if self.claims.credential != expected_credential {
            return Err(ProtocolError::WrongCredential);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseRequest {
    pub version: u8,
    pub credential: PublicKey,
    pub client_transport: EndpointId,
    pub nonce: [u8; 32],
    pub proof: Signature,
}

impl LeaseRequest {
    pub fn sign(
        credential_secret: &SecretKey,
        client_transport: EndpointId,
        bootstrap: EndpointId,
        nonce: [u8; 32],
    ) -> Result<Self, ProtocolError> {
        let credential = credential_secret.public();
        let proof = credential_secret.sign(&lease_request_signing_bytes(
            credential,
            client_transport,
            bootstrap,
            nonce,
        )?);
        Ok(Self {
            version: PROTOCOL_VERSION,
            credential,
            client_transport,
            nonce,
            proof,
        })
    }

    pub fn verify(
        &self,
        bootstrap: EndpointId,
        remote_transport: EndpointId,
    ) -> Result<(), ProtocolError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        if self.client_transport != remote_transport {
            return Err(ProtocolError::WrongTransportEndpoint);
        }
        self.credential
            .verify(
                &lease_request_signing_bytes(
                    self.credential,
                    self.client_transport,
                    bootstrap,
                    self.nonce,
                )?,
                &self.proof,
            )
            .map_err(|_| ProtocolError::InvalidCredentialProof)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeChallenge {
    pub version: u8,
    pub nonce: [u8; 32],
}

impl EdgeChallenge {
    pub fn new(nonce: [u8; 32]) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            nonce,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeAuthenticate {
    pub version: u8,
    pub lease: SignedAccessLease,
    pub client_transport: EndpointId,
    pub challenge: EdgeChallenge,
    pub compression_offers: Vec<CompressionCodec>,
    pub proof: Signature,
}

impl EdgeAuthenticate {
    pub fn sign(
        lease: SignedAccessLease,
        credential_secret: &SecretKey,
        client_transport: EndpointId,
        challenge: EdgeChallenge,
        compression_offers: Vec<CompressionCodec>,
    ) -> Result<Self, ProtocolError> {
        validate_compression_offers(&compression_offers)?;
        if lease.claims.credential != credential_secret.public() {
            return Err(ProtocolError::WrongCredential);
        }
        let mut auth = Self {
            version: PROTOCOL_VERSION,
            lease,
            client_transport,
            challenge,
            compression_offers,
            proof: credential_secret.sign(b"placeholder"),
        };
        auth.proof = credential_secret.sign(&edge_proof_signing_bytes(&auth)?);
        Ok(auth)
    }

    pub fn verify(
        &self,
        bootstrap_public: PublicKey,
        now_unix_seconds: u64,
        worker: EndpointId,
        remote_transport: EndpointId,
        expected_challenge: &EdgeChallenge,
    ) -> Result<(), ProtocolError> {
        if self.version != PROTOCOL_VERSION || self.challenge.version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        if &self.challenge != expected_challenge {
            return Err(ProtocolError::WrongChallenge);
        }
        if self.client_transport != remote_transport {
            return Err(ProtocolError::WrongTransportEndpoint);
        }
        validate_compression_offers(&self.compression_offers)?;
        self.lease.verify(
            bootstrap_public,
            now_unix_seconds,
            worker,
            self.lease.claims.credential,
        )?;
        self.lease
            .claims
            .credential
            .verify(&edge_proof_signing_bytes(self)?, &self.proof)
            .map_err(|_| ProtocolError::InvalidCredentialProof)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeAuthenticated {
    pub version: u8,
    pub compression: CompressionCodec,
}

impl EdgeAuthenticated {
    pub fn select(
        compression: CompressionCodec,
        offers: &[CompressionCodec],
    ) -> Result<Self, ProtocolError> {
        validate_compression_offers(offers)?;
        if !offers.contains(&compression) {
            return Err(ProtocolError::CompressionNotOffered);
        }
        Ok(Self {
            version: PROTOCOL_VERSION,
            compression,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StreamPrelude {
    pub version: u8,
    pub protocol: ApplicationProtocol,
    pub compression: CompressionCodec,
}

impl StreamPrelude {
    pub fn new(protocol: ApplicationProtocol, compression: CompressionCodec) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            protocol,
            compression,
        }
    }

    pub fn verify(
        &self,
        lease: &AccessLeaseClaims,
        negotiated_compression: CompressionCodec,
    ) -> Result<(), ProtocolError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        if !lease.permits(self.protocol) {
            return Err(ProtocolError::ProtocolNotPermitted);
        }
        let expected_compression = match self.protocol {
            ApplicationProtocol::Cancellation => CompressionCodec::None,
            _ => negotiated_compression,
        };
        if self.compression != expected_compression {
            return Err(ProtocolError::CompressionNotOffered);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelayPolicy {
    PublicDefault,
    Custom(Vec<RelayUrl>),
}

impl RelayPolicy {
    pub fn from_config(configured: Option<Vec<String>>) -> Result<Self, ProtocolError> {
        let Some(configured) = configured else {
            return Ok(Self::PublicDefault);
        };
        if configured.is_empty() {
            return Err(ProtocolError::EmptyRelayList);
        }
        let mut seen = HashSet::with_capacity(configured.len());
        let mut relays = Vec::with_capacity(configured.len());
        for raw in configured {
            let relay = raw
                .parse::<RelayUrl>()
                .map_err(|_| ProtocolError::InvalidRelayUrl(raw.clone()))?;
            if !seen.insert(relay.clone()) {
                return Err(ProtocolError::DuplicateRelay);
            }
            relays.push(relay);
        }
        Ok(Self::Custom(relays))
    }

    pub fn iroh_mode(&self) -> iroh::RelayMode {
        match self {
            Self::PublicDefault => iroh::RelayMode::Default,
            Self::Custom(relays) => iroh::RelayMode::custom(relays.clone()),
        }
    }
}

pub fn encode_control<T: Serialize>(message: &T) -> Result<Vec<u8>, ProtocolError> {
    let encoded = serde_json::to_vec(message)
        .map_err(|error| ProtocolError::InvalidControlMessage(error.to_string()))?;
    if encoded.len() > MAX_CONTROL_MESSAGE_BYTES {
        return Err(ProtocolError::ControlMessageTooLarge);
    }
    Ok(encoded)
}

pub fn decode_control<T: DeserializeOwned>(encoded: &[u8]) -> Result<T, ProtocolError> {
    if encoded.len() > MAX_CONTROL_MESSAGE_BYTES {
        return Err(ProtocolError::ControlMessageTooLarge);
    }
    serde_json::from_slice(encoded)
        .map_err(|error| ProtocolError::InvalidControlMessage(error.to_string()))
}

fn validate_login_role(role: &str) -> Result<(), ProtocolError> {
    if role.is_empty()
        || role.len() > MAX_LOGIN_ROLE_BYTES
        || role
            .chars()
            .any(|character| character == '\0' || character.is_control())
    {
        return Err(ProtocolError::InvalidLoginRole);
    }
    Ok(())
}

fn validate_protocols(protocols: &[ApplicationProtocol]) -> Result<(), ProtocolError> {
    let unique = protocols.iter().copied().collect::<HashSet<_>>();
    if protocols.is_empty() || protocols.len() > 3 || unique.len() != protocols.len() {
        return Err(ProtocolError::InvalidApplicationProtocols);
    }
    Ok(())
}

fn validate_compression_offers(offers: &[CompressionCodec]) -> Result<(), ProtocolError> {
    let unique = offers.iter().copied().collect::<HashSet<_>>();
    if !offers.contains(&CompressionCodec::None) || offers.len() != unique.len() || offers.len() > 2
    {
        return Err(ProtocolError::MissingUncompressedCodec);
    }
    Ok(())
}

fn domain_message<T: Serialize>(domain: &[u8], value: &T) -> Result<Vec<u8>, ProtocolError> {
    let mut bytes = Vec::from(domain);
    bytes.extend(encode_control(value)?);
    Ok(bytes)
}

fn lease_signing_bytes(claims: &AccessLeaseClaims) -> Result<Vec<u8>, ProtocolError> {
    domain_message(LEASE_DOMAIN, claims)
}

#[derive(Serialize)]
struct LeaseRequestProof {
    version: u8,
    credential: PublicKey,
    client_transport: EndpointId,
    bootstrap: EndpointId,
    nonce: [u8; 32],
}

fn lease_request_signing_bytes(
    credential: PublicKey,
    client_transport: EndpointId,
    bootstrap: EndpointId,
    nonce: [u8; 32],
) -> Result<Vec<u8>, ProtocolError> {
    domain_message(
        LEASE_REQUEST_DOMAIN,
        &LeaseRequestProof {
            version: PROTOCOL_VERSION,
            credential,
            client_transport,
            bootstrap,
            nonce,
        },
    )
}

#[derive(Serialize)]
struct EdgeProof<'a> {
    version: u8,
    lease: &'a SignedAccessLease,
    client_transport: EndpointId,
    challenge: &'a EdgeChallenge,
    compression_offers: &'a [CompressionCodec],
}

fn edge_proof_signing_bytes(auth: &EdgeAuthenticate) -> Result<Vec<u8>, ProtocolError> {
    domain_message(
        EDGE_PROOF_DOMAIN,
        &EdgeProof {
            version: auth.version,
            lease: &auth.lease,
            client_transport: auth.client_transport,
            challenge: &auth.challenge,
            compression_offers: &auth.compression_offers,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease_fixture(now: u64) -> (SecretKey, SecretKey, SecretKey, SignedAccessLease) {
        let bootstrap = SecretKey::from_bytes(&[1; 32]);
        let credential = SecretKey::from_bytes(&[2; 32]);
        let worker = SecretKey::from_bytes(&[3; 32]);
        let claims = AccessLeaseClaims::new(
            credential.public(),
            "reader",
            EndpointAddr::new(worker.public()).with_ip_addr("127.0.0.1:4242".parse().unwrap()),
            7,
            now,
            now + 60,
            vec![
                ApplicationProtocol::Pgwire,
                ApplicationProtocol::Cancellation,
            ],
        )
        .unwrap();
        let lease = SignedAccessLease::issue(claims, &bootstrap).unwrap();
        (bootstrap, credential, worker, lease)
    }

    #[test]
    fn lease_is_short_lived_signed_and_worker_bound() {
        let now = 1_700_000_000;
        let (bootstrap, credential, worker, lease) = lease_fixture(now);
        lease
            .verify(
                bootstrap.public(),
                now,
                worker.public(),
                credential.public(),
            )
            .unwrap();
        assert!(matches!(
            lease.verify(
                bootstrap.public(),
                now + 60,
                worker.public(),
                credential.public()
            ),
            Err(ProtocolError::LeaseExpired)
        ));
        assert!(matches!(
            lease.verify(
                bootstrap.public(),
                now,
                SecretKey::from_bytes(&[4; 32]).public(),
                credential.public()
            ),
            Err(ProtocolError::WrongWorker)
        ));
    }

    #[test]
    fn modified_claims_fail_signature_validation() {
        let now = 1_700_000_000;
        let (bootstrap, credential, worker, mut lease) = lease_fixture(now);
        lease.claims.login_role = "editor".to_string();
        assert!(matches!(
            lease.verify(
                bootstrap.public(),
                now,
                worker.public(),
                credential.public()
            ),
            Err(ProtocolError::InvalidLeaseSignature)
        ));
    }

    #[test]
    fn lease_request_binds_credential_transport_and_bootstrap() {
        let credential = SecretKey::from_bytes(&[5; 32]);
        let client_transport = SecretKey::from_bytes(&[6; 32]).public();
        let bootstrap = SecretKey::from_bytes(&[7; 32]).public();
        let request =
            LeaseRequest::sign(&credential, client_transport, bootstrap, [8; 32]).unwrap();
        request.verify(bootstrap, client_transport).unwrap();
        assert!(matches!(
            request.verify(bootstrap, SecretKey::from_bytes(&[9; 32]).public()),
            Err(ProtocolError::WrongTransportEndpoint)
        ));
        assert!(matches!(
            request.verify(SecretKey::from_bytes(&[10; 32]).public(), client_transport),
            Err(ProtocolError::InvalidCredentialProof)
        ));
    }

    #[test]
    fn edge_proof_binds_fresh_challenge_and_transport() {
        let now = 1_700_000_000;
        let (bootstrap, credential, worker, lease) = lease_fixture(now);
        let client_transport = SecretKey::from_bytes(&[11; 32]).public();
        let challenge = EdgeChallenge::new([12; 32]);
        let auth = EdgeAuthenticate::sign(
            lease,
            &credential,
            client_transport,
            challenge.clone(),
            vec![CompressionCodec::None],
        )
        .unwrap();
        auth.verify(
            bootstrap.public(),
            now,
            worker.public(),
            client_transport,
            &challenge,
        )
        .unwrap();
        assert!(matches!(
            auth.verify(
                bootstrap.public(),
                now,
                worker.public(),
                client_transport,
                &EdgeChallenge::new([13; 32])
            ),
            Err(ProtocolError::WrongChallenge)
        ));
    }

    #[test]
    fn compression_policy_requires_none_and_selects_lz4_only_in_auto_mode() {
        let auto = CompressionPolicy::Auto.offers();
        assert_eq!(
            CompressionPolicy::Auto.select(&auto).unwrap(),
            CompressionCodec::Lz4Block
        );
        assert_eq!(
            CompressionPolicy::Off.select(&auto).unwrap(),
            CompressionCodec::None
        );
        assert!(matches!(
            CompressionPolicy::Auto.select(&[CompressionCodec::Lz4Block]),
            Err(ProtocolError::MissingUncompressedCodec)
        ));
    }

    #[test]
    fn cancellation_streams_are_always_uncompressed() {
        let (_, _, _, lease) = lease_fixture(1_700_000_000);
        StreamPrelude::new(ApplicationProtocol::Cancellation, CompressionCodec::None)
            .verify(&lease.claims, CompressionCodec::Lz4Block)
            .unwrap();
        assert!(
            StreamPrelude::new(
                ApplicationProtocol::Cancellation,
                CompressionCodec::Lz4Block
            )
            .verify(&lease.claims, CompressionCodec::Lz4Block)
            .is_err()
        );
    }

    #[test]
    fn relay_policy_distinguishes_omitted_empty_and_custom() {
        assert_eq!(
            RelayPolicy::from_config(None).unwrap(),
            RelayPolicy::PublicDefault
        );
        assert!(matches!(
            RelayPolicy::from_config(Some(vec![])),
            Err(ProtocolError::EmptyRelayList)
        ));
        assert!(matches!(
            RelayPolicy::from_config(Some(vec!["not a URL".to_string()])),
            Err(ProtocolError::InvalidRelayUrl(_))
        ));
        let custom =
            RelayPolicy::from_config(Some(vec!["https://relay.example.test".to_string()])).unwrap();
        assert!(matches!(custom, RelayPolicy::Custom(relays) if relays.len() == 1));
    }

    #[test]
    fn control_messages_are_bounded_and_reject_unknown_fields() {
        let challenge = EdgeChallenge::new([14; 32]);
        let encoded = encode_control(&challenge).unwrap();
        assert_eq!(
            decode_control::<EdgeChallenge>(&encoded).unwrap(),
            challenge
        );
        assert!(matches!(
            decode_control::<EdgeChallenge>(
                br#"{"version":1,"nonce":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"extra":true}"#
            ),
            Err(ProtocolError::InvalidControlMessage(_))
        ));
        assert!(matches!(
            decode_control::<EdgeChallenge>(&vec![b'x'; MAX_CONTROL_MESSAGE_BYTES + 1]),
            Err(ProtocolError::ControlMessageTooLarge)
        ));
    }

    #[test]
    fn invalid_roles_protocols_and_lifetimes_fail_closed() {
        let credential = SecretKey::from_bytes(&[15; 32]);
        let worker = SecretKey::from_bytes(&[16; 32]);
        let build = |role: &str, expires, protocols| {
            AccessLeaseClaims::new(
                credential.public(),
                role,
                EndpointAddr::new(worker.public()),
                1,
                100,
                expires,
                protocols,
            )
        };
        assert!(matches!(
            build("", 110, vec![ApplicationProtocol::Pgwire]),
            Err(ProtocolError::InvalidLoginRole)
        ));
        assert!(matches!(
            build("reader", 100, vec![ApplicationProtocol::Pgwire]),
            Err(ProtocolError::InvalidLeaseTime)
        ));
        assert!(matches!(
            build(
                "reader",
                110,
                vec![ApplicationProtocol::Pgwire, ApplicationProtocol::Pgwire]
            ),
            Err(ProtocolError::InvalidApplicationProtocols)
        ));
    }
}

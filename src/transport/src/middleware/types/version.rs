//! Version validation and enforcement middleware.
//!
//! This module provides a middleware that ensures protocol compatibility between
//! the client (CLI) and the server (Daemon) by checking version metadata
//! attached to messages.
//!
//! # Compatibility Logic
//!
//! The middleware follows a "Safe-SemVer" approach for compatibility:
//! - **Major** and **Minor** versions must match exactly.
//! - **Patch** versions are ignored during validation, allowing for minor bug fixes
//!   to be deployed independently without breaking communication.
//!
//! For example, a client with version `1.2.0` is compatible with a server at `1.2.5`,
//! but incompatible with versions `1.3.0` or `2.2.0`.
//!
//! # Metadata
//!
//! The middleware uses the `X-Knot-Version` metadata key to transport version information.
//! Outbound messages automatically have the current crate version attached, and
//! inbound messages are validated against this version.
use crate::{
    messages::Message,
    middleware::{Inbound, Outbound, traits::Middleware},
    transport::{RawTransport, TransportSpec},
};
use async_trait::async_trait;
use knot_core::errors::TransportError;
use tracing::{trace, warn};

/// Metadata key used to store the application version.
const VERSION_META_KEY: &str = "X-Knot-Version";

/// The current version of the crate, captured at compile time.
const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Internal helper to check if two SemVer strings are compatible.
///
/// Compatibility is defined as matching Major and Minor versions.
/// Patch versions are allowed to differ.
fn is_compatible(remote: &str, local: &str) -> bool {
    let mut remote_split = remote.split('.');
    let mut local_split = local.split('.');

    // Check Major
    if remote_split.next() != local_split.next() {
        return false;
    }

    // Check Minor
    if remote_split.next() != local_split.next() {
        return false;
    }

    true
}

/// Middleware for enforcing protocol versioning.
///
/// `VersionMiddleware` automatically injects the current package version into
/// all outbound messages and validates the version of all inbound messages.
///
/// # Examples
///
/// ```rust,ignore
/// use knot_transport::middleware::types::VersionMiddleware;
///
/// // Create a strict middleware (rejects messages without version)
/// let mw = VersionMiddleware::new(false);
///
/// // Create a lenient middleware (useful for rolling updates)
/// let mw_lenient = VersionMiddleware::new(true);
/// ```
#[derive(Debug)]
pub struct VersionMiddleware {
    /// Whether to allow messages that are missing the version metadata key.
    allow_missing: bool,
}

impl VersionMiddleware {
    /// Creates a new `VersionMiddleware`.
    ///
    /// # Arguments
    ///
    /// * `allow_missing` - If true, messages without version metadata will be accepted.
    pub fn new(allow_missing: bool) -> Self {
        Self { allow_missing }
    }
}

impl Default for VersionMiddleware {
    /// Returns a strict `VersionMiddleware` that rejects missing metadata.
    fn default() -> Self {
        Self::new(false)
    }
}

#[async_trait]
impl<R: RawTransport, S: TransportSpec> Middleware<R, S> for VersionMiddleware {
    /// Validates the version metadata of an inbound message.
    ///
    /// If the version is missing and `allow_missing` is false, it returns a
    /// [`TransportError::MiddlewareBlocked`].
    ///
    /// If the version is present but incompatible (Major/Minor mismatch),
    /// it returns a [`TransportError::MiddlewareBlocked`].
    async fn on_recv(
        &self,
        msg: &Message<S::Req, S::Res, S::Ev>,
        next: Inbound<'_, R, S>,
    ) -> Result<(), TransportError> {
        match msg.get_meta(VERSION_META_KEY) {
            None => {
                if self.allow_missing {
                    trace!("allow_missing is enabled, skipping version check");
                    next.run(msg).await
                } else {
                    warn!("Rejecting message: missing version metadata");
                    Err(TransportError::MiddlewareBlocked {
                        name: "VersionMiddleware".to_string(),
                        reason: "Missing version metadata".to_string(),
                    })
                }
            }
            Some(version) => {
                if !is_compatible(version, CRATE_VERSION) {
                    warn!(
                        "Version incompatibility detected: {} is not compatible with {}",
                        version, CRATE_VERSION
                    );
                    return Err(TransportError::MiddlewareBlocked {
                        name: "VersionMiddleware".to_string(),
                        reason: format!(
                            "Incompatible version (current: {}, received: {})",
                            CRATE_VERSION, version
                        ),
                    });
                }

                next.run(msg).await
            }
        }
    }

    /// Injects the current crate version into the outbound message metadata.
    async fn on_send(
        &self,
        msg: &mut Message<S::Req, S::Res, S::Ev>,
        next: Outbound<'_, R, S>,
    ) -> Result<(), TransportError> {
        msg.set_meta(VERSION_META_KEY, CRATE_VERSION)?;

        trace!("Appended version metadata to message: {}", CRATE_VERSION);
        next.run(msg).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::JsonCodec;
    use crate::middleware::Pipeline;
    use serde::{Deserialize, Serialize};
    use tokio::sync::mpsc;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    enum MockReq {
        Ping,
    }
    #[derive(Debug, Clone, Serialize, Deserialize)]
    enum MockRes {
        Pong,
    }
    #[derive(Debug, Clone, Serialize, Deserialize)]
    enum MockEv {
        Event,
    }

    #[derive(Debug)]
    struct MockSpec;
    impl TransportSpec for MockSpec {
        type Req = MockReq;
        type Res = MockRes;
        type Ev = MockEv;
        type C = JsonCodec;
    }

    struct MockRaw;
    #[async_trait]
    impl RawTransport for MockRaw {
        async fn send_frame_internal<'a>(&self, _f: &'a [u8]) -> Result<(), TransportError> {
            Ok(())
        }
        async fn recv_frame_internal(&self) -> Result<Vec<u8>, TransportError> {
            let (_, mut rx) = mpsc::channel(1);
            rx.recv().await.ok_or(TransportError::ConnectionClosed)
        }
    }

    #[test]
    fn test_version_compatibility() {
        assert!(is_compatible("1.2.3", "1.2.3"));
        assert!(is_compatible("1.2.0", "1.2.9")); // Patch mismatch is OK
        assert!(is_compatible("1.2.3", "1.2.0")); // Patch mismatch is OK
        assert!(!is_compatible("1.3.0", "1.2.0")); // Minor mismatch is NOT OK
        assert!(!is_compatible("2.0.0", "1.0.0")); // Major mismatch is NOT OK
    }

    #[tokio::test]
    async fn test_on_send_adds_version() {
        let mut pipeline = Pipeline::<MockRaw, MockSpec>::default();
        pipeline.add_middleware(VersionMiddleware::new(false));
        let mut msg = Message::<MockReq, MockRes, MockEv>::request(1, MockReq::Ping);

        let result = pipeline.execute_send(&mut msg).await;
        assert!(result.is_ok());
        assert_eq!(msg.get_meta(VERSION_META_KEY).unwrap(), CRATE_VERSION);
    }

    #[tokio::test]
    async fn test_on_recv_compatible_version() {
        let mut pipeline = Pipeline::<MockRaw, MockSpec>::default();
        pipeline.add_middleware(VersionMiddleware::new(false));
        let mut msg = Message::<MockReq, MockRes, MockEv>::request(1, MockReq::Ping);
        msg.set_meta(VERSION_META_KEY, CRATE_VERSION).unwrap();

        let inbound = pipeline.execute_recv(&msg).await;
        assert!(inbound.is_ok());
    }

    #[tokio::test]
    async fn test_on_recv_incompatible_version() {
        let mut pipeline = Pipeline::<MockRaw, MockSpec>::default();
        pipeline.add_middleware(VersionMiddleware::new(false));
        let mut msg = Message::<MockReq, MockRes, MockEv>::request(1, MockReq::Ping);
        msg.set_meta(VERSION_META_KEY, "99.99.99").unwrap();

        let inbound = pipeline.execute_recv(&msg).await;

        assert!(inbound.is_err());
        if let Err(TransportError::MiddlewareBlocked { name, .. }) = inbound {
            assert_eq!(name, "VersionMiddleware");
        } else {
            panic!("Expected MiddlewareBlocked error");
        }
    }

    #[tokio::test]
    async fn test_on_recv_missing_version_blocked() {
        let mut pipeline = Pipeline::<MockRaw, MockSpec>::default();
        pipeline.add_middleware(VersionMiddleware::new(false));
        let msg = Message::<MockReq, MockRes, MockEv>::request(1, MockReq::Ping);

        let inbound = pipeline.execute_recv(&msg).await;
        assert!(inbound.is_err());
    }

    #[tokio::test]
    async fn test_on_recv_missing_version_allowed() {
        let mut pipeline = Pipeline::<MockRaw, MockSpec>::default();
        pipeline.add_middleware(VersionMiddleware::new(true));
        let msg = Message::<MockReq, MockRes, MockEv>::request(1, MockReq::Ping);

        let inbound = pipeline.execute_recv(&msg).await;
        assert!(inbound.is_ok());
    }
}

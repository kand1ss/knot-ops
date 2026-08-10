use crate::{builder::ClientBuilder, errors::ClientError, states::ConnectState};

/// The primary entry point for interacting with the Knot daemon.
///
/// `KnotClient` serves as a stateless namespace that provides convenient methods
/// for bootstrapping a connection to the background daemon. You can either use
/// the highly customizable [`ClientBuilder`] for advanced initialization, or rely
/// on the [`Self::connect`] shorthand for standard environments.
pub struct KnotClient;
impl KnotClient {
    /// Creates a new `ClientBuilder` with default configurations.
    ///
    /// The builder allows you to customize the daemon launch strategy (e.g., overriding
    /// the default launcher with a specific executable path or mock implementation)
    /// before establishing the connection.
    ///
    /// # Returns
    ///
    /// Returns a default-initialized [`ClientBuilder`].
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// A convenience method to rapidly connect to the daemon using default settings.
    ///
    /// This is a fast-path execution that is semantically identical to calling
    /// `KnotClient::builder().connect(directory)`. It will automatically resolve the
    /// workspace, inspect the process state via the OS table, and attempt to establish
    /// a gRPC connection.
    ///
    /// # Arguments
    ///
    /// * `directory` - The target filesystem path from which to begin searching for
    ///   the `.knot` workspace directory.
    ///
    /// # Returns
    ///
    /// Returns a [`ConnectState`] enum representing the daemon's exact lifecycle phase
    /// (e.g., `Offline`, `Connected`, `Hung`, or `Stale`).
    ///
    /// # Errors
    ///
    /// Returns a `ClientError` if the workspace is not initialized, or if a severe
    /// filesystem/I/O error occurs during the inspection sequence.
    pub async fn connect() -> Result<ConnectState, ClientError> {
        Self::builder().connect().await
    }
}

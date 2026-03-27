use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::HashMap,
    ops::{Deref, DerefMut},
};

/// A specialized container for message metadata, optimized for efficiency and flexibility.
///
/// `MetadataMap` acts as a key-value store (based on `HashMap`) where both keys and values
/// are stored as `Cow<'static, str>`. This allows the transport to:
/// 1. Use zero-cost static strings for common keys (e.g., `"content-type"`).
/// 2. Seamlessly handle dynamic strings when necessary.
///
/// ### Behavioral Note
/// Thanks to the implementation of [`Deref`] and [`DerefMut`], this structure
/// transparently exposes all standard `HashMap` methods (like `.get()`, `.iter()`, etc.)
/// while maintaining its specialized type constraints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetadataMap(pub(crate) HashMap<Cow<'static, str>, Cow<'static, str>>);
impl MetadataMap {
    /// Creates a new, empty `MetadataMap`.
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Creates a new `MetadataMap` with a pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self(HashMap::with_capacity(capacity))
    }

    /// Inserts a key-value pair into the map, automatically converting inputs
    /// into optimized [`Cow`] strings.
    ///
    /// # Example
    /// ```rust,ignore
    /// let mut meta = MetadataMap::new();
    /// meta.insert_str("service-id", "knot-daemon"); // Static &str (No allocation)
    /// meta.insert_str("dynamic-key", format!("user-{}", 123)); // String (Owned)
    /// ```
    pub fn insert_str<K, V>(&mut self, key: K, value: V)
    where
        K: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>,
    {
        self.0.insert(key.into(), value.into());
    }
}
impl Deref for MetadataMap {
    type Target = HashMap<Cow<'static, str>, Cow<'static, str>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for MetadataMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

//! # Hash References
//!
//!
//! Some MLS messages refer to other MLS objects by hash.  For example, Welcome
//! messages refer to KeyPackages for the members being welcomed, and Commits refer
//! to Proposals they cover.  These identifiers are computed as follows:
//!
//! ```text
//! opaque HashReference[16];
//!
//! MakeHashRef(value) = KDF.expand(KDF.extract("", value), "MLS 1.0 ref", 16)
//!
//! HashReference KeyPackageRef;
//! HashReference ProposalRef;
//! ```
//!
//! For a KeyPackageRef, the `value` input is the encoded KeyPackage, and the
//! ciphersuite specified in the KeyPackage determines the KDF used.  For a
//! ProposalRef, the `value` input is the MLSPlaintext carrying the proposal, and
//! the KDF is determined by the group's ciphersuite.

use std::convert::TryInto;

use openmls_traits::{crypto::OpenMlsCrypto, types::CryptoError};
use tls_codec::{TlsDeserialize, TlsSerialize, TlsSize};

use super::Ciphersuite;

const LABEL: &[u8; 11] = b"MLS 1.0 ref";
const VALUE_LEN: usize = 16;
type Value = [u8; VALUE_LEN];

/// A reference to an MLS object computed as an HKDF of the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TlsDeserialize, TlsSerialize, TlsSize)]
pub struct HashReference {
    value: Value,
}

/// A reference to a key package.
/// This value uniquely identifies a key package.
pub type KeyPackageRef = HashReference;

/// A reference to a proposal.
/// This value uniquely identifies a proposal.
pub type ProposalRef = HashReference;

impl HashReference {
    /// Compute a new [`HashReference`] value for a `value`.
    pub fn new(
        value: &[u8],
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCrypto,
    ) -> Result<Self, CryptoError> {
        let okm = backend.hkdf_expand(
            ciphersuite.hash,
            &backend.hkdf_extract(ciphersuite.hash, &[], value)?,
            LABEL,
            VALUE_LEN,
        )?;
        let value: Value = okm.try_into().map_err(|_| CryptoError::InvalidLength)?;
        Ok(Self { value })
    }

    /// Get a reference to the hash reference's value.
    pub fn value(&self) -> &[u8; 16] {
        &self.value
    }
}

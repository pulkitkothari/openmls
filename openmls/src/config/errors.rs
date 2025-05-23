//!# Config errors
//!
//! A `ConfigError` is thrown when either the configuration itself is invalid or
//! inconsistent, or if an MLS configuration is being used that is not
//! supported.

implement_error! {
    pub enum ConfigError {
        InvalidConfig = "Invalid configuration.",
        UnsupportedMlsVersion = "MLS version is not supported by this configuration.",
        UnsupportedCiphersuite = "Ciphersuite is not supported by this configuration.",
        UnsupportedSignatureScheme = "Signature scheme is not supported by this configuration.",
        UnsupportedProposalType = "Unsupported, required proposal type.",
        UnsupportedExtensionsType = "Unsupported, required extension type.",
        IncompatibleMlsVersion = "Operation on incompatible MLS versions.",
        IncompatibleCiphersuite = "Operation on incompatible cipher suites.",
        LibraryError = "Library error",
    }
}

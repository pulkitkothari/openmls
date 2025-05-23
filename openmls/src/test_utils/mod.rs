//! Test utilities
#![allow(dead_code)]

pub use openmls_traits::OpenMlsCryptoProvider;
pub use rstest::*;
pub use rstest_reuse::{self, *};

pub use crate::{
    ciphersuite::{ciphersuites::CiphersuiteName, Ciphersuite},
    config::Config,
    utils::*,
};

use serde::{self, de::DeserializeOwned, Serialize};
use std::{
    fs::File,
    io::{BufReader, Write},
};

pub mod test_framework;

pub(crate) fn write(file_name: &str, obj: impl Serialize) {
    let mut file = match File::create(file_name) {
        Ok(f) => f,
        Err(_) => panic!("Couldn't open file {}.", file_name),
    };
    file.write_all(
        serde_json::to_string_pretty(&obj)
            .expect("Error serializing test vectors")
            .as_bytes(),
    )
    .expect("Error writing test vector file");
}

pub(crate) fn read<T: DeserializeOwned>(file_name: &str) -> T {
    let file = match File::open(file_name) {
        Ok(f) => f,
        Err(_) => panic!("Couldn't open file {}.", file_name),
    };
    let reader = BufReader::new(file);
    match serde_json::from_reader(reader) {
        Ok(r) => r,
        Err(e) => panic!("Error reading file.\n{:?}", e),
    }
}

/// Convert `bytes` to a hex string.
pub fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut hex = String::new();
    for &b in bytes {
        hex += &format!("{:02X}", b);
    }
    hex
}

/// Convert a hex string to a byte vector.
pub fn hex_to_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0);
    let mut bytes = Vec::new();
    for i in 0..(hex.len() / 2) {
        bytes.push(
            u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("An unexpected error occurred."),
        );
    }
    bytes
}

/// Convert a hex string to a byte vector.
/// If the input is `None`, this returns an empty vector.
pub fn hex_to_bytes_option(hex: Option<String>) -> Vec<u8> {
    match hex {
        Some(s) => hex_to_bytes(&s),
        None => vec![],
    }
}

// === Define backend per platform ===

// For now we only use Evercrypt on specific platforms and only if the feature was enabled

#[cfg(all(
    target_arch = "x86_64",
    not(target_os = "macos"),
    not(target_family = "wasm"),
    feature = "evercrypt",
))]
pub use evercrypt_backend::OpenMlsEvercrypt;

// This backend is currently used on all platforms
pub use openmls_rust_crypto::OpenMlsRustCrypto;

// === Backends ===

#[cfg(any(
    not(target_arch = "x86_64"),
    target_os = "macos",
    target_family = "wasm",
    not(feature = "evercrypt")
))]
#[template]
#[rstest(backend,
    case::rust_crypto(&OpenMlsRustCrypto::default()),
  )
]
pub fn backends(backend: &impl OpenMlsCryptoProvider) {}

// For now we only use Evercrypt on specific platforms and only if the feature was enabled

#[cfg(all(
    target_arch = "x86_64",
    not(target_os = "macos"),
    not(target_family = "wasm"),
    feature = "evercrypt",
))]
#[template]
#[rstest(backend,
    case::rust_crypto(&OpenMlsRustCrypto::default()),
    case::evercrypt(&evercrypt_backend::OpenMlsEvercrypt::default()),
  )
]
pub fn backends(backend: &impl OpenMlsCryptoProvider) {}

// === Ciphersuites ===

// For now we support all ciphersuites, regardless of the backend

#[allow(non_snake_case)]
#[template]
#[rstest(ciphersuite,
    case::MLS10_128_DHKEMX25519_AES128GCM_SHA256_Ed25519(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMX25519_AES128GCM_SHA256_Ed25519).expect("Ciphersuite not supported.")),
    case::MLS10_128_DHKEMP256_AES128GCM_SHA256_P256(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMP256_AES128GCM_SHA256_P256).expect("Ciphersuite not supported.")),
    case::MLS10_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519).expect("Ciphersuite not supported.")),
  )
]
pub fn ciphersuites(ciphersuite: &'static Ciphersuite) {}

// === Ciphersuites & backends ===

#[cfg(any(
    not(target_arch = "x86_64"),
    target_os = "macos",
    target_family = "wasm",
    not(feature = "evercrypt"),
))]
#[allow(non_snake_case)]
#[template]
#[rstest(ciphersuite, backend,
    case::rust_crypto_MLS10_128_DHKEMX25519_AES128GCM_SHA256_Ed25519(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMX25519_AES128GCM_SHA256_Ed25519).expect("Ciphersuite not supported."), &OpenMlsRustCrypto::default()),
    case::rust_crypto_MLS10_128_DHKEMP256_AES128GCM_SHA256_P256(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMP256_AES128GCM_SHA256_P256).expect("Ciphersuite not supported."), &OpenMlsRustCrypto::default()),
    case::rust_crypto_MLS10_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519).expect("Ciphersuite not supported."), &OpenMlsRustCrypto::default()),
  )
]
pub fn ciphersuites_and_backends(
    ciphersuite: &'static Ciphersuite,
    backend: &impl OpenMlsCryptoProvider,
) {
}

// For now we only use Evercrypt on specific platforms and only if the feature was enabled

#[cfg(all(
    target_arch = "x86_64",
    not(target_os = "macos"),
    not(target_family = "wasm"),
    feature = "evercrypt",
))]
#[allow(non_snake_case)]
#[template]
#[rstest(ciphersuite, backend,
    case::rust_crypto_MLS10_128_DHKEMX25519_AES128GCM_SHA256_Ed25519(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMX25519_AES128GCM_SHA256_Ed25519).expect("Ciphersuite not supported."), &OpenMlsRustCrypto::default()),
    case::rust_crypto_MLS10_128_DHKEMP256_AES128GCM_SHA256_P256(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMP256_AES128GCM_SHA256_P256).expect("Ciphersuite not supported."), &OpenMlsRustCrypto::default()),
    case::rust_crypto_MLS10_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519).expect("Ciphersuite not supported."), &OpenMlsRustCrypto::default()),
    case::evercrypt_MLS10_128_DHKEMX25519_AES128GCM_SHA256_Ed25519(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMX25519_AES128GCM_SHA256_Ed25519).expect("Ciphersuite not supported."), &evercrypt_backend::OpenMlsEvercrypt::default()),
    case::evercrypt_MLS10_128_DHKEMP256_AES128GCM_SHA256_P256(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMP256_AES128GCM_SHA256_P256).expect("Ciphersuite not supported."), &evercrypt_backend::OpenMlsEvercrypt::default()),
    case::evercrypt_MLS10_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519(Config::ciphersuite(CiphersuiteName::MLS10_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519).expect("Ciphersuite not supported."), &evercrypt_backend::OpenMlsEvercrypt::default()),
  )
]
pub fn ciphersuites_and_backends(ciphersuite: &Ciphersuite, backend: &impl OpenMlsCryptoProvider) {}

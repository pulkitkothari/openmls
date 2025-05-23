//! This module contains tests regarding the use of [`MessageSecretsStore`]

use crate::{
    group::past_secrets::MessageSecretsStore, group::GroupEpoch, schedule::MessageSecrets,
    test_utils::*,
};

#[apply(ciphersuites_and_backends)]
fn test_secret_tree_store(ciphersuite: &'static Ciphersuite, backend: &impl OpenMlsCryptoProvider) {
    // Create a store that keeps up to 3 epochs
    let mut message_secrets_store =
        MessageSecretsStore::new_with_secret(3, MessageSecrets::random(ciphersuite, backend));

    // Add message secrets to the store
    message_secrets_store.add(GroupEpoch(0), MessageSecrets::random(ciphersuite, backend));

    // Make sure we can access the message secrets we just stored
    assert!(message_secrets_store
        .secrets_for_epoch_mut(GroupEpoch(0))
        .is_some());

    // Add 5 more message secrets, this should drop trees from earlier epochs
    for i in 1..6u64 {
        message_secrets_store.add(GroupEpoch(i), MessageSecrets::random(ciphersuite, backend));
    }

    // These epochs should be in the store
    assert!(message_secrets_store
        .secrets_for_epoch_mut(GroupEpoch(3))
        .is_some());
    assert!(message_secrets_store
        .secrets_for_epoch_mut(GroupEpoch(4))
        .is_some());
    assert!(message_secrets_store
        .secrets_for_epoch_mut(GroupEpoch(5))
        .is_some());

    // These epochs should not be in the store
    assert!(message_secrets_store
        .secrets_for_epoch_mut(GroupEpoch(0))
        .is_none());
    assert!(message_secrets_store
        .secrets_for_epoch_mut(GroupEpoch(1))
        .is_none());
    assert!(message_secrets_store
        .secrets_for_epoch_mut(GroupEpoch(2))
        .is_none());
    assert!(message_secrets_store
        .secrets_for_epoch_mut(GroupEpoch(6))
        .is_none());
}

#[apply(ciphersuites_and_backends)]
fn test_empty_secret_tree_store(
    ciphersuite: &'static Ciphersuite,
    backend: &impl OpenMlsCryptoProvider,
) {
    // Create a store that keeps no epochs
    let mut message_secrets_store =
        MessageSecretsStore::new_with_secret(0, MessageSecrets::random(ciphersuite, backend));

    // Add message secrets to the store
    message_secrets_store.add(GroupEpoch(0), MessageSecrets::random(ciphersuite, backend));

    // Make sure we cannot access the message secrets we just stored
    assert!(message_secrets_store
        .secrets_for_epoch_mut(GroupEpoch(0))
        .is_none());
}

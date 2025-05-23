use openmls_traits::types::CryptoError;
use tls_codec::{Error as TlsCodecError, Serialize, TlsSerialize, TlsSize};

use crate::ciphersuite::*;
use crate::framing::*;
use crate::schedule::*;
use crate::tree::{index::*, sender_ratchet::*, treemath::*};

use super::*;

implement_error! {
    pub enum SecretTreeError {
        Simple {
            TooDistantInThePast = "Generation is too old to be processed.",
            TooDistantInTheFuture = "Generation is too far in the future to be processed.",
            IndexOutOfBounds = "Index out of bounds",
            LibraryError = "An unrecoverable error has occurred due to a bug in the implementation.",
        }
        Complex {
            CodecError(TlsCodecError) =
                "TLS (de)serialization error occurred.",
            CryptoError(CryptoError) =
                "See [`CryptoError`](openmls_traits::types::CryptoError) for details.",
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum SecretType {
    HandshakeSecret,
    ApplicationSecret,
}

impl From<&ContentType> for SecretType {
    fn from(content_type: &ContentType) -> SecretType {
        match content_type {
            ContentType::Application => SecretType::ApplicationSecret,
            ContentType::Commit => SecretType::HandshakeSecret,
            ContentType::Proposal => SecretType::HandshakeSecret,
        }
    }
}

impl From<&MlsPlaintext> for SecretType {
    fn from(mls_plaintext: &MlsPlaintext) -> SecretType {
        SecretType::from(&mls_plaintext.content_type())
    }
}

/// Derives secrets for inner nodes of a SecretTree
pub(crate) fn derive_tree_secret(
    secret: &Secret,
    label: &str,
    node: u32,
    generation: u32,
    length: usize,
    backend: &impl OpenMlsCryptoProvider,
) -> Result<Secret, SecretTreeError> {
    log::debug!(
        "Derive tree secret with label \"{}\" for node {} in generation {} of length {}",
        label,
        node,
        generation,
        length
    );
    let tree_context = TreeContext { node, generation };
    log_crypto!(trace, "Input secret {:x?}", secret.as_slice());
    log_crypto!(trace, "Tree context {:?}", tree_context);
    let serialized_tree_context = tree_context.tls_serialize_detached()?;
    Ok(secret.kdf_expand_label(backend, label, &serialized_tree_context, length)?)
}

#[derive(Debug, TlsSerialize, TlsSize)]
pub struct TreeContext {
    pub(crate) node: u32,
    pub(crate) generation: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, TlsSerialize, TlsSize)]
#[cfg_attr(any(feature = "test-utils", test), derive(PartialEq))]
pub(crate) struct SecretTreeNode {
    pub(crate) secret: Secret,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(any(feature = "test-utils", test), derive(PartialEq, Clone))]
pub struct SecretTree {
    nodes: Vec<Option<SecretTreeNode>>,
    handshake_sender_ratchets: Vec<Option<SenderRatchet>>,
    application_sender_ratchets: Vec<Option<SenderRatchet>>,
    size: SecretTreeLeafIndex,
}

impl SecretTree {
    /// Creates a new SecretTree based on an `encryption_secret` and group size
    /// `size`. The inner nodes of the tree and the SenderRatchets only get
    /// initialized when secrets are requested either through `secret()`
    /// or `next_secret()`.
    pub(crate) fn new(encryption_secret: EncryptionSecret, size: SecretTreeLeafIndex) -> Self {
        let root = root(size);
        let num_indices = SecretTreeNodeIndex::from(size).as_usize() - 1;
        let mut nodes = vec![None; num_indices];
        nodes[root.as_usize()] = Some(SecretTreeNode {
            secret: encryption_secret.consume_secret(),
        });

        SecretTree {
            nodes,
            handshake_sender_ratchets: vec![None; size.as_usize()],
            application_sender_ratchets: vec![None; size.as_usize()],
            size,
        }
    }

    /// Get current generation for a specific SenderRatchet
    #[cfg(test)]
    pub(crate) fn generation(&self, index: SecretTreeLeafIndex, secret_type: SecretType) -> u32 {
        match self
            .ratchet_opt(index, secret_type)
            .expect("Index out of bounds.")
        {
            Some(sender_ratchet) => sender_ratchet.generation(),
            None => 0,
        }
    }

    /// Initializes a specific SenderRatchet pair for a given index by
    /// calculating and deleting the appropriate values in the SecretTree
    fn initialize_sender_ratchets(
        &mut self,
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        index: SecretTreeLeafIndex,
    ) -> Result<(), SecretTreeError> {
        log::trace!(
            "Initializing sender ratchets for {:?} with {}",
            index,
            ciphersuite
        );
        if index >= self.size {
            log::error!("Index is larger than the tree size.");
            return Err(SecretTreeError::IndexOutOfBounds);
        }
        // Check if SenderRatchets are already initialized
        if self
            .ratchet_opt(index, SecretType::HandshakeSecret)
            .expect("Index out of bounds.")
            .is_some()
            && self
                .ratchet_opt(index, SecretType::ApplicationSecret)
                .expect("Index out of bounds.")
                .is_some()
        {
            log::trace!("The sender ratchets are initialized already.");
            return Ok(());
        }
        // Calculate direct path
        let index_in_tree = SecretTreeNodeIndex::from(index);
        let mut dir_path = vec![index_in_tree];
        dir_path.extend(
            leaf_direct_path(index, self.size)
                .expect("initialize_sender_ratchets: Error while computing direct path."),
        );
        log::trace!("Direct path for leaf {:?}: {:?}", index, dir_path);
        let mut empty_nodes: Vec<SecretTreeNodeIndex> = vec![];
        for n in dir_path {
            empty_nodes.push(n);
            if self.nodes[n.as_usize()].is_some() {
                break;
            }
        }
        // Remove leaf and invert direct path
        empty_nodes.remove(0);
        empty_nodes.reverse();
        // Find empty nodes
        for n in empty_nodes {
            self.derive_down(ciphersuite, backend, n)?;
        }
        // Calculate node secret and initialize SenderRatchets
        let node_secret = match &self.nodes[index_in_tree.as_usize()] {
            Some(node) => &node.secret,
            // We just derived all necessary nodes so this should not happen
            None => return Err(SecretTreeError::LibraryError),
        };

        let handshake_ratchet_secret = derive_tree_secret(
            node_secret,
            "handshake",
            index_in_tree.as_u32(),
            0,
            ciphersuite.hash_length(),
            backend,
        )?;
        let handshake_sender_ratchet = SenderRatchet::new(index, &handshake_ratchet_secret);
        self.handshake_sender_ratchets[index.as_usize()] = Some(handshake_sender_ratchet);
        let application_ratchet_secret = derive_tree_secret(
            node_secret,
            "application",
            index_in_tree.as_u32(),
            0,
            ciphersuite.hash_length(),
            backend,
        )?;
        let application_sender_ratchet = SenderRatchet::new(index, &application_ratchet_secret);
        self.application_sender_ratchets[index.as_usize()] = Some(application_sender_ratchet);
        // Delete leaf node
        self.nodes[index_in_tree.as_usize()] = None;
        Ok(())
    }

    /// Return RatchetSecrets for a given index and generation. This should be
    /// called when decrypting an MlsCiphertext received from another member.
    /// Returns an error if index or generation are out of bound.
    pub(crate) fn secret_for_decryption(
        &mut self,
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        index: SecretTreeLeafIndex,
        secret_type: SecretType,
        generation: u32,
        configuration: &SenderRatchetConfiguration,
    ) -> Result<RatchetSecrets, SecretTreeError> {
        log::debug!(
            "Generating {:?} decryption secret for {:?} in generation {} with {}",
            secret_type,
            index,
            generation,
            ciphersuite,
        );
        // Check tree bounds
        if index >= self.size {
            return Err(SecretTreeError::IndexOutOfBounds);
        }
        if self.ratchet_opt(index, secret_type)?.is_none() {
            self.initialize_sender_ratchets(ciphersuite, backend, index)?;
        }
        let sender_ratchet = self.ratchet_mut(index, secret_type);
        sender_ratchet.secret_for_decryption(ciphersuite, backend, generation, configuration)
    }

    /// Return the next RatchetSecrets that should be used for encryption and
    /// then increments the generation.
    pub(crate) fn secret_for_encryption(
        &mut self,
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        index: SecretTreeLeafIndex,
        secret_type: SecretType,
    ) -> Result<(u32, RatchetSecrets), SecretTreeError> {
        if self.ratchet_opt(index, secret_type)?.is_none() {
            self.initialize_sender_ratchets(ciphersuite, backend, index)
                .expect("Index out of bounds");
        }
        let sender_ratchet = self.ratchet_mut(index, secret_type);
        sender_ratchet.secret_for_encryption(ciphersuite, backend)
    }

    /// Returns a mutable reference to a specific SenderRatchet. The
    /// SenderRatchet needs to be initialized.
    fn ratchet_mut(
        &mut self,
        index: SecretTreeLeafIndex,
        secret_type: SecretType,
    ) -> &mut SenderRatchet {
        let sender_ratchets = match secret_type {
            SecretType::HandshakeSecret => &mut self.handshake_sender_ratchets,
            SecretType::ApplicationSecret => &mut self.application_sender_ratchets,
        };
        sender_ratchets
            .get_mut(index.as_usize())
            .unwrap_or_else(|| panic!("SenderRatchets not initialized: {}", index.as_usize()))
            .as_mut()
            .expect("SecretTree not initialized")
    }

    /// Returns an optional reference to a specific SenderRatchet
    fn ratchet_opt(
        &self,
        index: SecretTreeLeafIndex,
        secret_type: SecretType,
    ) -> Result<Option<&SenderRatchet>, SecretTreeError> {
        let sender_ratchets = match secret_type {
            SecretType::HandshakeSecret => &self.handshake_sender_ratchets,
            SecretType::ApplicationSecret => &self.application_sender_ratchets,
        };
        match sender_ratchets.get(index.as_usize()) {
            Some(sender_ratchet_option) => Ok(sender_ratchet_option.as_ref()),
            None => Err(SecretTreeError::IndexOutOfBounds),
        }
    }

    /// Derives the secrets for the child leaves in a SecretTree and blanks the
    /// parent leaf.
    fn derive_down(
        &mut self,
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        index_in_tree: SecretTreeNodeIndex,
    ) -> Result<(), SecretTreeError> {
        log::debug!(
            "Deriving tree secret for node {} with {}",
            index_in_tree.as_u32(),
            ciphersuite
        );
        let hash_len = ciphersuite.hash_length();
        let node_secret = match &self.nodes[index_in_tree.as_usize()] {
            Some(node) => &node.secret,
            // This function only gets called top to bottom, so this should not happen
            None => return Err(SecretTreeError::LibraryError),
        };
        log_crypto!(trace, "Node secret: {:x?}", node_secret.as_slice());
        let left_index =
            left(index_in_tree).expect("derive_down: Error while computing left child.");
        let right_index = right(index_in_tree, self.size)
            .expect("derive_down: Error while computing right child.");
        let left_secret = derive_tree_secret(
            node_secret,
            "tree",
            left_index.as_u32(),
            0,
            hash_len,
            backend,
        )?;
        let right_secret = derive_tree_secret(
            node_secret,
            "tree",
            right_index.as_u32(),
            0,
            hash_len,
            backend,
        )?;
        log_crypto!(
            trace,
            "Left node ({}) secret: {:x?}",
            left_index.as_u32(),
            left_secret.as_slice()
        );
        log_crypto!(
            trace,
            "Right node ({}) secret: {:x?}",
            right_index.as_u32(),
            right_secret.as_slice()
        );
        self.nodes[left_index.as_usize()] = Some(SecretTreeNode {
            secret: left_secret,
        });
        self.nodes[right_index.as_usize()] = Some(SecretTreeNode {
            secret: right_secret,
        });
        self.nodes[index_in_tree.as_usize()] = None;
        Ok(())
    }
}

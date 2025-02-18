use openmls_traits::OpenMlsCryptoProvider;
use tls_codec::{
    Deserialize, Serialize, Size, TlsByteSliceU16, TlsByteVecU16, TlsByteVecU32, TlsByteVecU8,
    TlsDeserialize, TlsSerialize, TlsSize,
};

use crate::tree::{secret_tree::SecretType, sender_ratchet::SenderRatchetConfiguration};

use super::*;

/// `MlsCiphertext` is the framing struct for an encrypted `MlsPlaintext`.
/// This message format is meant to be sent to and received from the Delivery
/// Service.
#[derive(Debug, PartialEq, Clone, TlsSerialize, TlsSize)]
pub(crate) struct MlsCiphertext {
    wire_format: WireFormat,
    group_id: GroupId,
    epoch: GroupEpoch,
    content_type: ContentType,
    authenticated_data: TlsByteVecU32,
    encrypted_sender_data: TlsByteVecU8,
    ciphertext: TlsByteVecU32,
}

pub(crate) struct MlsMessageHeader {
    pub(crate) group_id: GroupId,
    pub(crate) epoch: GroupEpoch,
    pub(crate) sender: LeafIndex,
}

impl MlsCiphertext {
    pub(crate) fn new(
        wire_format: WireFormat,
        group_id: GroupId,
        epoch: GroupEpoch,
        content_type: ContentType,
        authenticated_data: TlsByteVecU32,
        encrypted_sender_data: TlsByteVecU8,
        ciphertext: TlsByteVecU32,
    ) -> Self {
        Self {
            wire_format,
            group_id,
            epoch,
            content_type,
            authenticated_data,
            encrypted_sender_data,
            ciphertext,
        }
    }
    /// Try to create a new `MlsCiphertext` from an `MlsPlaintext`
    pub(crate) fn try_from_plaintext(
        mls_plaintext: &MlsPlaintext,
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        header: MlsMessageHeader,
        message_secrets: &mut MessageSecrets,
        padding_size: usize,
    ) -> Result<MlsCiphertext, MlsCiphertextError> {
        log::debug!("MlsCiphertext::try_from_plaintext");
        log::trace!("  ciphersuite: {}", ciphersuite);
        // Check the plaintext has the correct wire format
        if mls_plaintext.wire_format() != WireFormat::MlsCiphertext {
            return Err(MlsCiphertextError::WrongWireFormat);
        }
        // Serialize the content AAD
        let mls_ciphertext_content_aad = MlsCiphertextContentAad {
            group_id: header.group_id.clone(),
            epoch: header.epoch,
            content_type: mls_plaintext.content_type(),
            authenticated_data: TlsByteSliceU32(mls_plaintext.authenticated_data()),
        };
        let mls_ciphertext_content_aad_bytes =
            mls_ciphertext_content_aad.tls_serialize_detached()?;
        // Extract generation and key material for encryption
        let secret_type = SecretType::try_from(mls_plaintext)
            .map_err(|_| MlsCiphertextError::InvalidContentType)?;
        let (generation, (ratchet_key, mut ratchet_nonce)) = message_secrets
            .secret_tree_mut()
            .secret_for_encryption(ciphersuite, backend, header.sender.into(), secret_type)?;
        // Sample reuse guard uniformly at random.
        let reuse_guard: ReuseGuard = ReuseGuard::try_from_random(backend)?;
        // Prepare the nonce by xoring with the reuse guard.
        ratchet_nonce.xor_with_reuse_guard(&reuse_guard);
        // Encrypt the payload
        let ciphertext = ratchet_key
            .aead_seal(
                backend,
                &Self::encode_padded_ciphertext_content_detached(
                    mls_plaintext,
                    padding_size,
                    ciphersuite.mac_length(),
                )?,
                &mls_ciphertext_content_aad_bytes,
                &ratchet_nonce,
            )
            .map_err(|e| {
                log::error!("MlsCiphertext::try_from_plaintext encryption error {:?}", e);
                MlsCiphertextError::EncryptionError
            })?;
        // Derive the sender data key from the key schedule using the ciphertext.
        let sender_data_key = message_secrets
            .sender_data_secret()
            .derive_aead_key(backend, &ciphertext)?;
        // Derive initial nonce from the key schedule using the ciphertext.
        let sender_data_nonce = message_secrets.sender_data_secret().derive_aead_nonce(
            ciphersuite,
            backend,
            &ciphertext,
        )?;
        // Compute sender data nonce by xoring reuse guard and key schedule
        // nonce as per spec.
        let mls_sender_data_aad = MlsSenderDataAad::new(
            header.group_id.clone(),
            header.epoch,
            mls_plaintext.content_type(),
        );
        // Serialize the sender data AAD
        let mls_sender_data_aad_bytes = mls_sender_data_aad.tls_serialize_detached()?;
        let sender_data = MlsSenderData::new(mls_plaintext.sender_index(), generation, reuse_guard);
        // Encrypt the sender data
        let encrypted_sender_data = sender_data_key
            .aead_seal(
                backend,
                &sender_data.tls_serialize_detached()?,
                &mls_sender_data_aad_bytes,
                &sender_data_nonce,
            )
            .map_err(|e| {
                log::error!("MlsCiphertext::try_from_plaintext encryption error {:?}", e);
                MlsCiphertextError::EncryptionError
            })?;
        Ok(MlsCiphertext {
            wire_format: WireFormat::MlsCiphertext,
            group_id: header.group_id,
            epoch: header.epoch,
            content_type: mls_plaintext.content_type(),
            authenticated_data: mls_plaintext.authenticated_data().into(),
            encrypted_sender_data: encrypted_sender_data.into(),
            ciphertext: ciphertext.into(),
        })
    }

    /// Decrypt the sender data from this [`MlsCiphertext`].
    pub(crate) fn sender_data(
        &self,
        message_secrets: &mut MessageSecrets,
        backend: &impl OpenMlsCryptoProvider,
        ciphersuite: &Ciphersuite,
    ) -> Result<MlsSenderData, MlsCiphertextError> {
        log::debug!("Decrypting MlsCiphertext");
        // Check the ciphertext has the correct wire format
        if self.wire_format != WireFormat::MlsCiphertext {
            return Err(MlsCiphertextError::WrongWireFormat);
        }
        // Derive key from the key schedule using the ciphertext.
        let sender_data_key = message_secrets
            .sender_data_secret()
            .derive_aead_key(backend, self.ciphertext.as_slice())?;
        // Derive initial nonce from the key schedule using the ciphertext.
        let sender_data_nonce = message_secrets.sender_data_secret().derive_aead_nonce(
            ciphersuite,
            backend,
            self.ciphertext.as_slice(),
        )?;
        // Serialize sender data AAD
        let mls_sender_data_aad =
            MlsSenderDataAad::new(self.group_id.clone(), self.epoch, self.content_type);
        let mls_sender_data_aad_bytes = mls_sender_data_aad.tls_serialize_detached()?;
        // Decrypt sender data
        let sender_data_bytes = sender_data_key
            .aead_open(
                backend,
                self.encrypted_sender_data.as_slice(),
                &mls_sender_data_aad_bytes,
                &sender_data_nonce,
            )
            .map_err(|_| {
                log::error!("Sender data decryption error");
                MlsCiphertextError::DecryptionError
            })?;
        log::trace!("  Successfully decrypted sender data.");
        Ok(MlsSenderData::tls_deserialize(
            &mut sender_data_bytes.as_slice(),
        )?)
    }

    /// Decrypt this [`MlsCiphertext`] and return the [`MlsCiphertextContent`].
    #[inline]
    fn decrypt(
        &self,
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        message_secrets: &mut MessageSecrets,
        sender_ratchet_configuration: &SenderRatchetConfiguration,
        sender_data: &MlsSenderData,
    ) -> Result<MlsCiphertextContent, MlsCiphertextError> {
        let secret_type = SecretType::try_from(&self.content_type)
            .map_err(|_| MlsCiphertextError::InvalidContentType)?;
        // Extract generation and key material for encryption
        let (ratchet_key, mut ratchet_nonce) = message_secrets
            .secret_tree_mut()
            .secret_for_decryption(
                ciphersuite,
                backend,
                sender_data.sender.into(),
                secret_type,
                sender_data.generation,
                sender_ratchet_configuration,
            )
            .map_err(|_| {
                log::error!("  Ciphertext generation out of bounds");
                MlsCiphertextError::GenerationOutOfBound
            })?;
        // Prepare the nonce by xoring with the reuse guard.
        ratchet_nonce.xor_with_reuse_guard(&sender_data.reuse_guard);
        // Serialize content AAD
        let mls_ciphertext_content_aad_bytes = MlsCiphertextContentAad {
            group_id: self.group_id.clone(),
            epoch: self.epoch,
            content_type: self.content_type,
            authenticated_data: TlsByteSliceU32(self.authenticated_data.as_slice()),
        }
        .tls_serialize_detached()?;
        // Decrypt payload
        let mls_ciphertext_content_bytes = ratchet_key
            .aead_open(
                backend,
                self.ciphertext.as_slice(),
                &mls_ciphertext_content_aad_bytes,
                &ratchet_nonce,
            )
            .map_err(|_| {
                log::error!("  Ciphertext decryption error");
                MlsCiphertextError::DecryptionError
            })?;
        log_content!(
            trace,
            "  Successfully decrypted MlsPlaintext bytes: {:x?}",
            mls_ciphertext_content_bytes
        );
        Ok(MlsCiphertextContent::deserialize(
            self.content_type,
            &mut mls_ciphertext_content_bytes.as_slice(),
        )?)
    }

    /// This function decrypts an [`MlsCiphertext`] into an [`VerifiableMlsPlaintext`].
    /// In order to get an [`MlsPlaintext`] the result must be verified.
    pub(crate) fn to_plaintext(
        &self,
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        message_secrets: &mut MessageSecrets,
        sender_ratchet_configuration: &SenderRatchetConfiguration,
        sender_data: MlsSenderData,
    ) -> Result<VerifiableMlsPlaintext, MlsCiphertextError> {
        let mls_ciphertext_content = self.decrypt(
            ciphersuite,
            backend,
            message_secrets,
            sender_ratchet_configuration,
            &sender_data,
        )?;

        // Extract sender. The sender type is always of type Member for MlsCiphertext.
        let sender = Sender {
            sender_type: SenderType::Member,
            sender: sender_data.sender,
        };
        log_content!(
            trace,
            "  Successfully decoded MlsPlaintext with: {:x?}",
            mls_ciphertext_content.content
        );

        let verifiable = VerifiableMlsPlaintext::new(
            MlsPlaintextTbs::new(
                self.wire_format,
                self.group_id.clone(),
                self.epoch,
                sender,
                self.authenticated_data.clone(),
                Payload {
                    payload: mls_ciphertext_content.content,
                    content_type: self.content_type,
                },
            ),
            mls_ciphertext_content.signature,
            mls_ciphertext_content.confirmation_tag,
            None, /* MlsCiphertexts don't carry along the membership tag. */
        );
        Ok(verifiable)
    }

    /// Returns `true` if this is a handshake message and `false` otherwise.
    #[cfg(test)]
    pub(crate) fn is_handshake_message(&self) -> bool {
        self.content_type.is_handshake_message()
    }

    /// Encodes the `MLSCiphertextContent` struct with padding
    /// ```text
    /// struct {
    ///     select (MLSCiphertext.content_type) {
    ///         case application:
    ///             opaque application_data<0..2^32-1>;
    ///
    ///         case proposal:
    ///             Proposal proposal;
    ///
    ///         case commit:
    ///             Commit commit;
    ///     }
    ///
    ///     opaque signature<0..2^16-1>;
    ///     optional<MAC> confirmation_tag;
    ///     opaque padding<0..2^16-1>;
    /// } MLSCiphertextContent;
    /// ```
    fn encode_padded_ciphertext_content_detached(
        mls_plaintext: &MlsPlaintext,
        padding_size: usize,
        mac_len: usize,
    ) -> Result<Vec<u8>, tls_codec::Error> {
        // Persist all initial fields manually (avoids cloning them)
        let buffer = &mut Vec::with_capacity(
            mls_plaintext.content().tls_serialized_len()
                + mls_plaintext.signature().tls_serialized_len()
                + mls_plaintext.confirmation_tag().tls_serialized_len(),
        );
        mls_plaintext.content().tls_serialize(buffer)?;
        mls_plaintext.signature().tls_serialize(buffer)?;
        mls_plaintext.confirmation_tag().tls_serialize(buffer)?;
        // Add padding if needed
        let padding_length = if padding_size > 0 {
            // Calculate padding block size
            // The length of the padding block takes 2 bytes and the AEAD tag is also added.
            let padding_offset = buffer.len() + 2 + mac_len;
            // Return padding block size
            (padding_size - (padding_offset % padding_size)) % padding_size
        } else {
            0
        };
        TlsByteSliceU16(&vec![0u8; padding_length]).tls_serialize(buffer)?;
        Ok(buffer.to_vec())
    }

    /// Get the `group_id` in the `MlsCiphertext`.
    pub(crate) fn group_id(&self) -> &GroupId {
        &self.group_id
    }

    /// Get the cipher text bytes as slice.
    #[cfg(test)]
    pub(crate) fn ciphertext(&self) -> &[u8] {
        self.ciphertext.as_slice()
    }

    /// Get the `epoch` in the `MlsCiphertext`.
    pub(crate) fn epoch(&self) -> GroupEpoch {
        self.epoch
    }

    /// Get the `content_type` in the `MlsCiphertext`.
    pub(crate) fn content_type(&self) -> ContentType {
        self.content_type
    }

    /// Set the wire format.
    #[cfg(test)]
    pub(super) fn set_wire_format(&mut self, wire_format: WireFormat) {
        self.wire_format = wire_format;
    }

    /// Set the ciphertext.
    #[cfg(test)]
    pub(crate) fn set_ciphertext(&mut self, ciphertext: Vec<u8>) {
        self.ciphertext = ciphertext.into();
    }
}

// === Helper structs ===

#[derive(Clone, TlsDeserialize, TlsSerialize, TlsSize)]
#[cfg_attr(test, derive(Debug))]
pub(crate) struct MlsSenderData {
    // TODO: #541 replace sender with [`KeyPackageRef`]
    pub(crate) sender: LeafIndex,
    pub(crate) generation: u32,
    pub(crate) reuse_guard: ReuseGuard,
}

impl MlsSenderData {
    pub(crate) fn new(sender: LeafIndex, generation: u32, reuse_guard: ReuseGuard) -> Self {
        MlsSenderData {
            sender,
            generation,
            reuse_guard,
        }
    }
}

#[derive(Clone, TlsDeserialize, TlsSerialize, TlsSize)]
pub(crate) struct MlsSenderDataAad {
    pub(crate) group_id: GroupId,
    pub(crate) epoch: GroupEpoch,
    pub(crate) content_type: ContentType,
}

impl MlsSenderDataAad {
    fn new(group_id: GroupId, epoch: GroupEpoch, content_type: ContentType) -> Self {
        Self {
            group_id,
            epoch,
            content_type,
        }
    }
}

#[derive(Debug, Clone, TlsSerialize, TlsSize)]
pub(crate) struct MlsCiphertextContent {
    pub(crate) content: MlsPlaintextContentType,
    pub(crate) signature: Signature,
    pub(crate) confirmation_tag: Option<ConfirmationTag>,
    pub(crate) padding: TlsByteVecU16,
}

#[derive(TlsSerialize, TlsSize)]
pub(crate) struct MlsCiphertextContentAad<'a> {
    pub(crate) group_id: GroupId,
    pub(crate) epoch: GroupEpoch,
    pub(crate) content_type: ContentType,
    pub(crate) authenticated_data: TlsByteSliceU32<'a>,
}

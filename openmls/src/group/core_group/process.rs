use core_group::{proposals::QueuedProposal, staged_commit::StagedCommit};

use super::{proposals::ProposalStore, *};

impl CoreGroup {
    /// This function is used to parse messages from the DS.
    /// It checks for syntactic errors and makes some semantic checks as well.
    /// If the input is a [MlsCiphertext] message, it will be decrypted.
    /// Returns an [UnverifiedMessage] that can be inspected and later processed in
    /// [Self::process_unverified_message()].
    /// Checks the following semantic validation:
    ///  - ValSem002
    ///  - ValSem003
    ///  - ValSem004
    ///  - ValSem005
    ///  - ValSem006
    ///  - ValSem007
    ///  - ValSem009
    pub(crate) fn parse_message(
        &mut self,
        backend: &impl OpenMlsCryptoProvider,
        message: MlsMessageIn,
        sender_ratchet_configuration: &SenderRatchetConfiguration,
    ) -> Result<UnverifiedMessage, CoreGroupError> {
        // Checks the following semantic validation:
        //  - ValSem002
        //  - ValSem003
        self.validate_framing(&message)?;

        // Checks the following semantic validation:
        //  - ValSem006
        let decrypted_message = match message.wire_format() {
            WireFormat::MlsPlaintext => DecryptedMessage::from_inbound_plaintext(message)?,
            WireFormat::MlsCiphertext => {
                // If the message is older than the current epoch, we need to fetch the correct secret tree first
                let ciphersuite = self.ciphersuite();
                let message_secrets = self.message_secrets_mut(message.epoch())?;
                DecryptedMessage::from_inbound_ciphertext(
                    message,
                    ciphersuite,
                    backend,
                    message_secrets,
                    sender_ratchet_configuration,
                )?
            }
        };

        // Checks the following semantic validation:
        //  - ValSem004
        //  - ValSem005
        //  - ValSem007
        //  - ValSem009
        self.validate_plaintext(decrypted_message.plaintext())?;

        // Extract the credential if the sender is a member or a new member.
        // Checks the following semantic validation:
        //  - ValSem245
        //  - ValSem246
        //  - Prepares ValSem247 by setting the right credential. The remainder
        //    of ValSem247 is validated as part of ValSem010.
        // Preconfigured senders are not supported yet #106/#151.
        let credential = decrypted_message.credential(self.treesync())?;

        Ok(UnverifiedMessage::from_decrypted_message(
            decrypted_message,
            Some(credential),
        ))
    }

    /// This processing function does most of the semantic verifications.
    /// It returns a [ProcessedMessage] enum.
    /// Checks the following semantic validation:
    ///  - ValSem008
    ///  - ValSem010
    ///  - ValSem100
    ///  - ValSem101
    ///  - ValSem102
    ///  - ValSem103
    ///  - ValSem104
    ///  - ValSem105
    ///  - ValSem106
    ///  - ValSem107
    ///  - ValSem108
    ///  - ValSem109
    ///  - ValSem110
    pub(crate) fn process_unverified_message(
        &mut self,
        unverified_message: UnverifiedMessage,
        signature_key: Option<&SignaturePublicKey>,
        proposal_store: &ProposalStore,
        own_kpbs: &[KeyPackageBundle],
        backend: &impl OpenMlsCryptoProvider,
    ) -> Result<ProcessedMessage, CoreGroupError> {
        // Add the context to the message and verify the membership tag if necessary.
        // If the message is older than the current epoch, we need to fetch the correct secret tree first.
        let message_secrets = self.message_secrets_mut(unverified_message.epoch())?;

        // Checks the following semantic validation:
        //  - ValSem008
        let context_plaintext = UnverifiedContextMessage::from_unverified_message(
            unverified_message,
            message_secrets,
            backend,
        )?;

        match context_plaintext {
            UnverifiedContextMessage::Group(unverified_message) => {
                // Checks the following semantic validation:
                //  - ValSem010
                let verified_member_message =
                    unverified_message.into_verified(backend, signature_key)?;

                Ok(match verified_member_message.plaintext().content() {
                    MlsPlaintextContentType::Application(application_message) => {
                        ProcessedMessage::ApplicationMessage(ApplicationMessage::new(
                            application_message.as_slice().to_vec(),
                            *verified_member_message.plaintext().sender(),
                        ))
                    }
                    MlsPlaintextContentType::Proposal(_proposal) => {
                        ProcessedMessage::ProposalMessage(Box::new(
                            QueuedProposal::from_mls_plaintext(
                                self.ciphersuite(),
                                backend,
                                verified_member_message.take_plaintext(),
                            )?,
                        ))
                    }
                    MlsPlaintextContentType::Commit(_commit) => {
                        //  - ValSem100
                        //  - ValSem101
                        //  - ValSem102
                        //  - ValSem103
                        //  - ValSem104
                        //  - ValSem105
                        //  - ValSem106
                        //  - ValSem107
                        //  - ValSem108
                        //  - ValSem109
                        //  - ValSem110
                        let staged_commit = self.stage_commit(
                            verified_member_message.plaintext(),
                            proposal_store,
                            own_kpbs,
                            backend,
                        )?;
                        ProcessedMessage::StagedCommitMessage(Box::new(staged_commit))
                    }
                })
            }
            UnverifiedContextMessage::Preconfigured(external_message) => {
                // Signature verification
                if let Some(signature_public_key) = signature_key {
                    let _verified_external_message =
                        external_message.into_verified(backend, signature_public_key)?;
                } else {
                    return Err(CoreGroupError::NoSignatureKey);
                }

                // We don't support external messages from preconfigured senders yet
                // TODO #151/#106
                todo!()
            }
        }
    }

    /// Merge a [StagedCommit] into the group after inspection
    pub(crate) fn merge_staged_commit(
        &mut self,
        staged_commit: StagedCommit,
        proposal_store: &mut ProposalStore,
    ) -> Result<(), CoreGroupError> {
        // Save the past epoch
        let past_epoch = self.context().epoch();
        // Merge the staged commit into the group state and store the secret tree from the
        // previous epoch in the message secrets store.
        if let Some(message_secrets) = self.merge_commit(staged_commit)? {
            self.message_secrets_store.add(past_epoch, message_secrets);
        }
        // Empty the proposal store
        proposal_store.empty();
        Ok(())
    }
}

mod application;
pub mod config;
mod creation;
pub mod errors;
mod exporting;
mod membership;
pub mod processing;
mod resumption;
mod ser;
#[cfg(test)]
mod test_mls_group;
mod updates;

use crate::credentials::CredentialBundle;
use crate::{treesync::node::Node, treesync::LeafIndex};

use openmls_traits::{key_store::OpenMlsKeyStore, OpenMlsCryptoProvider};

use crate::{
    ciphersuite::signable::Signable,
    credentials::Credential,
    framing::*,
    group::*,
    key_packages::KeyPackageBundlePayload,
    key_packages::{KeyPackage, KeyPackageBundle},
    messages::{proposals::*, Welcome},
    schedule::ResumptionSecret,
};

use std::io::{Error, Read, Write};

pub use config::*;
pub use errors::{
    EmptyInputError, InvalidMessageError, MlsGroupError, PendingProposalsError, UseAfterEviction,
};
pub(crate) use resumption::ResumptionSecretStore;
use ser::*;

use super::proposals::{ProposalStore, QueuedProposal};
use super::staged_commit::StagedCommit;

#[derive(Debug, Serialize, Deserialize)]
pub enum PendingCommitState {
    Member(StagedCommit),
    External(StagedCommit),
}

impl PendingCommitState {
    /// Return a reference to the [`StagedCommit`] contained in the
    /// [`PendingCommitState`] enum.
    pub(crate) fn staged_commit(&self) -> &StagedCommit {
        match self {
            PendingCommitState::Member(pc) => pc,
            PendingCommitState::External(pc) => pc,
        }
    }
}

impl From<PendingCommitState> for StagedCommit {
    fn from(pcs: PendingCommitState) -> Self {
        match pcs {
            PendingCommitState::Member(pc) => pc,
            PendingCommitState::External(pc) => pc,
        }
    }
}

/// [`MlsGroupState`] determines the state of an [`MlsGroup`]. The different
/// states and their transitions are as follows:
///
/// * [`MlsGroupState::Operational`]: This is the main state of the group, which
/// allows access to all of its functionality, (except merging pending commits,
/// see the [`MlsGroupState::PendingCommit`] for more information) and it's the
/// state the group starts in (except when created via
/// [`MlsGroup::join_by_external_commit()`], see the functions documentation for
/// more information). From this `Operational`, the group state can either
/// transition to [`MlsGroupState::Inactive`], when it processes a commit that
/// removes this client from the group, or to [`MlsGroupState::PendingCommit`],
/// when this client creates a commit.
///
/// * [`MlsGroupState::Inactive`]: A group can enter this state from any other
/// state when it processes a commit that removes this client from the group.
/// This is a terminal state that the group can not exit from. If the clients
/// wants to re-join the group, it can either be added by a group member or it
/// can join via external commit.
///
/// * [`MlsGroupState:PendingCommit`]: This state is split into two possible
/// sub-states, one for each
/// [`CommitType`](crate::group::core_group::create_commit_params::CommitType):
/// [`PendingCommitState::Member`] and [`PendingCommitState::Member`]:
///
///   * If the client creates a commit for this group, the `PendingCommit` state
///   is entered with [`PendingCommitState::Member`] and with the [`StagedCommit`] as
///   additional state variable. In this state, it can perform the same
///   operations as in the [`MlsGroupState::Operational`], except that it cannot
///   create proposals or commits. However, it can merge or clear the stored
///   [`StagedCommit`], where both actions result in a transition to the
///   [`MlsGroupState::Operational`]. Additionally, if a commit from another
///   group member is processed, the own pending commit is also cleared and
///   either the `Inactive` state is entered (if this client was removed from
///   the group as part of the processed commit), or the `Operational` state is
///   entered.
///
///   * A group can enter the [`PendingCommitState::External`] sub-state only as
///   the initial state when the group is created via
///   [`MlsGroup::join_by_external_commit()`]. In contrast to the
///   [`PendingCommitState::Member`] `PendingCommit` state, the only possible
///   functionality that can be used is the [`MlsGroup::merge_pending_commit()`]
///   function, which merges the pending external commit and transitions the
///   state to [`MlsGroupState::PendingCommit`]. For more information on the
///   external commit process, see [`MlsGroup::join_by_external_commit()`] or
///   Section 11.2.1 of the MLS specification.
#[derive(Debug, Serialize, Deserialize)]
pub enum MlsGroupState {
    PendingCommit(Box<PendingCommitState>),
    Operational,
    Inactive,
}

/// A `MlsGroup` represents an [CoreGroup] with
/// an easier, high-level API designed to be used in production. The API exposes
/// high level functions to manage a group by adding/removing members, get the
/// current member list, etc.
///
/// The API is modeled such that it can serve as a direct interface to the
/// Delivery Service. Functions that modify the public state of the group will
/// return a `Vec<MLSMessage>` that can be sent to the Delivery
/// Service directly. Conversely, incoming messages from the Delivery Service
/// can be fed into [process_message()](`MlsGroup::process_message()`).
///
/// A `MlsGroup` has an internal queue of pending proposals that builds up
/// as new messages are processed. When creating proposals, those messages are
/// not automatically appended to this queue, instead they have to be processed
/// again through [process_message()](`MlsGroup::process_message()`). This
/// allows the Delivery Service to reject them (e.g. if they reference the wrong
/// epoch).
///
/// If incoming messages or applied operations are semantically or syntactically
/// incorrect, an error event will be returned with a corresponding error
/// message and the state of the group will remain unchanged.
///
/// The application policy for the group can be enforced by implementing the
/// validator callback functions and selectively allowing/ disallowing each
/// operation (see [`MlsGroupCallbacks`])
///
/// Changes to the group state are dispatched as events through callback
/// functions (see [`MlsGroupCallbacks`]).
///
/// An `MlsGroup` has an internal state variable determining if it is active or
/// inactive, as well as if it has a pending commit. See [`MlsGroupState`] for
/// more information.
#[derive(Debug)]
pub struct MlsGroup {
    // The group configuration. See `MlsGroupCongig` for more information.
    mls_group_config: MlsGroupConfig,
    // the internal `CoreGroup` used for lower level operations. See `CoreGroup` for more
    // information.
    group: CoreGroup,
    // A [ProposalStore] that stores incoming proposals from the DS within one epoch.
    // The store is emptied after every epoch change.
    proposal_store: ProposalStore,
    // Own `KeyPackageBundle`s that were created for update proposals and that
    // are needed in case an update proposal is commited by another group
    // member. The vector is emptied after every epoch change.
    own_kpbs: Vec<KeyPackageBundle>,
    // The AAD that is used for all outgoing handshake messages. The AAD can be set through
    // `set_aad()`.
    aad: Vec<u8>,
    // Resumption secret store. This is where the resumption secrets are kept in a rollover list.
    resumption_secret_store: ResumptionSecretStore,
    // A variable that indicates the state of the group. See [`MlsGroupState`]
    // for more information.
    group_state: MlsGroupState,
    // A flag that indicates if the group state has changed and needs to be persisted again. The value
    // is set to `InnerState::Changed` whenever an the internal group state is change and is set to
    // `InnerState::Persisted` once the state has been persisted.
    state_changed: InnerState,
}

impl MlsGroup {
    // === Configuration ===

    /// Gets the configuration
    pub fn configuration(&self) -> &MlsGroupConfig {
        &self.mls_group_config
    }

    /// Sets the configuration
    pub fn set_configuration(&mut self, mls_group_config: &MlsGroupConfig) {
        self.mls_group_config = mls_group_config.clone();

        // Since the state of the group might be changed, arm the state flag
        self.flag_state_change();
    }

    /// Gets the AAD used in the framing
    pub fn aad(&self) -> &[u8] {
        &self.aad
    }

    /// Sets the AAD used in the framing
    pub fn set_aad(&mut self, aad: &[u8]) {
        self.aad = aad.to_vec();

        // Since the state of the group might be changed, arm the state flag
        self.flag_state_change();
    }

    // === Advanced functions ===

    /// Returns the group's ciphersuite
    pub fn ciphersuite(&self) -> &Ciphersuite {
        self.group.ciphersuite()
    }

    /// Returns whether the own client is still a member of the group or if it
    /// was already evicted
    pub fn is_active(&self) -> bool {
        !matches!(self.group_state, MlsGroupState::Inactive)
    }

    /// Returns own credential. If the group is inactive, it returns a
    /// `UseAfterEviction` error.
    pub fn credential(&self) -> Result<&Credential, MlsGroupError> {
        if !self.is_active() {
            return Err(MlsGroupError::UseAfterEviction(UseAfterEviction::Error));
        }
        let tree = self.group.treesync();
        Ok(tree.own_leaf_node()?.key_package().credential())
    }

    /// Get group ID
    pub fn group_id(&self) -> &GroupId {
        self.group.group_id()
    }

    /// Returns an `Iterator` over staged proposals.
    pub fn pending_proposals(&self) -> impl Iterator<Item = &QueuedProposal> {
        self.proposal_store.proposals()
    }

    /// Returns a reference to the [`StagedCommit`] of the most recently created
    /// commit. If there was no commit created in this epoch, either because
    /// this commit or another commit was merged, it returns `None`.
    pub fn pending_commit(&self) -> Option<&StagedCommit> {
        match self.group_state {
            MlsGroupState::PendingCommit(ref pending_commit_state) => {
                Some(pending_commit_state.staged_commit())
            }
            MlsGroupState::Operational => None,
            MlsGroupState::Inactive => None,
        }
    }

    /// Sets the `group_state` to [`MlsGroupState::Operational`], thus clearing
    /// any potentially pending commits.
    ///
    /// Returns an error if the group was created through an external commit and
    /// the resulting external commit has not been merged yet. For more
    /// information, see [`MlsGroup::join_by_external_commit()`].
    ///
    /// Use with caution! This function should only be used if it is clear that
    /// the pending commit will not be used in the group. In particular, if a
    /// pending commit is later accepted by the group, this client will lack the
    /// key material to encrypt or decrypt group messages.
    pub fn clear_pending_commit(&mut self) -> Result<(), MlsGroupError> {
        match self.group_state {
            MlsGroupState::PendingCommit(ref pending_commit_state) => {
                match **pending_commit_state {
                    PendingCommitState::Member(_) => self.group_state = MlsGroupState::Operational,
                    PendingCommitState::External(_) => {
                        return Err(MlsGroupError::ExternalCommitError)
                    }
                }
            }
            MlsGroupState::Operational | MlsGroupState::Inactive => (),
        }
        Ok(())
    }

    // === Load & save ===

    /// Loads the state from persisted state
    pub fn load<R: Read>(reader: R) -> Result<MlsGroup, Error> {
        let serialized_mls_group: SerializedMlsGroup = serde_json::from_reader(reader)?;
        Ok(serialized_mls_group.into_mls_group())
    }

    /// Persists the state
    pub fn save<W: Write>(&mut self, writer: &mut W) -> Result<(), Error> {
        let serialized_mls_group = serde_json::to_string_pretty(self)?;
        writer.write_all(&serialized_mls_group.into_bytes())?;
        self.state_changed = InnerState::Persisted;
        Ok(())
    }

    /// Returns `true` if the internal state has changed and needs to be persisted and
    /// `false` otherwise. Calling [save()] resets the value to `false`.
    pub fn state_changed(&self) -> InnerState {
        self.state_changed
    }

    // === Extensions ===

    /// Export the Ratchet Tree
    pub fn export_ratchet_tree(&self) -> Vec<Option<Node>> {
        self.group.treesync().export_nodes()
    }

    #[cfg(any(feature = "test-utils", test))]
    pub fn export_group_context(&self) -> &GroupContext {
        self.group.context()
    }

    #[cfg(any(feature = "test-utils", test))]
    pub fn tree_hash(&self) -> &[u8] {
        self.group.treesync().tree_hash()
    }

    #[cfg(any(feature = "test-utils", test))]
    pub fn print_tree(&self, message: &str) {
        print_tree(self.group.treesync(), message)
    }

    /// Get the underlying [CoreGroup].
    #[cfg(test)]
    pub(crate) fn group(&self) -> &CoreGroup {
        &self.group
    }
}

// Private methods of MlsGroup
impl MlsGroup {
    /// Converts MlsPlaintext to MlsMessageOut. Depending on whether handshake
    /// message should be encrypted, MlsPlaintext messages are encrypted to
    /// MlsCiphertext first.
    fn plaintext_to_mls_message(
        &mut self,
        plaintext: MlsPlaintext,
        backend: &impl OpenMlsCryptoProvider,
    ) -> Result<MlsMessageOut, MlsGroupError> {
        let msg = match self.configuration().wire_format_policy().outgoing() {
            OutgoingWireFormatPolicy::AlwaysPlaintext => MlsMessageOut::from(plaintext),
            OutgoingWireFormatPolicy::AlwaysCiphertext => {
                let ciphertext =
                    self.group
                        .encrypt(plaintext, self.configuration().padding_size(), backend)?;
                MlsMessageOut::from(ciphertext)
            }
        };
        Ok(msg)
    }

    /// Arm the state changed flag function
    fn flag_state_change(&mut self) {
        self.state_changed = InnerState::Changed;
    }

    /// Group framing parameters
    fn framing_parameters(&self) -> FramingParameters {
        FramingParameters::new(
            &self.aad,
            self.mls_group_config.wire_format_policy().outgoing(),
        )
    }

    /// Check if the group is operational. Throws an error if the group is
    /// inactive or if there is a pending commit.
    fn is_operational(&self) -> Result<(), MlsGroupError> {
        match self.group_state {
            MlsGroupState::PendingCommit(_) => Err(MlsGroupError::PendingCommitError),
            MlsGroupState::Inactive => {
                Err(MlsGroupError::UseAfterEviction(UseAfterEviction::Error))
            }
            MlsGroupState::Operational => Ok(()),
        }
    }
}

/// `Enum` that indicates whether the inner group state has been modified since the last time it was persisted.
/// `InnerState::Changed` indicates that the state has changed and that [`.save()`] should be called.
/// `InnerState::Persisted` indicates that the state has not been modified and therefore doesn't need to be persisted.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum InnerState {
    Changed,
    Persisted,
}

use crate::group::errors::*;

use crate::messages::proposals::{
    AddProposal, PreSharedKeyProposal, Proposal, ProposalOrRef, ProposalOrRefType,
    ProposalReference, ProposalType, RemoveProposal, UpdateProposal,
};
use crate::treesync::LeafIndex;
use crate::{ciphersuite::*, framing::*};

use openmls_traits::OpenMlsCryptoProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::collections::{hash_map::Entry, HashMap};

/// A [ProposalStore] can store the standalone proposals that are received from the DS
/// in between two commit messages.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProposalStore {
    queued_proposals: Vec<QueuedProposal>,
}

impl ProposalStore {
    pub(crate) fn new() -> Self {
        Self {
            queued_proposals: Vec::new(),
        }
    }
    #[cfg(any(feature = "test-utils", test))]
    pub(crate) fn from_queued_proposal(queued_proposal: QueuedProposal) -> Self {
        Self {
            queued_proposals: vec![queued_proposal],
        }
    }
    pub(crate) fn add(&mut self, queued_proposal: QueuedProposal) {
        self.queued_proposals.push(queued_proposal);
    }
    pub(crate) fn proposals(&self) -> impl Iterator<Item = &QueuedProposal> {
        self.queued_proposals.iter()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.queued_proposals.is_empty()
    }
    pub(crate) fn empty(&mut self) {
        self.queued_proposals = Vec::new();
    }
}

/// Alternative representation of a Proposal, where the sender is extracted from
/// the encapsulating MlsPlaintext and the ProposalReference is attached.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueuedProposal {
    proposal: Proposal,
    proposal_reference: ProposalReference,
    sender: Sender,
    proposal_or_ref_type: ProposalOrRefType,
}

impl QueuedProposal {
    /// Creates a new [QueuedProposal] from an [MlsPlaintext]
    pub(crate) fn from_mls_plaintext(
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        mls_plaintext: MlsPlaintext,
    ) -> Result<Self, QueuedProposalError> {
        let proposal = match mls_plaintext.content() {
            MlsPlaintextContentType::Proposal(p) => p,
            _ => return Err(QueuedProposalError::WrongContentType),
        };
        let proposal_reference = ProposalReference::from_proposal(ciphersuite, backend, proposal)?;
        Ok(Self {
            proposal: proposal.clone(), // FIXME
            proposal_reference,
            sender: *mls_plaintext.sender(),
            proposal_or_ref_type: ProposalOrRefType::Reference,
        })
    }
    /// Creates a new [QueuedProposal] from a [Proposal] and [Sender]
    pub(crate) fn from_proposal_and_sender(
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        proposal: Proposal,
        sender: Sender,
    ) -> Result<Self, QueuedProposalError> {
        let proposal_reference = ProposalReference::from_proposal(ciphersuite, backend, &proposal)?;
        Ok(Self {
            proposal,
            proposal_reference,
            sender,
            proposal_or_ref_type: ProposalOrRefType::Proposal,
        })
    }
    /// Returns the `Proposal` as a reference
    pub fn proposal(&self) -> &Proposal {
        &self.proposal
    }
    /// Returns the `ProposalReference`.
    pub(crate) fn proposal_reference(&self) -> ProposalReference {
        self.proposal_reference.clone()
    }
    /// Returns the `ProposalOrRefType`.
    pub(crate) fn proposal_or_ref_type(&self) -> ProposalOrRefType {
        self.proposal_or_ref_type
    }
    /// Returns the `Sender` as a reference
    pub fn sender(&self) -> &Sender {
        &self.sender
    }
}

/// Proposal queue that helps filtering and sorting Proposals received during one
/// epoch. The Proposals are stored in a `HashMap` which maps Proposal
/// references to Proposals, such that, given a reference, a proposal can be
/// accessed efficiently. To enable iteration over the queue in order, the
/// `ProposalQueue` also contains a vector of `ProposalReference`s.
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ProposalQueue {
    /// `proposal_references` holds references to the proposals in the queue and
    /// determines the order of the queue.
    proposal_references: Vec<ProposalReference>,
    /// `queued_proposals` contains the actual proposals in the queue. They are
    /// stored in a `HashMap` to allow for efficient access to the proposals.
    queued_proposals: HashMap<ProposalReference, QueuedProposal>,
}

impl ProposalQueue {
    /// Returns a new `QueuedProposalQueue` from proposals that were committed and
    /// don't need filtering.
    /// This functions does the following checks:
    ///  - ValSem200
    pub(crate) fn from_committed_proposals(
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        committed_proposals: Vec<ProposalOrRef>,
        proposal_store: &ProposalStore,
        sender: Sender,
    ) -> Result<Self, ProposalQueueError> {
        // Feed the `proposals_by_reference` in a `HashMap` so that we can easily
        // extract then by reference later
        let mut proposals_by_reference_queue: HashMap<ProposalReference, QueuedProposal> =
            HashMap::new();
        for queued_proposal in proposal_store.proposals() {
            proposals_by_reference_queue.insert(
                queued_proposal.proposal_reference(),
                queued_proposal.clone(),
            );
        }
        // Build the actual queue
        let mut proposal_queue = ProposalQueue::default();

        // Iterate over the committed proposals and insert the proposals in the queue
        for proposal_or_ref in committed_proposals.into_iter() {
            let queued_proposal = match proposal_or_ref {
                ProposalOrRef::Proposal(proposal) => {
                    // ValSem200
                    if let Proposal::Remove(ref remove_proposal) = proposal {
                        if remove_proposal.removed() == sender.sender && sender.is_member() {
                            return Err(ProposalQueueError::SelfRemoval);
                        }
                    }

                    QueuedProposal::from_proposal_and_sender(
                        ciphersuite,
                        backend,
                        proposal,
                        sender,
                    )?
                }
                ProposalOrRef::Reference(ref proposal_reference) => {
                    match proposals_by_reference_queue.get(proposal_reference) {
                        Some(queued_proposal) => {
                            // ValSem200
                            if let Proposal::Remove(ref remove_proposal) = queued_proposal.proposal
                            {
                                if remove_proposal.removed() == sender.sender && sender.is_member()
                                {
                                    return Err(ProposalQueueError::SelfRemoval);
                                }
                            }

                            queued_proposal.clone()
                        }
                        None => return Err(ProposalQueueError::ProposalNotFound),
                    }
                }
            };
            proposal_queue.add(queued_proposal);
        }

        Ok(proposal_queue)
    }

    /// Returns proposal for a given proposal ID
    pub(crate) fn get(&self, proposal_reference: &ProposalReference) -> Option<&QueuedProposal> {
        self.queued_proposals.get(proposal_reference)
    }

    /// Add a new [QueuedProposal] to the queue
    pub(crate) fn add(&mut self, queued_proposal: QueuedProposal) {
        let proposal_reference = queued_proposal.proposal_reference();
        // Only add the proposal if it's not already there
        if let Entry::Vacant(entry) = self.queued_proposals.entry(proposal_reference.clone()) {
            // Add the proposal reference to ensure the correct order
            self.proposal_references.push(proposal_reference);
            // Add the proposal to the queue
            entry.insert(queued_proposal);
        }
    }

    /// Returns an iterator over a list of `QueuedProposal` filtered by proposal
    /// type
    pub(crate) fn filtered_by_type(
        &self,
        proposal_type: ProposalType,
    ) -> impl Iterator<Item = &QueuedProposal> {
        // Iterate over the reference to extract the proposals in the right order
        self.proposal_references
            .iter()
            .filter(move |&pr| match self.queued_proposals.get(pr) {
                Some(p) => p.proposal.is_type(proposal_type),
                None => false,
            })
            .filter_map(move |reference| self.get(reference))
    }

    /// Returns an iterator over all `QueuedProposal` in the queue
    /// in the order of the the Commit message
    pub(crate) fn queued_proposals(&self) -> impl Iterator<Item = &QueuedProposal> {
        // Iterate over the reference to extract the proposals in the right order
        self.proposal_references
            .iter()
            .filter_map(move |reference| self.get(reference))
    }

    /// Returns an iterator over all Add proposals in the queue
    /// in the order of the the Commit message
    pub(crate) fn add_proposals(&self) -> impl Iterator<Item = QueuedAddProposal> {
        self.queued_proposals().filter_map(|queued_proposal| {
            if let Proposal::Add(add_proposal) = queued_proposal.proposal() {
                let sender = queued_proposal.sender();
                Some(QueuedAddProposal {
                    add_proposal,
                    sender,
                })
            } else {
                None
            }
        })
    }

    /// Returns an iterator over all Remove proposals in the queue
    /// in the order of the the Commit message
    pub(crate) fn remove_proposals(&self) -> impl Iterator<Item = QueuedRemoveProposal> {
        self.queued_proposals().filter_map(|queued_proposal| {
            if let Proposal::Remove(remove_proposal) = queued_proposal.proposal() {
                let sender = queued_proposal.sender();
                Some(QueuedRemoveProposal {
                    remove_proposal,
                    sender,
                })
            } else {
                None
            }
        })
    }

    /// Returns an iterator over all Update in the queue
    /// in the order of the the Commit message
    pub(crate) fn update_proposals(&self) -> impl Iterator<Item = QueuedUpdateProposal> {
        self.queued_proposals().filter_map(|queued_proposal| {
            if let Proposal::Update(update_proposal) = queued_proposal.proposal() {
                let sender = queued_proposal.sender();
                Some(QueuedUpdateProposal {
                    update_proposal,
                    sender,
                })
            } else {
                None
            }
        })
    }

    /// Returns an iterator over all PresharedKey proposals in the queue
    /// in the order of the the Commit message
    pub(crate) fn psk_proposals(&self) -> impl Iterator<Item = QueuedPskProposal> {
        self.queued_proposals().filter_map(|queued_proposal| {
            if let Proposal::PreSharedKey(psk_proposal) = queued_proposal.proposal() {
                let sender = queued_proposal.sender();
                Some(QueuedPskProposal {
                    psk_proposal,
                    sender,
                })
            } else {
                None
            }
        })
    }

    /// Filters received proposals
    ///
    /// 11.2 Commit
    /// If there are multiple proposals that apply to the same leaf,
    /// the committer chooses one and includes only that one in the Commit,
    /// considering the rest invalid. The committer MUST prefer any Remove
    /// received, or the most recent Update for the leaf if there are no
    /// Removes. If there are multiple Add proposals for the same client,
    /// the committer again chooses one to include and considers the rest
    /// invalid.
    ///
    /// The function performs the following steps:
    ///
    /// - Extract Adds and filter for duplicates
    /// - Build member list with chains: Updates & Removes
    /// - Check for invalid indexes and drop proposal
    /// - Check for presence of Removes and delete Updates
    /// - Only keep the last Update
    ///
    /// Return a [`ProposalQueue`] and a bool that indicates whether Updates for the
    /// own node were included
    pub(crate) fn filter_proposals(
        ciphersuite: &Ciphersuite,
        backend: &impl OpenMlsCryptoProvider,
        sender_type: SenderType,
        proposal_store: &ProposalStore,
        inline_proposals: &[Proposal],
        own_index: LeafIndex,
        leaf_count: LeafIndex,
    ) -> Result<(Self, bool), ProposalQueueError> {
        fn to_usize(leaf_index: LeafIndex) -> Result<usize, ProposalQueueError> {
            usize::try_from(leaf_index).map_err(|_| ProposalQueueError::ArchitectureError)
        }

        #[derive(Clone)]
        struct Member {
            updates: Vec<QueuedProposal>,
            removes: Vec<QueuedProposal>,
        }
        let mut members: Vec<Member> = vec![
            Member {
                updates: vec![],
                removes: vec![],
            };
            to_usize(leaf_count)?
        ];
        let mut adds: HashSet<ProposalReference> = HashSet::new();
        let mut valid_proposals: HashSet<ProposalReference> = HashSet::new();
        let mut proposal_pool: HashMap<ProposalReference, QueuedProposal> = HashMap::new();
        let mut contains_own_updates = false;
        let mut contains_external_init = false;

        let sender = Sender {
            sender_type,
            sender: own_index,
        };

        // Aggregate both proposal types to a common iterator
        // We checked earlier that only proposals can end up here
        let mut queued_proposal_list: Vec<QueuedProposal> =
            proposal_store.proposals().cloned().collect();

        queued_proposal_list.extend(
            inline_proposals
                .iter()
                .map(|p| {
                    QueuedProposal::from_proposal_and_sender(
                        ciphersuite,
                        backend,
                        p.clone(),
                        sender,
                    )
                })
                .collect::<Result<Vec<QueuedProposal>, _>>()?
                .into_iter(),
        );

        // Parse proposals and build adds and member list
        for queued_proposal in queued_proposal_list {
            match queued_proposal.proposal {
                Proposal::Add(_) => {
                    adds.insert(queued_proposal.proposal_reference());
                    proposal_pool.insert(queued_proposal.proposal_reference(), queued_proposal);
                }
                Proposal::Update(_) => {
                    let sender_index = queued_proposal.sender.sender;
                    if sender_index != own_index {
                        members[to_usize(sender_index)?]
                            .updates
                            .push(queued_proposal.clone());
                    } else {
                        contains_own_updates = true;
                    }
                    let proposal_reference = queued_proposal.proposal_reference();
                    proposal_pool.insert(proposal_reference, queued_proposal);
                }
                Proposal::Remove(ref remove_proposal) => {
                    let removed_index = remove_proposal.removed;
                    if removed_index < leaf_count {
                        members[to_usize(removed_index)?]
                            .updates
                            .push(queued_proposal.clone());
                    }
                    let proposal_reference = queued_proposal.proposal_reference();
                    proposal_pool.insert(proposal_reference, queued_proposal);
                }
                Proposal::PreSharedKey(_) => {
                    valid_proposals.insert(queued_proposal.proposal_reference());
                    proposal_pool.insert(queued_proposal.proposal_reference(), queued_proposal);
                }
                Proposal::ReInit(_) => {
                    // TODO #141: Only keep one ReInit
                    proposal_pool.insert(queued_proposal.proposal_reference(), queued_proposal);
                }
                Proposal::ExternalInit(_) => {
                    // Only use the first external init proposal we find.
                    if !contains_external_init {
                        valid_proposals.insert(queued_proposal.proposal_reference());
                        proposal_pool.insert(queued_proposal.proposal_reference(), queued_proposal);
                        contains_external_init = true;
                    }
                }
                Proposal::AppAck(_) => unimplemented!("See #291"),
                Proposal::GroupContextExtensions(_) => {
                    // TODO: Validate proposal?
                    proposal_pool.insert(queued_proposal.proposal_reference(), queued_proposal);
                }
            }
        }
        // Check for presence of Removes and delete Updates
        for member in members.iter_mut() {
            // Check if there are Removes
            if let Some(last_remove) = member.removes.last() {
                // Delete all Updates when a Remove is found
                member.updates = Vec::new();
                // Only keep the last Remove
                valid_proposals.insert(last_remove.proposal_reference());
            }
            if let Some(last_update) = member.updates.last() {
                // Only keep the last Update
                valid_proposals.insert(last_update.proposal_reference());
            }
        }
        // Only retain `adds` and `valid_proposals`
        let mut proposal_queue = ProposalQueue::default();
        for proposal_reference in adds.iter().chain(valid_proposals.iter()) {
            proposal_queue.add(match proposal_pool.get(proposal_reference) {
                Some(queued_proposal) => queued_proposal.clone(),
                None => return Err(ProposalQueueError::ProposalNotFound),
            });
        }
        Ok((proposal_queue, contains_own_updates))
    }
    /// Returns `true` if all `ProposalReference` values from the list are
    /// contained in the queue
    #[cfg(test)]
    pub(crate) fn contains(&self, proposal_reference_list: &[ProposalReference]) -> bool {
        for proposal_reference in proposal_reference_list {
            if !self.queued_proposals.contains_key(proposal_reference) {
                return false;
            }
        }
        true
    }

    /// Returns the list of all proposals that are covered by a Commit
    pub(crate) fn commit_list(&self) -> Vec<ProposalOrRef> {
        // Iterate over the reference to extract the proposals in the right order
        self.proposal_references
            .iter()
            .filter_map(|proposal_reference| self.queued_proposals.get(proposal_reference))
            .map(|queued_proposal| {
                // Differentiate the type of proposal
                match queued_proposal.proposal_or_ref_type {
                    ProposalOrRefType::Proposal => {
                        ProposalOrRef::Proposal(queued_proposal.proposal.clone())
                    }
                    ProposalOrRefType::Reference => {
                        ProposalOrRef::Reference(queued_proposal.proposal_reference.clone())
                    }
                }
            })
            .collect::<Vec<ProposalOrRef>>()
    }
}

/// A queued Add proposal
pub struct QueuedAddProposal<'a> {
    add_proposal: &'a AddProposal,
    sender: &'a Sender,
}

impl<'a> QueuedAddProposal<'a> {
    /// Returns a reference to the proposal
    pub fn add_proposal(&self) -> &AddProposal {
        self.add_proposal
    }

    /// Returns a reference to the sender
    pub fn sender(&self) -> &Sender {
        self.sender
    }
}

/// A queued Remove proposal
pub struct QueuedRemoveProposal<'a> {
    remove_proposal: &'a RemoveProposal,
    sender: &'a Sender,
}

impl<'a> QueuedRemoveProposal<'a> {
    /// Returns a reference to the proposal
    pub fn remove_proposal(&self) -> &RemoveProposal {
        self.remove_proposal
    }

    /// Returns a reference to the sender
    pub fn sender(&self) -> &Sender {
        self.sender
    }
}

/// A queued Update proposal
pub struct QueuedUpdateProposal<'a> {
    update_proposal: &'a UpdateProposal,
    sender: &'a Sender,
}

impl<'a> QueuedUpdateProposal<'a> {
    /// Returns a reference to the proposal
    pub fn update_proposal(&self) -> &UpdateProposal {
        self.update_proposal
    }

    /// Returns a reference to the sender
    pub fn sender(&self) -> &Sender {
        self.sender
    }
}

/// A queued PresharedKey proposal
pub struct QueuedPskProposal<'a> {
    psk_proposal: &'a PreSharedKeyProposal,
    sender: &'a Sender,
}

impl<'a> QueuedPskProposal<'a> {
    /// Returns a reference to the proposal
    pub fn psk_proposal(&self) -> &PreSharedKeyProposal {
        self.psk_proposal
    }

    /// Returns a reference to the sender
    pub fn sender(&self) -> &Sender {
        self.sender
    }
}

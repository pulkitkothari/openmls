//! This module contains an implementation of a binary tree based on an array
//! representation.

use std::convert::TryFrom;
use std::fmt::Debug;

use serde::{Deserialize, Serialize};

use super::{
    diff::{AbDiff, StagedAbDiff},
    treemath::TreeMathError,
};
use crate::binary_tree::LeafIndex;

/// The `NodeIndex` is used throughout this trait to index nodes as if the
/// underlying binary tree was implementing the array representation.
pub(crate) type NodeIndex = u32;

pub(super) fn to_node_index(leaf_index: LeafIndex) -> NodeIndex {
    leaf_index * 2
}

pub(crate) type TreeSize = NodeIndex;

#[derive(Clone, Debug, Serialize, Deserialize)]
/// A representation of a full, left-balanced binary tree that uses a simple
/// vector to store nodes.
pub(crate) struct ABinaryTree<T: Clone + Debug> {
    nodes: Vec<T>,
}

impl<T: Clone + Debug> TryFrom<Vec<T>> for ABinaryTree<T> {
    type Error = ABinaryTreeError;

    fn try_from(nodes: Vec<T>) -> Result<Self, Self::Error> {
        Self::new(nodes)
    }
}

impl<T: Clone + Debug> ABinaryTree<T> {
    /// Create a tree from the given vector of nodes. The vector of nodes can't
    /// be empty and has to yield a full, left-balanced binary tree. The nodes
    /// in the tree are ordered in the array-representation. This function
    /// throws a `InvalidNumberOfNodes` error if the number of nodes does not
    /// allow the creation of a full, left-balanced binary tree and an
    /// `OutOfRange` error if the number of given nodes exceeds the range of
    /// `NodeIndex`.
    pub(crate) fn new(nodes: Vec<T>) -> Result<Self, ABinaryTreeError> {
        if nodes.len() > NodeIndex::max_value() as usize {
            return Err(ABinaryTreeError::OutOfRange);
        } else if nodes.len() % 2 != 1 {
            return Err(ABinaryTreeError::InvalidNumberOfNodes);
        }
        Ok(ABinaryTree { nodes })
    }

    /// Obtain a reference to the data contained in the `Node` at index
    /// `node_index`, where the indexing corresponds to the array representation
    /// of the underlying binary tree. Returns None if the index is outside of
    /// the tree.
    pub(in crate::binary_tree) fn node_by_index(
        &self,
        node_index: NodeIndex,
    ) -> std::option::Option<&T> {
        self.nodes.get(node_index as usize)
    }

    /// Return the number of nodes in the tree.
    pub(in crate::binary_tree) fn size(&self) -> NodeIndex {
        let len = self.nodes.len();
        debug_assert!(len <= u32::MAX as usize);
        self.nodes.len() as u32
    }

    /// Return the number of leaves in the tree.
    pub(crate) fn leaf_count(&self) -> TreeSize {
        (self.size() + 1) / 2
    }

    /// Vector of leaves sorted from left to right in the tree.
    pub(crate) fn leaves(&self) -> Result<Vec<&T>, ABinaryTreeError> {
        let mut leaf_references = Vec::new();
        for leaf_index in 0..self.leaf_count() {
            let node_index = usize::try_from(to_node_index(leaf_index))
                // The tree size and thus the leaf count should fit into usize on 32
                // bit architectures.
                .map_err(|_| ABinaryTreeError::LibraryError)?;
            let node_ref = self
                .nodes
                .get(node_index)
                // Since the index is within the bounds of the tree, this should
                // be Some.
                .ok_or(ABinaryTreeError::LibraryError)?;
            leaf_references.push(node_ref);
        }
        Ok(leaf_references)
    }

    pub(crate) fn empty_diff(&self) -> AbDiff<'_, T> {
        self.into()
    }

    pub(crate) fn merge_diff(&mut self, diff: StagedAbDiff<T>) -> Result<(), ABinaryTreeError> {
        // The diff size should fit into a 32 bit usize.
        let diff_size = usize::try_from(diff.size()).map_err(|_| ABinaryTreeError::LibraryError)?;
        // If the size of the diff is smaller than the tree, truncate the tree
        // to the size of the diff.
        self.nodes.truncate(diff_size);

        // Iterate over the BTreeMap in order of indices.
        for (node_index, diff_node) in diff.diff().into_iter() {
            match node_index {
                // If the node would extend the tree, push it to the vector of nodes.
                node_index if node_index == self.size() => self.nodes.push(diff_node),
                // If the node index points too far outside of the tree,
                // something has gone wrong.
                node_index if node_index > self.size() => {
                    return Err(ABinaryTreeError::LibraryError)
                }
                // If the node_index points to somewhere within the size of the
                // tree, do a swap-remove.
                node_index => {
                    // Perform swap-remove.
                    self.nodes.push(diff_node);
                    self.nodes.swap_remove(
                        // If the node_index is larger that u32 max, something
                        // has gone wrong.
                        usize::try_from(node_index).map_err(|_| ABinaryTreeError::LibraryError)?,
                    );
                }
            }
        }
        Ok(())
    }

    /// Export the nodes of the tree in the array representation.
    pub(crate) fn export_nodes(&self) -> Vec<T> {
        self.nodes.clone()
    }
}

implement_error! {
    pub enum ABinaryTreeError {
        Simple {
            OutOfRange = "Adding nodes exceeds the maximum possible size of the tree.",
            NotEnoughNodes = "Not enough nodes to remove.",
            InvalidNumberOfNodes = "The given number of nodes does not allow the creation of a full, left-balanced binary tree.",
            OutOfBounds = "The given index is outside of the tree.",
            AddressCollision = "Found two nodes with the same address.",
            InvalidNode = "Can't add the default node to the tree.",
            NodeNotFound = "Can't find the node with the given address in the tree.",
            LibraryError = "An inconsistency in the internal state of the tree was detected.",
        }
        Complex {
            TreeMathError(TreeMathError) = "Error while traversing the tree.",
        }
    }
}

#[cfg(test)]
impl PartialEq for ABinaryTree<u32> {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes
    }
}

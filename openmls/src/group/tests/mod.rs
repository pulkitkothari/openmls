//! Unit tests for the core group

#[cfg(feature = "test-utils")]
pub mod kat_messages;

#[cfg(feature = "test-utils")]
pub mod kat_transcripts;

#[cfg(test)]
mod test_encoding;
#[cfg(test)]
mod test_external_commit_validation;
#[cfg(test)]
mod test_framing;
#[cfg(test)]
mod test_group;
#[cfg(test)]
mod test_past_secrets;
#[cfg(test)]
mod test_validation;
#[cfg(test)]
mod test_wire_format_policy;
#[cfg(test)]
pub(crate) mod utils;

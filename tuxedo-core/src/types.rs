//! The common types that will be used across a Tuxedo runtime, and not specific to any one piece

use crate::{dynamic_typing::DynamicallyTypedData, ConstraintChecker, Verifier};
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use sp_runtime::traits::{BlakeTwo256, Extrinsic, Hash as HashT};
use sp_std::vec::Vec;

// All Tuxedo chains use the same BlakeTwo256 hash.
pub type Hash = BlakeTwo256;
/// Opaque block hash type.
pub type OpaqueHash = <Hash as HashT>::Output;
/// All Tuxedo chains use the same u32 BlockNumber.
pub type BlockNumber = u32;
/// Because all tuxedo chains use the same Blocknumber and Hash types,
/// they also use the same concrete header type.
pub type Header = sp_runtime::generic::Header<BlockNumber, Hash>;
/// An alias for a Tuxedo block with all the common parts filled in.
pub type Block<V, C> = sp_runtime::generic::Block<Header, Transaction<V, C>>;
/// Opaque block type. It has a Standard Tuxedo header, and opaque transactions.
pub type OpaqueBlock = sp_runtime::generic::Block<Header, sp_runtime::OpaqueExtrinsic>;

/// A reference to a output that is expected to exist in the state.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, TypeInfo)]
pub struct OutputRef {
    /// A hash of the transaction that created this output
    pub tx_hash: H256,
    /// The index of this output among all outputs created by the same transaction
    pub index: u32,
}

/// A UTXO Transaction
///
/// Each transaction consumes some UTXOs (the inputs) and creates some new ones (the outputs).
///
/// The Transaction type is generic over two orthogonal pieces of validation logic:
/// 1. Verifier - A verifier checks that an individual input may be consumed. A typical example
///    of a verifier is checking that there is a signature by the proper owner. Other examples
///    may be that anyone can consume the input or no one can, or that a proof of work is required.
/// 2. ConstraintCheckers - A constraint checker checks that the transaction as a whole meets a set of requirements.
///    For example, that the total output value of a cryptocurrency transaction does not exceed its
///    input value. Or that a cryptokitty was created with the correct genetic material from its parents.
///
/// In the future, there may be additional notions of peeks (inputs that are not consumed)
/// and evictions (inputs that are forcefully consumed.)
/// Existing state to be read and consumed from storage
#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq, Clone, TypeInfo)]
pub struct Transaction<V, C> {
    /// Existing pieces of state to be read and consumed from storage
    pub inputs: Vec<Input>,
    /// Existing state to be read, but not consumed, from storage
    pub peeks: Vec<OutputRef>,
    /// New state to be placed into storage
    pub outputs: Vec<Output<V>>,
    /// Which piece of constraint checking logic is used to determine whether this transaction is valid
    pub checker: C,
}

impl<V: Clone, C: Clone> Transaction<V, C> {
    /// A helper function for transforming a transaction generic over one
    /// kind of constraint checker into a transaction generic over another type
    /// of constraint checker. This is useful when moving up and down the aggregation tree.
    pub fn transform<D: From<C>>(&self) -> Transaction<V, D> {
        Transaction {
            inputs: self.inputs.clone(),
            peeks: self.peeks.clone(),
            outputs: self.outputs.clone(),
            checker: self.checker.clone().into(),
        }
    }
}

// Manually implement Encode and Decode for the Transaction type
// so that its encoding is the same as an opaque Vec<u8>.
impl<V: Encode, C: Encode> Encode for Transaction<V, C> {
    fn encode_to<T: parity_scale_codec::Output + ?Sized>(&self, dest: &mut T) {
        let inputs = self.inputs.encode();
        let peeks = self.peeks.encode();
        let outputs = self.outputs.encode();
        let checker = self.checker.encode();

        let total_len = (inputs.len() + outputs.len() + peeks.len() + checker.len()) as u32;
        let size = parity_scale_codec::Compact::<u32>(total_len).encode();

        dest.write(&size);
        dest.write(&inputs);
        dest.write(&peeks);
        dest.write(&outputs);
        dest.write(&checker);
    }
}

impl<V: Decode, C: Decode> Decode for Transaction<V, C> {
    fn decode<I: parity_scale_codec::Input>(
        input: &mut I,
    ) -> Result<Self, parity_scale_codec::Error> {
        // Throw away the length of the vec. We just want the bytes.
        <parity_scale_codec::Compact<u32>>::skip(input)?;

        let inputs = <Vec<Input>>::decode(input)?;
        let peeks = <Vec<OutputRef>>::decode(input)?;
        let outputs = <Vec<Output<V>>>::decode(input)?;
        let checker = C::decode(input)?;

        Ok(Transaction {
            inputs,
            peeks,
            outputs,
            checker,
        })
    }
}

// We must implement this Extrinsic trait to use our Transaction type as the Block's associated Extrinsic type.
// See https://paritytech.github.io/polkadot-sdk/master/sp_runtime/traits/trait.Block.html#associatedtype.Extrinsic
//
// This trait's design has a preference for transactions that will have a single signature over the
// entire transaction, so it is not very useful for us. We still need to implement it to satisfy the bound,
// so we do a minimal implementation.
impl<V, C> Extrinsic for Transaction<V, C>
where
    C: TypeInfo + ConstraintChecker + 'static,
    V: TypeInfo + Verifier + 'static,
{
    type Call = Self;
    type SignaturePayload = ();

    fn new(data: Self, _: Option<Self::SignaturePayload>) -> Option<Self> {
        Some(data)
    }

    // The signature on this function is not the best. Ultimately it is
    // trying to distinguish between three potential types of transactions:
    // 1. Signed user transactions: `Some(true)`
    // 2. Unsigned user transactions: `None`
    // 3. Unsigned inherents: `Some(false)`
    //
    // In Substrate generally, and also in FRAME, all three of these could exist.
    // But in Tuxedo we will never have signed user transactions, and therefore
    // will never return `Some(true)`.
    //
    // Perhaps a dedicated enum makes more sense as the return type?
    // That would be a Substrate PR after this is more tried and true.
    fn is_signed(&self) -> Option<bool> {
        if self.checker.is_inherent() {
            Some(false)
        } else {
            None
        }
    }
}

/// A reference the a utxo that will be consumed along with proof that it may be consumed
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, TypeInfo)]
pub struct Input {
    /// a reference to the output being consumed
    pub output_ref: OutputRef,
    /// A means of showing that this input data can be used.
    /// It is most often a proof such as a signature, but could also be a forceful eviction.
    pub redeemer: RedemptionStrategy,
}

//TODO Consider making this enum generic over the redeemer type so there is not an additional decoding necessary.
// This would percolate up though. For example input would also have to be generic over the redeemer type.
// IDK if it is appropriate to be making so many types non-opaque??
/// An input can be consumed in two way. It can be redeemed normally (probably with some signature, or proof) or it
/// can be evicted. This enum is isomorphic to `Option<Vec<u8>>` but has more meaningful names.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, TypeInfo)]
pub enum RedemptionStrategy {
    /// The input is being consumed in the normal way with a signature or other proof provided by the spender.
    Redemption(Vec<u8>),
    /// The input is being forcefully evicted without satisfying its Verifier.
    Eviction,
}

impl Default for RedemptionStrategy {
    fn default() -> Self {
        Self::Redemption(Vec::new())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UtxoError<ConstraintCheckerError> {
    /// This transaction defines the same input multiple times
    DuplicateInput,
    /// This transaction defines an output that already existed in the UTXO set
    PreExistingOutput,
    /// The constraint checker errored.
    ConstraintCheckerError(ConstraintCheckerError),
    /// The Verifier errored.
    /// TODO determine whether it is useful to relay an inner error from the verifier.
    /// So far, I haven't seen a case, although it seems reasonable to think there might be one.
    VerifierError,
    /// One or more of the inputs required by this transaction is not present in the UTXO set
    MissingInput,
}

/// The Result of dispatching a UTXO transaction.
pub type DispatchResult<ConstraintCheckerError> = Result<(), UtxoError<ConstraintCheckerError>>;

/// An opaque piece of Transaction output data. This is how the data appears at the Runtime level. After
/// the verifier is checked, strongly typed data will be extracted and passed to the constraint checker.
/// In a cryptocurrency, the data represents a single coin. In Tuxedo, the type of
/// the contained data is generic.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, TypeInfo)]
pub struct Output<V> {
    pub payload: DynamicallyTypedData,
    pub verifier: V,
}

impl<V: Default> From<DynamicallyTypedData> for Output<V> {
    fn from(payload: DynamicallyTypedData) -> Self {
        Self {
            payload,
            verifier: Default::default(),
        }
    }
}

impl<V, V1: Into<V>, P: Into<DynamicallyTypedData>> From<(P, V1)> for Output<V> {
    fn from(values: (P, V1)) -> Self {
        Self {
            payload: values.0.into(),
            verifier: values.1.into(),
        }
    }
}

#[cfg(test)]
pub mod tests {

    use crate::{constraint_checker::testing::TestConstraintChecker, verifier::TestVerifier};

    use super::*;

    #[test]
    fn extrinsic_no_signed_payload() {
        let checker = TestConstraintChecker {
            checks: true,
            inherent: false,
        };
        let tx: Transaction<TestVerifier, TestConstraintChecker> = Transaction {
            inputs: Vec::new(),
            peeks: Vec::new(),
            outputs: Vec::new(),
            checker,
        };
        let e = Transaction::new(tx.clone(), None).unwrap();

        assert_eq!(e, tx);
        assert_eq!(e.is_signed(), None);
    }

    #[test]
    fn extrinsic_is_signed_works() {
        let checker = TestConstraintChecker {
            checks: true,
            inherent: false,
        };
        let tx: Transaction<TestVerifier, TestConstraintChecker> = Transaction {
            inputs: Vec::new(),
            peeks: Vec::new(),
            outputs: Vec::new(),
            checker,
        };
        let e = Transaction::new(tx.clone(), Some(())).unwrap();

        assert_eq!(e, tx);
        assert_eq!(e.is_signed(), None);
    }

    #[test]
    fn extrinsic_is_signed_works_for_inherents() {
        let checker = TestConstraintChecker {
            checks: true,
            inherent: true,
        };
        let tx: Transaction<TestVerifier, TestConstraintChecker> = Transaction {
            inputs: Vec::new(),
            peeks: Vec::new(),
            outputs: Vec::new(),
            checker,
        };
        let e = Transaction::new(tx.clone(), Some(())).unwrap();

        assert_eq!(e, tx);
        assert_eq!(e.is_signed(), Some(false));
    }
}

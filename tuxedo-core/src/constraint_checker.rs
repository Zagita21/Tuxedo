//! A constraint checker is a piece of logic that determines whether a transaction as a whole is valid
//! and should be committed. Most tuxedo pieces will provide one or more constraint checkers.
//! Constraint Checkers do not calculate the correct final state, but rather determine whether the
//! proposed final state (as specified by the output set) meets the necessary constraints.
//!
//! Constraint Checkers can be used to codify the laws of a monetary system, a chemistry or physics simulation,
//! NFT kitties, public elections and much more.
//!
//! The primary way for developers to write a constraint checker is with the `SimpleConstraintChecker`
//! trait. It provides a method called `check` which determines whether the relationship between the inputs
//! and outputs (and peeks) is valid. For example making sure no extra money was created, or making sure the chemical
//! reaction balances.
//!
//! ## Inherents
//!
//! If you need to tap in to [Substrate's inherent system](https://docs.substrate.io/learn/transaction-types/#inherent-transactions)
//! you may choose to also implement the `InherentHooks` trait for the same type that implements `SimpleConstraintChecker`.
//! When installing a constraint checker in your runtime, make sure to wrap it with the `InherentAdapter` type.
//! See the `inherents` module for more details.
//!
//! ## Constraint Checker Internals
//!
//! One of Tuxedo's killer features is its ability to aggregating pieces recursively.
//! To achieve this we have to consider that many intermediate layers in the aggregation tree
//! will have multiple inherent types. For this reason, we provide a much more flexible interface
//! that the aggregation macro can use called `ConstraintChecker`. Do not implement `ConstraintChecker`
//! directly.

use sp_core::H256;
use sp_inherents::{CheckInherentsResult, InherentData};
use sp_std::{fmt::Debug, vec::Vec};

use crate::{dynamic_typing::DynamicallyTypedData, types::Transaction, Verifier};
use parity_scale_codec::{Decode, Encode};
use sp_runtime::transaction_validity::TransactionPriority;

/// A particular constraint checker that a transaction can choose to be checked by.
/// Checks whether the input and output data from a transaction meets the codified constraints.
///
/// Additional transient information may be passed to the constraint checker by including it in the fields
/// of the constraint checker struct itself. Information passed in this way does not come from state, nor
/// is it stored in state.
pub trait SimpleConstraintChecker: Debug + Encode + Decode + Clone {
    /// The error type that this constraint checker may return
    type Error: Debug;

    /// The on chain logic that makes the final check for whether a transaction is valid.
    fn check(
        &self,
        input_data: &[DynamicallyTypedData],
        evicted_input_data: &[DynamicallyTypedData],
        peek_data: &[DynamicallyTypedData],
        output_data: &[DynamicallyTypedData],
    ) -> Result<TransactionPriority, Self::Error>;
}

/// The raw and fully powerful `ConstraintChecker` interface used by the
/// Tuxedo Executive.
///
/// You should never manually manually implement this trait.
/// If you are:
/// * Working on a simple non-inherent constraint checker -> Use the `SimpleConstraintChecker` trait instead
///   and rely on its blanket implementation.
/// * Working on an inherent constraint checker -> Implement `SimpleConstraintChecker` and `InherentHooks` and use the
/// `InherentAdapter` wrapper type.
/// * Considering an aggregate constraint checker that is part inherent, part not -> let the macro handle it for you.
///
/// If you are trying to implement some complex inherent logic that requires the interaction of
/// multiple inherents, or features a variable number of inherents in each block, you might be
/// able to express it by implementing this trait, but such designs are probably too complicated.
/// Think long and hard before implementing this trait directly.
pub trait ConstraintChecker: Debug + Encode + Decode + Clone {
    /// The error type that this constraint checker may return
    type Error: Debug;

    /// The on chain logic that makes the final check for whether a transaction is valid.
    fn check(
        &self,
        input_data: &[DynamicallyTypedData],
        evicted_input_data: &[DynamicallyTypedData],
        peek_data: &[DynamicallyTypedData],
        output_data: &[DynamicallyTypedData],
    ) -> Result<TransactionPriority, Self::Error>;

    /// Tells whether this extrinsic is an inherent or not.
    /// If you return true here, you must provide the correct inherent hooks above.
    fn is_inherent(&self) -> bool;

    /// Create the inherent extrinsics to insert into a block that is being authored locally.
    /// The inherent data is supplied by the authoring node.
    fn create_inherents<V: Verifier>(
        authoring_inherent_data: &InherentData,
        previous_inherents: Vec<(Transaction<V, Self>, H256)>,
    ) -> Vec<Transaction<V, Self>>;

    /// Perform off-chain pre-execution checks on the inherents.
    /// The inherent data is supplied by the importing node.
    /// The inherent data available here is not necessarily the
    /// same as what is available at authoring time.
    fn check_inherents<V: Verifier>(
        importing_inherent_data: &InherentData,
        inherents: Vec<Transaction<V, Self>>,
        results: &mut CheckInherentsResult,
    );

    /// Return the genesis transactions that are required for the inherents.
    fn genesis_transactions<V: Verifier>() -> Vec<Transaction<V, Self>>;
}

// We automatically supply every single simple constraint checker with a dummy set
// of inherent hooks. This allows "normal" non-inherent constraint checkers to satisfy the
// executive's expected interfaces without the piece author worrying about inherents.
impl<T: SimpleConstraintChecker> ConstraintChecker for T {
    // Use the same error type used in the simple implementation.
    type Error = <T as SimpleConstraintChecker>::Error;

    fn check(
        &self,
        input_data: &[DynamicallyTypedData],
        evicted_input_data: &[DynamicallyTypedData],
        peek_data: &[DynamicallyTypedData],
        output_data: &[DynamicallyTypedData],
    ) -> Result<TransactionPriority, Self::Error> {
        SimpleConstraintChecker::check(self, input_data, evicted_input_data, peek_data, output_data)
    }

    fn is_inherent(&self) -> bool {
        false
    }

    fn create_inherents<V>(
        _authoring_inherent_data: &InherentData,
        _previous_inherents: Vec<(Transaction<V, Self>, H256)>,
    ) -> Vec<Transaction<V, Self>> {
        Vec::new()
    }

    fn check_inherents<V>(
        _: &InherentData,
        inherents: Vec<Transaction<V, Self>>,
        _: &mut CheckInherentsResult,
    ) {
        // Inherents should always be empty for this stub implementation. Not just in valid blocks, but as an invariant.
        // The way we determined which inherents got here is by matching on the constraint checker.
        assert!(
            inherents.is_empty(),
            "inherent extrinsic was passed to check inherents stub implementation."
        )
    }

    fn genesis_transactions<V>() -> Vec<Transaction<V, Self>> {
        Vec::new()
    }
}

/// Utilities for writing constraint-checker-related unit tests
#[cfg(test)]
pub mod testing {
    use parity_scale_codec::{Decode, Encode};
    use scale_info::TypeInfo;
    use serde::{Deserialize, Serialize};

    use super::{ConstraintChecker, DynamicallyTypedData, TransactionPriority};

    /// A testing checker that passes (with zero priority) or not depending on
    /// the boolean value enclosed.
    #[derive(Serialize, Deserialize, Encode, Decode, Debug, Clone, PartialEq, Eq, TypeInfo)]
    pub struct TestConstraintChecker {
        /// Whether the checker should pass.
        pub checks: bool,
        /// Whether this constraint checker is an inherent.
        pub inherent: bool,
    }

    impl ConstraintChecker for TestConstraintChecker {
        type Error = ();

        fn check(
            &self,
            _input_data: &[DynamicallyTypedData],
            _evicted_input_data: &[DynamicallyTypedData],
            _peek_data: &[DynamicallyTypedData],
            _output_data: &[DynamicallyTypedData],
        ) -> Result<TransactionPriority, ()> {
            if self.checks {
                Ok(0)
            } else {
                Err(())
            }
        }

        fn is_inherent(&self) -> bool {
            self.inherent
        }

        fn create_inherents<V: crate::Verifier>(
            _: &sp_inherents::InherentData,
            _: Vec<(crate::types::Transaction<V, Self>, sp_core::H256)>,
        ) -> Vec<crate::types::Transaction<V, Self>> {
            unimplemented!()
        }

        fn check_inherents<V: crate::Verifier>(
            _: &sp_inherents::InherentData,
            _: Vec<crate::types::Transaction<V, Self>>,
            _: &mut sp_inherents::CheckInherentsResult,
        ) {
            unimplemented!()
        }

        fn genesis_transactions<V: crate::Verifier>() -> Vec<crate::types::Transaction<V, Self>> {
            unimplemented!()
        }
    }

    #[test]
    fn test_checker_passes() {
        let result = TestConstraintChecker {
            checks: true,
            inherent: false,
        }
        .check(&[], &[], &[], &[]);
        assert_eq!(result, Ok(0));
    }

    #[test]
    fn test_checker_fails() {
        let result = TestConstraintChecker {
            checks: false,
            inherent: false,
        }
        .check(&[], &[], &[], &[]);
        assert_eq!(result, Err(()));
    }
}

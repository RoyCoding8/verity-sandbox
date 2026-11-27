use super::{ScopeError, ScopeViolationKind};
use crate::types::RelPath;
use core::marker::PointeeSized;
use vstd::prelude::*;
use vstd::utf8::encode_utf8;

verus! {
    // AsRef permits stateful implementations. Only a type that opts into this
    // contract promises that calls return a stable view without unwinding.
    #[verifier::external_trait_specification]
    #[verifier::external_trait_extension(ScopeAsRef via ScopeAsRefImpl)]
    pub trait ExternalAsRef<T: PointeeSized>: PointeeSized {
        type ExternalTraitSpecificationFor: AsRef<T>;

        spec fn has_scope_ref() -> bool;
        spec fn scope_chars(&self) -> Seq<char>;
        spec fn ref_chars(value: &T) -> Seq<char>;

        fn as_ref(&self) -> (result: &T)
            ensures Self::has_scope_ref() ==> Self::ref_chars(result) == self.scope_chars(),
            no_unwind when Self::has_scope_ref();
    }

    pub open spec fn stable_path_view<P: AsRef<str>>() -> bool {
        &&& P::has_scope_ref()
        // Tie the generic AsRef target's projection to the actual str view.
        &&& forall|value: &str| #[trigger] P::ref_chars(value) == value@
    }

    pub open spec fn path_bytes<P: AsRef<str>>(path: &P) -> Seq<u8> {
        encode_utf8(path.scope_chars())
    }

    pub open spec fn within_prefix(path: Seq<u8>, prefix: Seq<u8>) -> bool {
        &&& 0 < prefix.len() <= path.len()
        &&& prefix =~= path.take(prefix.len() as int)
        &&& (path.len() == prefix.len() || path[prefix.len() as int] == b'/')
    }

    pub open spec fn matches_prefix<P: AsRef<str>>(path: Seq<u8>, prefixes: Seq<P>) -> bool {
        exists|i: int| 0 <= i < prefixes.len() &&
            within_prefix(path, #[trigger] path_bytes(&prefixes[i]))
    }

    pub open spec fn approved_path<P: AsRef<str>>(
        path: Seq<u8>, write_scope: Seq<P>, forbidden: Seq<P>,
    ) -> bool {
        !matches_prefix(path, forbidden) && matches_prefix(path, write_scope)
    }

    pub open spec fn scope_result<P: AsRef<str>>(
        result: Result<(), ScopeViolationKind>, changed: Seq<P>, write_scope: Seq<P>, forbidden: Seq<P>,
    ) -> bool {
        let first_bad = |i: int| {
            &&& 0 <= i < changed.len()
            &&& forall|j: int| 0 <= j < i ==>
                approved_path(#[trigger] path_bytes(&changed[j]), write_scope, forbidden)
        };
        match result {
            Ok(()) => changed.len() > 0 && forall|i: int| 0 <= i < changed.len() ==>
                approved_path(#[trigger] path_bytes(&changed[i]), write_scope, forbidden),
            Err(ScopeViolationKind::EmptyDiff) => changed.len() == 0,
            Err(ScopeViolationKind::ForbiddenPath { index }) => first_bad(index as int) &&
                matches_prefix(path_bytes(&changed[index as int]), forbidden),
            Err(ScopeViolationKind::OutsideWriteScope { index }) => first_bad(index as int) &&
                !matches_prefix(path_bytes(&changed[index as int]), forbidden) &&
                !matches_prefix(path_bytes(&changed[index as int]), write_scope),
        }
    }

    pub open spec fn scope_error(
        result: Result<(), ScopeError>, changed: Seq<RelPath>, write_scope: Seq<RelPath>,
        forbidden: Seq<RelPath>,
    ) -> bool {
        match result {
            Ok(()) => scope_result(Ok(()), changed, write_scope, forbidden),
            Err(ScopeError::EmptyDiff) => scope_result(
                Err(ScopeViolationKind::EmptyDiff), changed, write_scope, forbidden,
            ),
            Err(ScopeError::ForbiddenPath(path)) => exists|i: usize| {
                &&& i < changed.len()
                &&& path == #[trigger] changed[i as int]
                &&& scope_result(
                    Err(ScopeViolationKind::ForbiddenPath { index: i }), changed, write_scope,
                    forbidden,
                )
            },
            Err(ScopeError::OutsideWriteScope(path)) => exists|i: usize| {
                &&& i < changed.len()
                &&& path == #[trigger] changed[i as int]
                &&& scope_result(
                    Err(ScopeViolationKind::OutsideWriteScope { index: i }), changed, write_scope,
                    forbidden,
                )
            },
        }
    }
}

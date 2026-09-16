use super::super::backend::ProviderBinding;
use super::super::{ActionNamespace, binding_error};
use crate::checked_artifact::capability::{
    AsciiComponent, CheckedFsError, ObservedManagedObjectV1, ObservedNamespaceObjectV1,
    RetainedActionNamespaceV1, RetainedManagedParentV1,
};
use crate::checked_artifact::catalog::OpaqueRetainedCatalogV1;
use crate::checked_artifact::protocol::{
    ActionDigestV1, ActionSlotV1, AdmittedActionV1, CatalogNameClassificationV1,
    ProtocolRecordKindV1,
};

/// Opaque proof token carried by every retained namespace capability this
/// backend issues. It is derived from the admitted action's own digest, so it
/// mints no name and reveals no path; its only job is to make a capability
/// forged against another backend fail closed at `validate_operation`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ActionNamespaceHandleV1(pub(crate) [u8; 32]);

/// The one namespace source this backend currently holds retained.
pub(crate) struct RetainedSourceV1 {
    pub(crate) leaf: AsciiComponent,
    pub(crate) kind: ProtocolRecordKindV1,
    pub(crate) observed: ObservedNamespaceObjectV1,
}

/// The one managed component this backend currently holds retained: its parent
/// capability, and the staged directory or ownership marker its next edge will
/// consume.
pub(crate) struct RetainedManagedV1 {
    pub(crate) parent: RetainedManagedParentV1,
    pub(crate) source: Option<ObservedManagedObjectV1>,
}

/// The production `RawNamespaceBackend` over one admitted action directory.
pub(in crate::checked_artifact) struct HostActionNamespaceV1 {
    pub(crate) retained: RetainedActionNamespaceV1,
    pub(crate) provider: ProviderBinding,
    pub(crate) handle: ActionNamespaceHandleV1,
    pub(crate) source: Option<RetainedSourceV1>,
    pub(crate) managed: Option<RetainedManagedV1>,
}

/// The only constructor: an opaque retained catalog plus the admitted action it
/// admitted. There is no path argument and no caller-supplied handle, so a
/// namespace backend cannot exist over anything but a permit-retained,
/// reservation-bound action directory.
pub(in crate::checked_artifact) fn retain_action_namespace(
    catalog: &OpaqueRetainedCatalogV1<'_>,
    admitted: AdmittedActionV1,
) -> Result<ActionNamespace<HostActionNamespaceV1>, CheckedFsError> {
    let retained = catalog.retain_action_namespace(&admitted)?;
    let action = admitted.reservation().action_digest().bytes();
    let backend = HostActionNamespaceV1 {
        retained,
        provider: ProviderBinding::owner_new(admitted.reservation().record_digest().bytes()),
        handle: ActionNamespaceHandleV1(action),
        source: None,
        managed: None,
    };
    Ok(ActionNamespace::from_admitted(backend, admitted))
}

/// OPEN-B3, answered at R2-E Phase E2.1: the reserved target leaf's grammar.
///
/// The leaf must be a canonical `ActionSlotV1` name of **this** action. The
/// gate refuses the dotted `.ca1-*` shape and every other foreign name, because
/// wherever the target parent is catalog-owned `interior::exact_row` walks the
/// frozen infrastructure/action-row grammar and refuses anything else as an
/// unowned child — which makes the catalog unobservable, the `Ambiguous` dead
/// end this whole family is designed around. It refuses another action's slot
/// name for the same reason the grammar is action-scoped: two actions must not
/// be able to collide on one target parent.
///
/// It does **not** name one specific slot: no frozen slot names the *live*
/// alias (`RetiredRoamingAnchorAlias(ordinal)` is its retirement destination),
/// and E2.1 holds no authorization to mint one. The reserved leaf therefore
/// stays the caller's reservation inside a closed grammar, with `create_new` at
/// the alias's creation and the no-replace retirement as the collision guards.
///
/// **And it admits a slot another family is *using*** (E2 review [P3-1]). The
/// grammar is action-scoped, not family-scoped, so `Valid(_)` includes base
/// slots the authority record, the payload writers or the cleanup worklist own.
/// A live collision is a typed refusal — `create_new` fails on an occupied leaf
/// — so this is hygiene rather than corruption, and the alias's own bytes are
/// re-proved at every observation. But it is a real limit of the gate, and it
/// binds the first consumer: **E4 must reserve a leaf no other family of the
/// same action writes**, and must say which slot it picked and why. Narrowing
/// the gate to a family-scoped subset would need a slot this vocabulary does not
/// have, which is E2.1's authorization limit above.
pub(crate) fn require_reserved_target_leaf(
    leaf: &AsciiComponent,
    action: ActionDigestV1,
) -> Result<(), CheckedFsError> {
    match ActionSlotV1::parse(action, leaf.as_bytes()) {
        CatalogNameClassificationV1::Valid(_) => Ok(()),
        _ => Err(binding_error(
            "barrier target leaf is not a scheduled action slot name of this action",
        )),
    }
}

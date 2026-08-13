//! Immutable, ordered cleanup worklist and physical resolution table.

use std::io::Read;

use super::codec::{
    BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1, read_bounded_record_inner,
};
use super::schedule::{ActionDigestV1, RecordDigestV1, RequestOwnerBindingV1, ScheduleDigestV1};
use super::{generated, schedule::checked_array};
use crate::checked_artifact::capability::DurableObjectIdentityV1;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::checked_artifact) enum CleanupAliasV1 {
    Source,
    Goal,
    Authority,
}

impl CleanupAliasV1 {
    pub(in crate::checked_artifact) const ALL: [Self; 3] =
        [Self::Source, Self::Goal, Self::Authority];
    pub(in crate::checked_artifact) const COUNT: usize = Self::ALL.len();

    const fn bit(self) -> u8 {
        match self {
            Self::Source => 1,
            Self::Goal => 2,
            Self::Authority => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CleanupAliasSetV1(u8);

impl CleanupAliasSetV1 {
    pub(in crate::checked_artifact) const fn all() -> Self {
        Self(0b111)
    }

    pub(in crate::checked_artifact) const fn mask(self) -> u8 {
        self.0
    }

    pub(in crate::checked_artifact) const fn from_mask(value: u8) -> Option<Self> {
        if value & !0b111 == 0 {
            Some(Self(value))
        } else {
            None
        }
    }

    fn contains(self, alias: CleanupAliasV1) -> bool {
        self.0 & alias.bit() != 0
    }

    pub(in crate::checked_artifact) fn to_generated(self) -> Vec<generated::CheckedCleanupAlias> {
        CleanupAliasV1::ALL
            .into_iter()
            .filter(|alias| self.contains(*alias))
            .map(CleanupAliasV1::to_generated)
            .collect()
    }

    pub(in crate::checked_artifact) fn from_generated(
        values: &[generated::CheckedCleanupAlias],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let mut mask = 0;
        for value in values {
            let alias = CleanupAliasV1::from_generated(*value);
            if mask & alias.bit() != 0 {
                return Err(ProtocolCodecErrorV1::Invalid("duplicate cleanup alias"));
            }
            mask |= alias.bit();
        }
        Self::from_mask(mask).ok_or(ProtocolCodecErrorV1::Invalid("invalid cleanup alias set"))
    }
}

impl CleanupAliasV1 {
    fn to_generated(self) -> generated::CheckedCleanupAlias {
        match self {
            Self::Source => generated::CheckedCleanupAlias::Source,
            Self::Goal => generated::CheckedCleanupAlias::Goal,
            Self::Authority => generated::CheckedCleanupAlias::Authority,
        }
    }

    fn from_generated(value: generated::CheckedCleanupAlias) -> Self {
        match value {
            generated::CheckedCleanupAlias::Source => Self::Source,
            generated::CheckedCleanupAlias::Goal => Self::Goal,
            generated::CheckedCleanupAlias::Authority => Self::Authority,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct DurableLeafFingerprintV1 {
    identity: DurableObjectIdentityV1,
    length: u64,
    sha256: [u8; 32],
}

impl DurableLeafFingerprintV1 {
    pub(in crate::checked_artifact) fn new(
        identity: DurableObjectIdentityV1,
        length: u64,
        sha256: [u8; 32],
    ) -> Self {
        Self {
            identity,
            length,
            sha256,
        }
    }

    pub(in crate::checked_artifact) fn identity(&self) -> &DurableObjectIdentityV1 {
        &self.identity
    }

    pub(in crate::checked_artifact) const fn length(&self) -> u64 {
        self.length
    }

    pub(in crate::checked_artifact) const fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CleanupRowV1 {
    alias: CleanupAliasV1,
    expected: DurableLeafFingerprintV1,
}

impl CleanupRowV1 {
    pub(in crate::checked_artifact) fn new(
        alias: CleanupAliasV1,
        expected: DurableLeafFingerprintV1,
    ) -> Self {
        Self { alias, expected }
    }

    pub(in crate::checked_artifact) const fn alias(&self) -> CleanupAliasV1 {
        self.alias
    }

    pub(in crate::checked_artifact) fn expected(&self) -> &DurableLeafFingerprintV1 {
        &self.expected
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CleanupWorklistV1 {
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    reservation_digest: RecordDigestV1,
    schedule_digest: ScheduleDigestV1,
    rows: Vec<CleanupRowV1>,
}

impl CleanupWorklistV1 {
    pub(in crate::checked_artifact) fn try_new(
        reservation: &super::ActionCapacityReservationV1,
        rows: Vec<CleanupRowV1>,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let aliases = rows.iter().fold(0, |mask, row| mask | row.alias().bit());
        if CleanupAliasSetV1::from_mask(aliases) != Some(reservation.schedule().cleanup_aliases()) {
            return Err(ProtocolCodecErrorV1::Invalid(
                "cleanup rows do not match the reserved cleanup aliases",
            ));
        }
        Self::from_bound_fields(
            reservation.action_digest(),
            reservation.request_owner_binding(),
            reservation.record_digest(),
            reservation.schedule().digest(),
            rows,
        )
    }

    fn from_bound_fields(
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
        reservation_digest: RecordDigestV1,
        schedule_digest: ScheduleDigestV1,
        rows: Vec<CleanupRowV1>,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        if rows.len() > CleanupAliasV1::COUNT
            || rows
                .windows(2)
                .any(|pair| pair[0].alias() >= pair[1].alias())
            || rows
                .iter()
                .any(|row| !CleanupAliasSetV1::all().contains(row.alias()))
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "cleanup rows are not unique canonical aliases",
            ));
        }
        Ok(Self {
            action_digest,
            request_owner_binding,
            reservation_digest,
            schedule_digest,
            rows,
        })
    }

    pub(in crate::checked_artifact) fn rows(&self) -> &[CleanupRowV1] {
        &self.rows
    }

    pub(in crate::checked_artifact) const fn reservation_digest(&self) -> RecordDigestV1 {
        self.reservation_digest
    }

    pub(in crate::checked_artifact) fn matches_reservation(
        &self,
        reservation: &super::ActionCapacityReservationV1,
    ) -> bool {
        self.action_digest == reservation.action_digest()
            && self.request_owner_binding == reservation.request_owner_binding()
            && self.reservation_digest == reservation.record_digest()
            && self.schedule_digest == reservation.schedule().digest()
            && CleanupAliasSetV1::from_mask(
                self.rows
                    .iter()
                    .fold(0, |mask, row| mask | row.alias().bit()),
            ) == Some(reservation.schedule().cleanup_aliases())
    }

    pub(in crate::checked_artifact) fn encode_canonical(
        &self,
    ) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        Ok(crate::cbor::encode(&self.to_generated().to_cbor()))
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid cleanup taut encoding"))?;
        let wire = generated::CheckedCleanupWorklistV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid cleanup record shape"))?;
        let action = ActionDigestV1::new(checked_array(wire.action_digest)?);
        let owner = RequestOwnerBindingV1::new(checked_array(wire.request_owner_binding)?);
        let reservation = RecordDigestV1::new(checked_array(wire.reservation_digest)?);
        let schedule = ScheduleDigestV1::new(checked_array(wire.schedule_digest)?);
        let mut rows = Vec::new();
        rows.try_reserve_exact(wire.rows.len())
            .map_err(|_| ProtocolCodecErrorV1::Invalid("cleanup allocation failed"))?;
        for row in wire.rows {
            let alias = CleanupAliasV1::from_generated(row.alias);
            let identity = DurableObjectIdentityV1::decode_canonical(&crate::cbor::encode(
                &row.expected.identity.to_cbor(),
            ))
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid cleanup identity"))?;
            rows.push(CleanupRowV1::new(
                alias,
                DurableLeafFingerprintV1::new(
                    identity,
                    u64::from_le_bytes(checked_array(row.expected.length_u64le)?),
                    checked_array(row.expected.sha256)?,
                ),
            ));
        }
        let value = Self::from_bound_fields(action, owner, reservation, schedule, rows)?;
        if value.encode_canonical()? != bytes {
            return Err(ProtocolCodecErrorV1::Invalid("noncanonical cleanup record"));
        }
        Ok(value)
    }

    fn to_generated(&self) -> generated::CheckedCleanupWorklistV1 {
        generated::CheckedCleanupWorklistV1 {
            action_digest: self.action_digest.bytes().to_vec(),
            request_owner_binding: self.request_owner_binding.bytes().to_vec(),
            reservation_digest: self.reservation_digest.bytes().to_vec(),
            schedule_digest: self.schedule_digest.bytes().to_vec(),
            rows: self
                .rows
                .iter()
                .map(|row| generated::CheckedCleanupRowV1 {
                    alias: row.alias.to_generated(),
                    expected: generated::CheckedDurableLeafFingerprintV1 {
                        identity: row.expected.identity.to_generated(),
                        length_u64le: row.expected.length.to_le_bytes().to_vec(),
                        sha256: row.expected.sha256.to_vec(),
                    },
                })
                .collect(),
        }
    }
}

impl BoundedCanonicalRecordV1 for CleanupWorklistV1 {
    const KIND: ProtocolRecordKindV1 = ProtocolRecordKindV1::CleanupWorklist;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        self.encode_canonical()
    }

    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical(bytes)
    }
}

pub(in crate::checked_artifact) struct BoundCleanupWorklistV1(CleanupWorklistV1);

impl BoundCleanupWorklistV1 {
    pub(in crate::checked_artifact) fn len(&self) -> usize {
        self.0.rows.len()
    }

    pub(in crate::checked_artifact) fn is_empty(&self) -> bool {
        self.0.rows.is_empty()
    }

    pub(in crate::checked_artifact) fn row(&self, index: usize) -> Option<BoundCleanupRowV1<'_>> {
        self.0.rows.get(index).map(|row| BoundCleanupRowV1 { row })
    }

    pub(in crate::checked_artifact) fn classify(
        &self,
        index: usize,
        source: &CleanupPhysicalFactV1,
        destination: &CleanupPhysicalFactV1,
    ) -> Option<CleanupResolutionV1> {
        self.0
            .rows
            .get(index)
            .map(|row| classify_cleanup_row(row, source, destination))
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn value(&self) -> &CleanupWorklistV1 {
        &self.0
    }
}

#[derive(Clone, Copy)]
pub(in crate::checked_artifact) struct BoundCleanupRowV1<'a> {
    row: &'a CleanupRowV1,
}

impl BoundCleanupRowV1<'_> {
    pub(in crate::checked_artifact) const fn alias(&self) -> CleanupAliasV1 {
        self.row.alias()
    }

    pub(in crate::checked_artifact) fn expected(&self) -> &DurableLeafFingerprintV1 {
        self.row.expected()
    }
}

pub(in crate::checked_artifact) fn read_and_bind_cleanup_worklist(
    reader: impl Read,
    reservation: &super::ActionCapacityReservationV1,
) -> Result<BoundCleanupWorklistV1, ProtocolCodecErrorV1> {
    let value = read_bounded_record_inner::<CleanupWorklistV1>(reader)?;
    if !value.matches_reservation(reservation) {
        return Err(ProtocolCodecErrorV1::Invalid(
            "cleanup worklist does not match resident reservation",
        ));
    }
    Ok(BoundCleanupWorklistV1(value))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CleanupPhysicalFactV1 {
    Missing,
    Exact(DurableLeafFingerprintV1),
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CleanupResolutionV1 {
    Retire,
    Complete,
    Ambiguous,
}

fn classify_cleanup_row(
    row: &CleanupRowV1,
    source: &CleanupPhysicalFactV1,
    destination: &CleanupPhysicalFactV1,
) -> CleanupResolutionV1 {
    match (source, destination) {
        (CleanupPhysicalFactV1::Exact(value), CleanupPhysicalFactV1::Missing)
            if value == row.expected() =>
        {
            CleanupResolutionV1::Retire
        }
        (CleanupPhysicalFactV1::Missing, CleanupPhysicalFactV1::Exact(value))
            if value == row.expected() =>
        {
            CleanupResolutionV1::Complete
        }
        _ => CleanupResolutionV1::Ambiguous,
    }
}

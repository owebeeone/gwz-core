//! Immutable, ordered cleanup worklist and physical resolution table.

use super::codec::{BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1};
use super::schedule::{ActionDigestV1, RequestOwnerBindingV1, ScheduleDigestV1};
use super::{generated, schedule::checked_array};
use crate::checked_artifact::capability::DurableObjectIdentityV1;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::checked_artifact) enum CleanupAliasV1 {
    Source,
    Goal,
    Authority,
}

impl CleanupAliasV1 {
    const ALL: [Self; 3] = [Self::Source, Self::Goal, Self::Authority];

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
            reservation.schedule().digest(),
            rows,
        )
    }

    fn from_bound_fields(
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
        schedule_digest: ScheduleDigestV1,
        rows: Vec<CleanupRowV1>,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        if rows.len() > 3
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
            schedule_digest,
            rows,
        })
    }

    pub(in crate::checked_artifact) fn rows(&self) -> &[CleanupRowV1] {
        &self.rows
    }

    pub(in crate::checked_artifact) fn matches_reservation(
        &self,
        reservation: &super::ActionCapacityReservationV1,
    ) -> bool {
        self.action_digest == reservation.action_digest()
            && self.request_owner_binding == reservation.request_owner_binding()
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

    pub(in crate::checked_artifact) fn decode_canonical(
        bytes: &[u8],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid cleanup taut encoding"))?;
        let wire = generated::CheckedCleanupWorklistV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid cleanup record shape"))?;
        let action = ActionDigestV1::new(checked_array(wire.action_digest)?);
        let owner = RequestOwnerBindingV1::new(checked_array(wire.request_owner_binding)?);
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
        let value = Self::from_bound_fields(action, owner, schedule, rows)?;
        if value.encode_canonical()? != bytes {
            return Err(ProtocolCodecErrorV1::Invalid("noncanonical cleanup record"));
        }
        Ok(value)
    }

    fn to_generated(&self) -> generated::CheckedCleanupWorklistV1 {
        generated::CheckedCleanupWorklistV1 {
            action_digest: self.action_digest.bytes().to_vec(),
            request_owner_binding: self.request_owner_binding.bytes().to_vec(),
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

pub(in crate::checked_artifact) fn classify_cleanup_row(
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

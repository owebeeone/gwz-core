//! Distinct identity domains and the immutable physical action schedule.

use std::collections::BTreeSet;
use std::ops::Range;

use sha2::{Digest, Sha256};

use crate::checked_artifact::bootstrap::ManagedParentScheduleInputsV1;

use super::bounds::{
    MAX_BARRIER_INVOCATIONS_PER_ACTION, MAX_BOOTSTRAP_INTENT_GENERATIONS,
    MAX_MANAGED_PARENT_BOOTSTRAPS, MAX_MANAGED_PARENT_COMPONENTS,
};
use super::cleanup::CleanupAliasSetV1;
use super::codec::ProtocolCodecErrorV1;
use super::generated;

macro_rules! digest_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub(in crate::checked_artifact) struct $name([u8; 32]);

        impl $name {
            pub(in crate::checked_artifact) const fn new(value: [u8; 32]) -> Self {
                Self(value)
            }

            pub(in crate::checked_artifact) const fn bytes(self) -> [u8; 32] {
                self.0
            }
        }
    };
}

digest_type!(ActionDigestV1);
digest_type!(RequestOwnerBindingV1);
digest_type!(ScheduleDigestV1);
digest_type!(RecordDigestV1);

impl ActionDigestV1 {
    pub(in crate::checked_artifact) fn hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut value = String::with_capacity(64);
        for byte in self.0 {
            value.push(HEX[(byte >> 4) as usize] as char);
            value.push(HEX[(byte & 0x0f) as usize] as char);
        }
        value
    }

    pub(in crate::checked_artifact) fn from_hex(value: &str) -> Option<Self> {
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let mut bytes = [0; 32];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            };
            bytes[index] = (digit(pair[0])? << 4) | digit(pair[1])?;
        }
        Some(Self(bytes))
    }
}

macro_rules! ordinal_type {
    ($name:ident, $limit:expr) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub(in crate::checked_artifact) struct $name(u8);

        impl $name {
            pub(in crate::checked_artifact) fn new(value: usize) -> Result<Self, ScheduleErrorV1> {
                (value < $limit)
                    .then_some(Self(value as u8))
                    .ok_or(ScheduleErrorV1::OutOfBounds)
            }

            pub(in crate::checked_artifact) const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

ordinal_type!(BarrierOrdinalV1, MAX_BARRIER_INVOCATIONS_PER_ACTION);
ordinal_type!(BootstrapOrdinalV1, MAX_MANAGED_PARENT_BOOTSTRAPS);
ordinal_type!(BootstrapGenerationV1, MAX_BOOTSTRAP_INTENT_GENERATIONS);
ordinal_type!(BootstrapComponentOrdinalV1, MAX_MANAGED_PARENT_COMPONENTS);
ordinal_type!(CleanupOrdinalV1, super::bounds::MAX_CLEANUP_ROWS);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ScheduleErrorV1 {
    OutOfBounds,
    EmptyBootstrap,
    DuplicateSpec,
    ArithmeticOverflow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedBootstrapInputV1 {
    spec_digest: [u8; 32],
    component_count: usize,
}

impl ManagedBootstrapInputV1 {
    pub(in crate::checked_artifact) fn new(
        spec_digest: [u8; 32],
        component_count: usize,
    ) -> Result<Self, ScheduleErrorV1> {
        if !(1..=MAX_MANAGED_PARENT_COMPONENTS).contains(&component_count) {
            return Err(ScheduleErrorV1::EmptyBootstrap);
        }
        Ok(Self {
            spec_digest,
            component_count,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedBootstrapRowV1 {
    spec_digest: [u8; 32],
    ordinal: BootstrapOrdinalV1,
    generation_range: Range<usize>,
    component_range: Range<usize>,
}

impl ManagedBootstrapRowV1 {
    pub(in crate::checked_artifact) const fn spec_digest(&self) -> [u8; 32] {
        self.spec_digest
    }

    pub(in crate::checked_artifact) const fn ordinal(&self) -> BootstrapOrdinalV1 {
        self.ordinal
    }

    pub(in crate::checked_artifact) fn generation_range(&self) -> Range<usize> {
        self.generation_range.clone()
    }

    pub(in crate::checked_artifact) fn component_range(&self) -> Range<usize> {
        self.component_range.clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ActionScheduleV1 {
    barrier_count: usize,
    bootstrap_rows: Vec<ManagedBootstrapRowV1>,
    component_count: usize,
    generation_count: usize,
    cleanup_aliases: CleanupAliasSetV1,
    managed_plan_digest: [u8; 32],
    digest: ScheduleDigestV1,
}

impl ActionScheduleV1 {
    pub(in crate::checked_artifact) fn try_new(
        barrier_count: usize,
        inputs: Vec<ManagedBootstrapInputV1>,
        cleanup_aliases: CleanupAliasSetV1,
    ) -> Result<Self, ScheduleErrorV1> {
        Self::try_new_inner(barrier_count, inputs, cleanup_aliases, None, true)
    }

    pub(in crate::checked_artifact) fn try_from_managed_plan(
        barrier_count: usize,
        inputs: &ManagedParentScheduleInputsV1,
        cleanup_aliases: CleanupAliasSetV1,
    ) -> Result<Self, ScheduleErrorV1> {
        Self::try_new_inner(
            barrier_count,
            inputs.rows().to_vec(),
            cleanup_aliases,
            Some(inputs.plan_digest()),
            false,
        )
    }

    fn try_new_inner(
        barrier_count: usize,
        mut inputs: Vec<ManagedBootstrapInputV1>,
        cleanup_aliases: CleanupAliasSetV1,
        managed_plan_digest: Option<[u8; 32]>,
        canonicalize_legacy_inputs: bool,
    ) -> Result<Self, ScheduleErrorV1> {
        if barrier_count > MAX_BARRIER_INVOCATIONS_PER_ACTION
            || inputs.len() > MAX_MANAGED_PARENT_BOOTSTRAPS
        {
            return Err(ScheduleErrorV1::OutOfBounds);
        }
        if canonicalize_legacy_inputs {
            inputs.sort_by_key(|input| input.spec_digest);
        }
        let unique = inputs
            .iter()
            .map(|input| input.spec_digest)
            .collect::<BTreeSet<_>>();
        if unique.len() != inputs.len() {
            return Err(ScheduleErrorV1::DuplicateSpec);
        }
        let component_count = inputs.iter().try_fold(0usize, |sum, input| {
            sum.checked_add(input.component_count)
                .ok_or(ScheduleErrorV1::ArithmeticOverflow)
        })?;
        if component_count > MAX_MANAGED_PARENT_COMPONENTS {
            return Err(ScheduleErrorV1::OutOfBounds);
        }

        let mut component_start = 0usize;
        let mut generation_start = 0usize;
        let mut rows = Vec::new();
        rows.try_reserve_exact(inputs.len())
            .map_err(|_| ScheduleErrorV1::ArithmeticOverflow)?;
        for (index, input) in inputs.into_iter().enumerate() {
            let component_end = component_start + input.component_count;
            let generation_end = generation_start + 1 + 2 * input.component_count;
            rows.push(ManagedBootstrapRowV1 {
                spec_digest: input.spec_digest,
                ordinal: BootstrapOrdinalV1::new(index)?,
                generation_range: generation_start..generation_end,
                component_range: component_start..component_end,
            });
            component_start = component_end;
            generation_start = generation_end;
        }
        if generation_start > MAX_BOOTSTRAP_INTENT_GENERATIONS {
            return Err(ScheduleErrorV1::OutOfBounds);
        }
        let managed_plan_digest =
            managed_plan_digest.unwrap_or_else(|| digest_schedule_rows(&rows));
        let mut value = Self {
            barrier_count,
            bootstrap_rows: rows,
            component_count,
            generation_count: generation_start,
            cleanup_aliases,
            managed_plan_digest,
            digest: ScheduleDigestV1::new([0; 32]),
        };
        value.digest = ScheduleDigestV1::new(Sha256::digest(value.digest_material()).into());
        Ok(value)
    }

    pub(in crate::checked_artifact) const fn generation_count(&self) -> usize {
        self.generation_count
    }

    pub(in crate::checked_artifact) const fn barrier_count(&self) -> usize {
        self.barrier_count
    }

    pub(in crate::checked_artifact) const fn component_count(&self) -> usize {
        self.component_count
    }

    pub(in crate::checked_artifact) fn bootstrap_rows(&self) -> &[ManagedBootstrapRowV1] {
        &self.bootstrap_rows
    }

    pub(in crate::checked_artifact) const fn digest(&self) -> ScheduleDigestV1 {
        self.digest
    }

    pub(in crate::checked_artifact) const fn cleanup_aliases(&self) -> CleanupAliasSetV1 {
        self.cleanup_aliases
    }

    pub(in crate::checked_artifact) const fn managed_plan_digest(&self) -> [u8; 32] {
        self.managed_plan_digest
    }

    pub(in crate::checked_artifact) fn encode_canonical(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated().to_cbor())
    }

    pub(super) fn decode_canonical(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid schedule taut encoding"))?;
        let wire = generated::CheckedActionScheduleV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid schedule record shape"))?;
        let barrier_count = checked_usize(wire.barrier_count)?;
        let inputs = wire
            .bootstraps
            .into_iter()
            .map(|input| {
                ManagedBootstrapInputV1::new(
                    checked_array(input.spec_digest)?,
                    checked_usize(input.component_count)?,
                )
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid bootstrap schedule"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let aliases = CleanupAliasSetV1::from_generated(&wire.cleanup_aliases)?;
        let stored_digest = checked_array(wire.schedule_digest)?;
        let stored_plan_digest = checked_array(wire.managed_plan_digest)?;
        let value = Self::try_new_inner(
            barrier_count,
            inputs,
            aliases,
            Some(stored_plan_digest),
            false,
        )
        .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid schedule"))?;
        if value.digest.bytes() != stored_digest
            || value.managed_plan_digest != stored_plan_digest
            || value.encode_canonical() != bytes
        {
            return Err(ProtocolCodecErrorV1::Invalid("noncanonical schedule"));
        }
        Ok(value)
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn test_decode_canonical(
        bytes: &[u8],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical(bytes)
    }

    fn digest_material(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated_with_digest(Vec::new()).to_cbor())
    }

    pub(in crate::checked_artifact) fn to_generated(&self) -> generated::CheckedActionScheduleV1 {
        self.to_generated_with_digest(self.digest.bytes().to_vec())
    }

    fn to_generated_with_digest(
        &self,
        schedule_digest: Vec<u8>,
    ) -> generated::CheckedActionScheduleV1 {
        generated::CheckedActionScheduleV1 {
            barrier_count: self.barrier_count as i64,
            bootstraps: self
                .bootstrap_rows
                .iter()
                .map(|row| generated::CheckedManagedBootstrapInputV1 {
                    spec_digest: row.spec_digest.to_vec(),
                    component_count: (row.component_range.end - row.component_range.start) as i64,
                })
                .collect(),
            cleanup_aliases: self.cleanup_aliases.to_generated(),
            schedule_digest,
            managed_plan_digest: self.managed_plan_digest.to_vec(),
        }
    }
}

fn digest_schedule_rows(rows: &[ManagedBootstrapRowV1]) -> [u8; 32] {
    let material = rows
        .iter()
        .map(|row| {
            generated::CheckedManagedBootstrapInputV1 {
                spec_digest: row.spec_digest.to_vec(),
                component_count: (row.component_range.end - row.component_range.start) as i64,
            }
            .to_cbor()
        })
        .collect();
    Sha256::digest(crate::cbor::encode(&crate::cbor::Cbor::Array(material))).into()
}

pub(in crate::checked_artifact) fn checked_array<const N: usize>(
    value: Vec<u8>,
) -> Result<[u8; N], ProtocolCodecErrorV1> {
    value
        .try_into()
        .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid fixed-size byte field"))
}

pub(in crate::checked_artifact) fn checked_usize(
    value: i64,
) -> Result<usize, ProtocolCodecErrorV1> {
    usize::try_from(value).map_err(|_| ProtocolCodecErrorV1::Invalid("invalid unsigned integer"))
}

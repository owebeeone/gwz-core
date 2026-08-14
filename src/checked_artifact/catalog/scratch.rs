use crate::checked_artifact::capability::{
    CheckedFsError, DurableCatalogTargetDigestV1, HistoricalCollisionDigestV1,
};
use crate::checked_artifact::protocol::CatalogBootstrapOwnershipTokenV1;

const SCRATCH_PREFIX: &[u8] = b"checked-artifacts-catalog-bootstrap-v1.scratch.";
const HEX_BYTES: usize = 64;
pub(in crate::checked_artifact) const CATALOG_SCRATCH_NAME_BYTES_V1: usize =
    SCRATCH_PREFIX.len() + HEX_BYTES * 3 + 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogScratchNameV1 {
    bytes: [u8; CATALOG_SCRATCH_NAME_BYTES_V1],
    durable_target_digest: DurableCatalogTargetDigestV1,
    historical_collision_digest: HistoricalCollisionDigestV1,
    ownership_token: CatalogBootstrapOwnershipTokenV1,
}

impl CatalogScratchNameV1 {
    pub(in crate::checked_artifact) fn new(
        durable_target_digest: DurableCatalogTargetDigestV1,
        historical_collision_digest: HistoricalCollisionDigestV1,
        ownership_token: CatalogBootstrapOwnershipTokenV1,
    ) -> Self {
        let mut bytes = [0; CATALOG_SCRATCH_NAME_BYTES_V1];
        let mut offset = 0;
        append(&mut bytes, &mut offset, SCRATCH_PREFIX);
        append_hex(&mut bytes, &mut offset, &durable_target_digest.bytes());
        bytes[offset] = b'.';
        offset += 1;
        append_hex(
            &mut bytes,
            &mut offset,
            &historical_collision_digest.bytes(),
        );
        bytes[offset] = b'.';
        offset += 1;
        append_hex(&mut bytes, &mut offset, ownership_token.as_bytes());
        debug_assert_eq!(offset, CATALOG_SCRATCH_NAME_BYTES_V1);
        Self {
            bytes,
            durable_target_digest,
            historical_collision_digest,
            ownership_token,
        }
    }

    pub(in crate::checked_artifact) fn parse(bytes: &[u8]) -> Result<Self, CheckedFsError> {
        if bytes.len() != CATALOG_SCRATCH_NAME_BYTES_V1 || !bytes.starts_with(SCRATCH_PREFIX) {
            return Err(invalid_name());
        }
        let mut offset = SCRATCH_PREFIX.len();
        let target = decode_hex(&bytes[offset..offset + HEX_BYTES])?;
        offset += HEX_BYTES;
        if bytes.get(offset) != Some(&b'.') {
            return Err(invalid_name());
        }
        offset += 1;
        let historical = decode_hex(&bytes[offset..offset + HEX_BYTES])?;
        offset += HEX_BYTES;
        if bytes.get(offset) != Some(&b'.') {
            return Err(invalid_name());
        }
        offset += 1;
        let token = decode_hex(&bytes[offset..offset + HEX_BYTES])?;
        let ownership_token = CatalogBootstrapOwnershipTokenV1::try_from_random_bytes(token)
            .map_err(|_| invalid_name())?;
        let parsed = Self::new(
            DurableCatalogTargetDigestV1::owner_issue(target),
            HistoricalCollisionDigestV1::owner_issue(historical),
            ownership_token,
        );
        if parsed.as_bytes() != bytes {
            return Err(invalid_name());
        }
        Ok(parsed)
    }

    pub(in crate::checked_artifact) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(in crate::checked_artifact) const fn durable_target_digest(
        &self,
    ) -> DurableCatalogTargetDigestV1 {
        self.durable_target_digest
    }

    pub(in crate::checked_artifact) const fn historical_collision_digest(
        &self,
    ) -> HistoricalCollisionDigestV1 {
        self.historical_collision_digest
    }

    pub(in crate::checked_artifact) const fn ownership_token(
        &self,
    ) -> CatalogBootstrapOwnershipTokenV1 {
        self.ownership_token
    }

    pub(super) const fn prefix() -> &'static [u8] {
        SCRATCH_PREFIX
    }
}

fn append<const N: usize>(target: &mut [u8; N], offset: &mut usize, value: &[u8]) {
    target[*offset..*offset + value.len()].copy_from_slice(value);
    *offset += value.len();
}

fn append_hex<const N: usize>(target: &mut [u8; N], offset: &mut usize, value: &[u8; 32]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in value {
        target[*offset] = HEX[usize::from(byte >> 4)];
        target[*offset + 1] = HEX[usize::from(byte & 0x0f)];
        *offset += 2;
    }
}

fn decode_hex(value: &[u8]) -> Result<[u8; 32], CheckedFsError> {
    if value.len() != HEX_BYTES {
        return Err(invalid_name());
    }
    let mut decoded = [0; 32];
    for (index, pair) in value.chunks_exact(2).enumerate() {
        decoded[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(decoded)
}

fn nibble(value: u8) -> Result<u8, CheckedFsError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(invalid_name()),
    }
}

fn invalid_name() -> CheckedFsError {
    CheckedFsError::ambiguous(
        "catalog scratch name",
        "scratch name is not the exact lowercase v1 grammar",
    )
}

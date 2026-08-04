mod decode;
mod header;
mod raw_yaml;
mod scalar;

#[cfg(test)]
mod open_v0;
#[cfg(test)]
mod unknown_fields;

#[cfg(test)]
pub(crate) use open_v0::{
    OpenV0Adaptation, VerifiedV0Descriptor, adapt_open_v0_for_r3_tests, verified_v0_descriptor,
};

#[cfg(test)]
pub(crate) fn decode_v0_for_r3_tests(
    bytes: &[u8],
) -> Result<decode::DecodedV0Record, decode::RecordDecodeError> {
    decode::decode_production_v0(bytes)
}

pub(super) use decode::{RecordDecodeError, decode_production_v0};
pub(super) use header::{HeaderClassificationError, HeaderMalformedReason, MergeRecordHeader};
pub(super) use raw_yaml::StrictYamlError;

#[cfg(test)]
mod tests;

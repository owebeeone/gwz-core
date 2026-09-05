//! Typed object ids across the git2 boundary.
//!
//! Architecture §4: "Use typed OIDs including the repository's object format;
//! do not assume 40-character SHA-1." Every id that crosses out of this crate
//! is built from the repository's own [`ObjectFormat`], and the length check
//! in `ObjectId::from_bytes` is the witness — a digest whose length does not
//! match the repository's format is an error, never a silently truncated id.

use gwz_repo_contract::{ObjectFormat, ObjectId};

pub(crate) fn to_contract_oid(format: ObjectFormat, oid: git2::Oid) -> Result<ObjectId, String> {
    ObjectId::from_bytes(format, oid.as_bytes()).map_err(|error| error.to_string())
}

pub(crate) fn to_git_oid(oid: &ObjectId) -> Result<git2::Oid, String> {
    git2::Oid::from_bytes(oid.as_bytes()).map_err(|error| error.message().to_owned())
}

pub(crate) fn git_format(format: ObjectFormat) -> git2::ObjectFormat {
    match format {
        ObjectFormat::Sha1 => git2::ObjectFormat::Sha1,
        ObjectFormat::Sha256 => git2::ObjectFormat::Sha256,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_in_both_formats_and_a_length_mismatch_is_an_error() {
        for (format, hex) in [
            (ObjectFormat::Sha1, "ab".repeat(20)),
            (ObjectFormat::Sha256, "cd".repeat(32)),
        ] {
            let contract = ObjectId::parse_hex(format, &hex).expect("fixture id");
            let git = to_git_oid(&contract).expect("git id");
            assert_eq!(git.to_string(), hex);
            assert_eq!(to_contract_oid(format, git).expect("back"), contract);
        }
        let sha1 = git2::Oid::from_bytes(&[7u8; 20]).expect("sha1 id");
        assert!(to_contract_oid(ObjectFormat::Sha256, sha1).is_err());
    }
}

#![forbid(clippy::disallowed_methods)]

use super::*;

/// Whether this workspace's volume can prove the durable identity the checked
/// catalog needs for crash recovery.
///
/// DR-1 ship (1) W3 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §2, 2026-09-03).
/// Crash recovery is a CAPABILITY, not a gate: below the bar the merge still
/// runs, warns once and activates no catalog; `--filesystem-strict` is the only
/// way to turn the absence back into a refusal.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CrashRecoveryDecision {
    /// Identity proved; the catalog is activated exactly as it is today.
    Supported,
    /// Identity absent. `filesystem` is the volume's own name where the
    /// platform can give one, rendered `unknown` where it cannot.
    Unsupported {
        filesystem: Option<String>,
        gap: crate::MergeCrashRecoveryGap,
        /// M5d (`GwzM5-8M5d-Charter.md` §3, 2026-09-03): whether this volume
        /// proves the PERSISTENT FILE HANDLES the checked boundary's doors
        /// need — a second, independent question from the catalog's identity
        /// bar above it.
        ///
        /// The gap does not settle it, which is the whole reason this field
        /// exists: on Linux `no_durable_identity` implies the handle probe
        /// already failed, but `remote` and `volatile` do not — NFS and tmpfs
        /// answer `name_to_handle_at` — and those volumes must NOT carry the
        /// reverse-door limit. `false` means the record create publishes raw
        /// and the reverse doors may refuse; `true` is today's behaviour
        /// unchanged. It is carried only BELOW the bar: above it a handle
        /// failure is an anomaly at the door, not a capability the merge
        /// plans around.
        handles_ok: bool,
    },
}

/// The clause the ONE diagnostic gains on a handle-fail volume
/// (`GwzM5-8M5d-Charter.md` §3, "Reverse doors on handle-fail volumes",
/// third bullet: "Start on those volumes states this limit in the same
/// diagnostic (not a second unrelated warning class)").
///
/// It states the limit and stops. The escape itself is not repeated here:
/// the door that actually refuses renders it in full
/// (`capability::HANDLE_FAIL_REVERSE_DOOR_ESCAPE`), and a start that may
/// never abort at all should not be handed an abort procedure.
///
/// It begins with a space and is APPENDED, so ship (1)'s sentence stays
/// byte-identical at the head of the message for every pin that matches it.
const REVERSE_DOOR_LIMIT: &str = " Selected-root and --preserve abort may refuse until the workspace is on a handle-capable \
     volume.";

/// **On these three names.** The charter's own shorthand for them is
/// `to_protocol`, `warning` and the strict sentence; they are spelled with the
/// `crash_recovery_` prefix here because this module's `pub(crate)` surface is
/// an equality-pinned INVENTORY —
/// `check_checked_artifact_boundaries.py`'s `ENTRY_REFERENCES` scans every
/// production file for each visible name and requires the reference set to be
/// exactly the boundary's consumers. A bare `warning` or `to_protocol` matches
/// nine unrelated files between them, which is why every door in this file
/// already carries a long distinctive name. Same items, checkable names.
impl CrashRecoveryDecision {
    /// The response's machine truth (charter §3.4 channel 2): every consumer
    /// that must not depend on stderr reads this, not the diagnostic.
    pub(crate) fn crash_recovery_protocol(&self) -> crate::MergeCrashRecovery {
        match self {
            Self::Supported => crate::MergeCrashRecovery {
                supported: true,
                filesystem: None,
                gap: None,
                // M5d charter §3: ABSENT above the bar. The field says how a
                // below-bar merge behaves; above the bar there is nothing for
                // a consumer to plan around.
                handles_ok: None,
            },
            Self::Unsupported {
                filesystem,
                gap,
                handles_ok,
            } => crate::MergeCrashRecovery {
                supported: false,
                filesystem: filesystem.clone(),
                gap: Some(*gap),
                handles_ok: Some(*handles_ok),
            },
        }
    }

    /// The operator's exact sentence (charter §3.4). Drivers print `warning: `
    /// + this string on stderr; the `warning: ` prefix is theirs, not core's.
    ///
    /// **M5d (`GwzM5-8M5d-Charter.md` §3, 2026-09-03): ONE diagnostic, not
    /// two.** When the volume also fails the handle probe, this SAME string
    /// gains the reverse-door limit as an appended clause. Ship (1)'s sentence
    /// is byte-identical in front of it — the docs-manifest regex and the
    /// gwz-cli / gwz-py echo pins all match the first sentence — because a
    /// second Diagnostic would be a second warning class, which the charter
    /// forbids ("No second warning for the raw write itself").
    pub(crate) fn crash_recovery_warning(&self) -> String {
        let warning = format!(
            "{}. Merge will continue. Use --filesystem-strict to refuse.",
            self.gap_sentence()
        );
        match self {
            Self::Unsupported {
                handles_ok: false, ..
            } => format!("{warning}{REVERSE_DOOR_LIMIT}"),
            Self::Supported | Self::Unsupported { .. } => warning,
        }
    }

    /// The `--filesystem-strict` refusal (charter §3.6): the same sentence,
    /// then the one remedy a user can act on.
    pub(crate) fn crash_recovery_strict_refusal(&self) -> ModelError {
        ModelError::new(
            ErrorCode::UnsupportedOperation,
            format!(
                "checked catalog: {}; {}",
                self.gap_sentence(),
                super::capability::PERSISTENT_FILESYSTEM_IDENTITY_REMEDY
            ),
        )
    }

    /// `crash recovery is unsupported on <fs> (<parenthetical>)`, shared by the
    /// warning and the strict refusal so the two can never word the gap
    /// differently. `Supported` has no gap and never reaches either caller.
    fn gap_sentence(&self) -> String {
        let (filesystem, gap) = match self {
            Self::Supported => (None, None),
            Self::Unsupported {
                filesystem, gap, ..
            } => (filesystem.as_deref(), Some(*gap)),
        };
        let parenthetical = match gap {
            Some(crate::MergeCrashRecoveryGap::RemoteFilesystem) => "remote filesystem",
            Some(crate::MergeCrashRecoveryGap::VolatileFilesystem) => "volatile filesystem",
            Some(crate::MergeCrashRecoveryGap::NoDurableIdentity) | None => {
                "no durable filesystem identity"
            }
        };
        format!(
            "crash recovery is unsupported on {} ({parenthetical})",
            filesystem.unwrap_or("unknown")
        )
    }
}

pub(crate) fn crash_recovery_decision_in(
    context: &crate::operation_context::OperationServices,
    root: &Path,
) -> ModelResult<CrashRecoveryDecision> {
    let probe = probe_workspace_admission(context, root);
    let cause = match probe.admitted {
        Ok(()) => return Ok(CrashRecoveryDecision::Supported),
        Err(cause) => cause,
    };
    if let CheckedFsError::Ambiguous { .. } = cause {
        return Err(render_catalog_refusal(CATALOG_LABEL, cause));
    }
    let (filesystem, gap) = match probe.volume {
        Some(volume) if volume.volatile => (
            volume.name,
            crate::MergeCrashRecoveryGap::VolatileFilesystem,
        ),
        Some(volume) if volume.remote => {
            (volume.name, crate::MergeCrashRecoveryGap::RemoteFilesystem)
        }
        Some(volume) => (volume.name, crate::MergeCrashRecoveryGap::NoDurableIdentity),
        None => (None, crate::MergeCrashRecoveryGap::NoDurableIdentity),
    };
    Ok(CrashRecoveryDecision::Unsupported {
        filesystem,
        gap,
        handles_ok: directory_handles_ok(context.filesystem(), root),
    })
}

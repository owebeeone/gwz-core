"""GWZ v0 taut protocol schema."""

from taut.ir.dsl import BOOL, BYTES, INT, STR, Enum, F, List, Msg, Params, Ref, method, schema, service

SCHEMA = schema(
    # ---- service ----------------------------------------------------------
    service("GwzCore",
        # Create an empty workspace artifact set.
        method("create_workspace", role="in",
               params=Params(request=Ref.CreateWorkspaceRequest),
               out=Ref.CreateWorkspaceResponse),
        # Initialize a workspace from source URLs.
        method("init_from_sources", role="in",
               params=Params(request=Ref.InitFromSourcesRequest),
               out=Ref.InitFromSourcesResponse),
        # Clone a workspace root repository and materialize its locked members.
        method("clone_workspace", role="in",
               params=Params(request=Ref.CloneWorkspaceRequest),
               out=Ref.CloneWorkspaceResponse),
        # Register an existing local Git repository.
        method("add_existing_repo", role="in",
               params=Params(request=Ref.AddExistingRepoRequest),
               out=Ref.AddExistingRepoResponse),
        # Create a new local member repository.
        method("create_repo", role="in",
               params=Params(request=Ref.CreateRepoRequest),
               out=Ref.CreateRepoResponse),
        # Refresh registered member metadata from local Git repositories.
        method("repo_sync", role="in",
               params=Params(request=Ref.RepoSyncRequest),
               out=Ref.RepoSyncResponse),
        # Clone and register one repository as a workspace member.
        method("clone_repo_member", role="in",
               params=Params(request=Ref.CloneRepoMemberRequest),
               out=Ref.CloneRepoMemberResponse),
        # Soft-remove one active member from the current workspace composition.
        method("detach_repo_member", role="in",
               params=Params(request=Ref.DetachRepoMemberRequest),
               out=Ref.DetachRepoMemberResponse),
        # Reactivate one historical member designation.
        method("attach_repo_member", role="in",
               params=Params(request=Ref.AttachRepoMemberRequest),
               out=Ref.AttachRepoMemberResponse),
        # Move members to a lock/snapshot/tag/commit target.
        method("materialize", role="in",
               params=Params(request=Ref.MaterializeRequest),
               out=Ref.MaterializeResponse),
        # Read member Git and lock status.
        method("status", role="in",
               params=Params(request=Ref.StatusRequest),
               out=Ref.StatusResponse),
        # List the workspace's members (id, path, materialized).
        method("ls", role="in",
               params=Params(request=Ref.LsRequest),
               out=Ref.LsResponse),
        # Capture selected member state by snapshot id.
        method("snapshot", role="in",
               params=Params(request=Ref.SnapshotRequest),
               out=Ref.SnapshotResponse),
        # List workspace snapshots through the canonical core API.
        method("list_snapshots", role="in",
               params=Params(request=Ref.ListSnapshotsRequest),
               out=Ref.ListSnapshotsResponse),
        # Capture selected member state by GWZ tag name.
        method("tag", role="in",
               params=Params(request=Ref.TagRequest),
               out=Ref.TagResponse),
        # Capture live observed member state into the lock (no worktree mutation).
        method("capture", role="in",
               params=Params(request=Ref.CaptureRequest),
               out=Ref.CaptureResponse),
        # Commit staged (or, with all, tracked-modified) changes across members + root.
        method("commit", role="in",
               params=Params(request=Ref.CommitRequest),
               out=Ref.CommitResponse),
        # Stage pathspecs in the owning member/root repos (multi-repo git add).
        method("stage", role="in",
               params=Params(request=Ref.StageRequest),
               out=Ref.StageResponse),
        # Fetch and fast-forward to upstream heads.
        method("pull_head", role="in",
               params=Params(request=Ref.PullHeadRequest),
               out=Ref.PullHeadResponse),
        # Materialize members to a named snapshot.
        method("pull_snapshot", role="in",
               params=Params(request=Ref.PullSnapshotRequest),
               out=Ref.PullSnapshotResponse),
        # Push selected members to remotes.
        method("push", role="in",
               params=Params(request=Ref.PushRequest),
               out=Ref.PushResponse),
        # Coordinate native Git stashes across selected members.
        method("stash", role="in",
               params=Params(request=Ref.StashRequest),
               out=Ref.StashResponse),
        # Manage local Git branches across selected members.
        method("branch", role="in",
               params=Params(request=Ref.BranchRequest),
               out=Ref.BranchResponse),
        # Coordinate a recoverable Git merge across the frozen workspace selection.
        method("merge", role="in",
               params=Params(request=Ref.MergeRequest),
               out=Ref.MergeResponse),
        # Stream operation events by operation id.
        method("events.subscribe", role="out", shape="log",
               params=Params(operation_id=STR),
               out=Ref.OperationEvent),
        # Read the final operation result by operation id.
        method("operation.result", role="out",
               params=Params(operation_id=STR),
               out=Ref.OperationResult),
        # Plan the workspace diff: resolve targets/operands/pathspecs per repo and
        # return a changed-file manifest, aggregate + per-repo summary, scoped
        # operand classification (DiffParsedTarget), intentionally excluded
        # targets, and — unless the requested mode has no byte output — a
        # DiffOutputLogRef handle to the diff.output log.
        method("diff", role="in",
               params=Params(request=Ref.DiffRequest),
               out=Ref.DiffManifestResponse),
        # Read exact diff output records. shape="log": the append type is
        # DiffOutputRecord, delivered as the payload of the generic taut-shape
        # LogRecord. log_id comes from DiffManifestResponse.output.log_id.
        # Cursor/tail/EOF/close/backpressure/retention are the shape_log contract
        # — NOT fields here; params is only the log_id handle, mirroring the
        # events.subscribe shape="log" precedent above (operation_id only).
        method("diff.output", role="out", shape="log",
               params=Params(log_id=STR),
               out=Ref.DiffOutputRecord)),

    # ---- enums ------------------------------------------------------------
    # Operation action inferred from request type.
    ActionKind=Enum(
         create_workspace=0,
         init_from_sources=1,
         add_existing_repo=2,
         create_repo=3,
         materialize=4,
         status=5,
         snapshot=6,
         tag=7,
         pull_head=8,
         pull_snapshot=9,
         push=10,
         capture=11,
         commit=12,
         stage=13,
         ls=14,
         forall=15,
         repo_sync=16,
         stash=17,
         branch=18,
         clone_workspace=19,
         list_snapshots=20,
         diff=21,
         clone_repo_member=22,
         detach_repo_member=23,
         attach_repo_member=24,
         merge=25),

    # Operation kind for the `gwz tag` verb.
    TagOp=Enum(
         create=0,
         list=1,
         fetch=2,
         push=3,
         delete=4),

    # Operation kind for the `gwz stash` verb.
    StashOp=Enum(
         push=0,
         list=1,
         apply=2,
         pop=3,
         drop=4),

    # Pre-attempt membership classification for a stash bundle member.
    StashParticipation=Enum(
         stashed=0,
         empty=1,
         skipped=2),

    # Native stash save lifecycle for a stash bundle member.
    StashPushLifecycle=Enum(
         unattempted=0,
         saving=1,
         saved=2,
         empty=3,
         failed=4),

    # Restore/drop state for a stash bundle member.
    StashRestoreState=Enum(
         pending=0,
         applied=1,
         popped=2,
         dropped=3,
         noop=4,
         missing=5),

    # Operation kind for the `gwz branch` verb.
    BranchOp=Enum(
         list=0,
         create=1,
         delete=2,
         merge=3),

    # Lifecycle operation for the first-class `gwz merge` service method.
    MergeOp=Enum(
         start=0,
         resume=1,
         abort=2,
         status=3,
         gc=4),

    # Requested integration strategy. Non-normal strategies are reserved until M4.
    MergeMode=Enum(
         normal=0,
         ff_only=1,
         no_ff=2),

    # Read-only prediction returned by merge analysis and dry-run planning.
    MergeAnalysisKind=Enum(
         up_to_date=0,
         fast_forward=1,
         true_merge=2,
         unknown=3),

    # Exact mutating action journaled before Git is invoked.
    MergePendingActionKind=Enum(
         verify_up_to_date=0,
         fast_forward=1,
         true_merge=2,
         resolve_conflict=3),

    # Read-only reconciliation of a durable pending action against live Git.
    MergePendingActionState=Enum(
         not_started=0,
         expected_conflict=1,
         completed_exactly=2,
         ambiguous=3),

    # Durable lifecycle for one frozen merge participant.
    MergeParticipantState=Enum(
         planned=0,
         up_to_date=1,
         fast_forwarded=2,
         merged=3,
         conflicted=4,
         failed=5,
         unattempted=6,
         continued=7,
         aborted=8,
         rolled_back=9),

    # Durable lifecycle for the coordinated merge operation.
    MergeOperationState=Enum(
         executing=0,
         awaiting_resolution=1,
         halted=2,
         finalizing=3,
         preserving=4,
         rolling_back=5,
         completed=6,
         aborted=7,
         recovery_required=8,
         # Read-only status result when no coordinated merge is open.
         idle=9),

    # Participant state that differs from the durable merge record.
    MergeParticipantDriftKind=Enum(
         branch_changed=0,
         head_advanced=1,
         head_rewound=2,
         target_ref_changed=3,
         worktree_modified=4,
         index_modified=5,
         merge_state_missing=6,
         merge_head_changed=7,
         new_integration_state=8,
         repository_missing=9,
         head_diverged=10,
         object_missing=11,
         foreign_integration_state=12,
         pending_action_ambiguous=13),

    # Workspace/record state that differs from the durable merge baseline.
    MergeOperationDriftKind=Enum(
         baseline_lock_changed=0,
         baseline_manifest_changed=1,
         root_candidate_metadata_invalid=2,
         root_candidate_state_changed=3,
         record_unreadable=4),

    # Monotonic publication checkpoint while an operation is finalizing.
    MergePublicationStep=Enum(
         not_started=0,
         validating_results=1,
         preparing_candidate=2,
         committing_evidence=3,
         publishing_candidate=4,
         verifying_publication=5,
         complete=6),

    # Per-repository branch action result. Merge-only values are appended by B5a.
    BranchActionResult=Enum(
         listed=0,
         created=1,
         exists=2,
         deleted=3,
         switched=4,
         noop=5,
         skipped=6,
         merged=7,
         conflicted=8),

    # How `gwz forall` runs the command: direct argv, or via a shell.
    ExecMode=Enum(
         argv=0,
         shell=1),

    # Source backing a workspace member.
    SourceKind=Enum(
         git=0,
         archive=1,
         package=2,
         local=3,
         generated=4),

    # Concrete target kind selected by workspace-wide target selectors.
    TargetKind=Enum(
         root=0,
         member=1),

    # Whole-operation status across selected members.
    AggregateStatus=Enum(
         accepted=0,
         ok=1,
         noop=2,
         rejected=3,
         partial=4,
         failed=5,
         dirty=6,
         conflicted=7),

    # Per-member result status.
    MemberStatus=Enum(
         planned=0,
         ok=1,
         noop=2,
         skipped=3,
         rejected=4,
         failed=5,
         conflicted=6),

    # Explicit materialization target kind.
    MaterializeTargetKind=Enum(
         lock=0,
         head=1,
         snapshot=2,
         tag=3,
         commit=4,
         branch=5),

    # Source used when capturing a snapshot.
    SnapshotSourceKind=Enum(
         current=0,
         branch=1),

    # Requested pull/head sync behavior.
    SyncBehavior=Enum(
         fetch_only=0,
         ff_only=1,
         merge=2,
         rebase=3,
         reset=4,
         driver_selected=5),

    # Whether a member failure can be isolated.
    PartialBehavior=Enum(
         atomic=0,
         partial=1),

    # Whether destructive behavior is allowed.
    DestructiveBehavior=Enum(
         refuse=0,
         allow=1),

    # What to do with unsupported source kinds.
    UnsupportedMemberBehavior=Enum(
         fail=0,
         skip=1),

    # Planned member mutation kind.
    PlannedAction=Enum(
         noop=0,
         clone=1,
         fetch=2,
         fast_forward=3,
         checkout=4,
         init_repo=5,
         add_manifest_member=6,
         write_manifest=7,
         write_lock=8,
         write_snapshot=9,
         write_tag=10,
         push=11,
         merge=12,
         rebase=13,
         reset=14,
         detach_member=15,
         attach_member=16),

    # Current member state compared to the lock.
    LockMatch=Enum(
         unknown=0,
         matches=1,
         differs=2,
         missing=3),

    # Phase of a member's in-flight Git transfer, for progress events.
    GitProgressPhase=Enum(
         enumerating=0,
         counting=1,
         compressing=2,
         receiving=3,
         resolving=4,
         checking_out=5,
         writing=6),

    # Status response mode.
    StatusMode=Enum(
         summary=0,
         combined=1),

    # Path rendering mode for file status entries.
    StatusPathStyle=Enum(
         member_relative=0,
         workspace_relative=1),

    # Operation event kind.
    EventKind=Enum(
         operation_started=0,
         member_started=1,
         member_progress=2,
         member_finished=3,
         artifact_written=4,
         operation_finished=5,
         reset=6,
         operation_state_changed=7),

    # Event severity.
    Severity=Enum(
         debug=0,
         info=1,
         warn=2,
         error=3),

    # Stable protocol error codes.
    GwzErrorCode=Enum(
         ok=0,
         invalid_request=1,
         workspace_not_found=2,
         workspace_already_exists=3,
         nested_workspace=4,
         manifest_not_found=5,
         manifest_invalid=6,
         schema_unsupported=7,
         member_not_found=8,
         member_inactive=9,
         path_escape=10,
         path_collision=11,
         path_reserved=12,
         unsupported_source_kind=13,
         unsupported_operation=14,
         dirty_member=15,
         diverged_member=16,
         missing_remote=17,
         snapshot_not_found=18,
         lock_not_found=19,
         tag_not_found=20,
         tag_invalid=21,
         remote_rejected=22,
         git_command_failed=23,
         external_tool_missing=24,
         operation_not_found=25,
         attribution_denied=26,
         permission_denied=27,
         io_error=28,
         internal_error=29,
         branch_detached_head=30,
         branch_unborn_head=31,
         branch_mixed=32,
         stash_not_found=33,
         stash_incomplete=34,
         stash_conflict=35,
         source_identity_mismatch=36,
         deprecated_operation=37,
         merge_validation_failed=38,
         merge_id_mismatch=39,
         merge_drift=40,
         open_operation=41,
         merge_recovery_required=42,
         merge_phase_unsupported=43,
         root_merge_not_yet_supported=44,
         merge_record_unreadable=45),

    # ---- diff enums -------------------------------------------------------
    # Which two sides libgit2 compares, resolved per target repo from operands
    # and the cached/merge_base request flags.
    DiffComparisonKind=Enum(
         # git diff: index -> worktree (diff_index_to_workdir).
         worktree_vs_index=0,
         # git diff --cached [<commit>]: tree -> index (diff_tree_to_index).
         index_vs_tree=1,
         # git diff <commit>: tree -> worktree, index data for staged deletes
         # (diff_tree_to_workdir_with_index).
         worktree_vs_tree=2,
         # git diff <a> <b> / <a>..<b> / <a>...<b>: tree -> tree
         # (diff_tree_to_tree).
         tree_vs_tree=3),

    # Requested rendering of the diff. Metadata-only forms are answered from the
    # manifest with no diff.output read (name_only..summary, no_patch); byte
    # forms are read from diff.output. Histogram is deliberately absent (the
    # git2 0.21 wrapper has no setter); requesting it is unsupported_operation.
    DiffOutputFormat=Enum(
         patch=0,
         raw=1,
         name_only=2,
         name_status=3,
         stat=4,
         numstat=5,
         shortstat=6,
         summary=7,
         patch_with_raw=8,
         patch_with_stat=9,
         # Manifest/summary only; used by --quiet and JSON metadata modes.
         no_patch=10),

    # How much manifest work core does. full builds the whole file list + stats;
    # any_difference backs --quiet: stop at the first delta, skip similarity,
    # omit files, return only enough summary for the exit-code decision.
    DiffManifestMode=Enum(
         full=0,
         any_difference=1),

    # Diff algorithm. NO histogram in v0 (unsupported by the wrapper). Requesting
    # an unsupported algorithm is a typed unsupported_operation error, not a
    # silent downgrade.
    DiffAlgorithm=Enum(
         default=0,
         myers=1,
         minimal=2,
         patience=3),

    # Whitespace handling, mapped to libgit2 ignore_whitespace* /
    # ignore_blank_lines.
    DiffWhitespaceMode=Enum(
         default=0,
         ignore_all=1,
         ignore_change=2,
         ignore_eol=3,
         ignore_blank_lines=4),

    # Per-file change classification. copied is reserved for a later copy
    # project; v0 never emits it (find_copies=true is rejected) but the value is
    # defined so the enum need not change when copy support lands.
    DiffStatus=Enum(
         added=0,
         modified=1,
         deleted=2,
         renamed=3,
         copied=4,
         type_changed=5,
         unmerged=6),

    # Byte encoding advertised on the output log. utf8 for text-only outputs;
    # bytes for patch/binary. JSON/JSONL transports base64-expand BYTES, which is
    # a transport concern, not a schema field.
    DiffChunkEncoding=Enum(
         utf8=0,
         bytes=1),

    # The DiffOutputRecord discriminator. patch_bytes carries data; the boundary
    # kinds (file_started/file_finished) are ALWAYS emitted, in every output
    # format, so machine consumers can frame per-file output without parsing patch
    # text; stale_file marks a worktree race; diagnostic carries a non-fatal
    # message.
    DiffOutputRecordKind=Enum(
         patch_bytes=0,
         file_started=1,
         file_finished=2,
         stale_file=3,
         diagnostic=4),

    # Why a candidate target was excluded before diff execution (snapshot
    # narrowing). Reported in DiffManifestResponse.excluded_targets so a member
    # added after a snapshot was captured is explained rather than silently
    # dropped.
    DiffTargetExclusionReason=Enum(
         # Snapshot operand does not contain this member (commonly: member added
         # after the snapshot was captured).
         snapshot_missing=0,
         # Snapshot contains this member but records no Git commit for it.
         snapshot_missing_commit=1,
         # v0 snapshots do not record a workspace-root commit.
         root_not_in_snapshot=2),


    # ---- common request/response values ----------------------------------
    # Workspace selector used by existing-workspace operations.
    WorkspaceRef=Msg(
        # Explicit workspace root; if absent, handlers may discover upward.
        root=F(1, STR, optional=True),
        # Optional guard that must match the manifest workspace id.
        workspace_id=F(2, STR, optional=True)),

    # Logical principal that requested or drove an operation.
    OperationActor=Msg(
        # Stable actor URI or id, not necessarily a Git identity.
        actor_id=F(1, STR),
        # Human-readable display label for logs and UIs.
        display_name=F(2, STR, optional=True),
        email=F(3, STR, optional=True),
        # Authority that asserted this actor, such as a local agent or SSO realm.
        authority=F(4, STR, optional=True)),

    # Git author/committer identity to put on Git objects when applicable.
    GitObjectIdentity=Msg(
        name=F(1, STR),
        email=F(2, STR),
        # Unix epoch timestamp in milliseconds.
        time_ms=F(3, INT, optional=True),
        # Offset from UTC in minutes at time_ms.
        timezone_offset_minutes=F(4, INT, optional=True)),

    # Attribution supplied by a driver; separates actor from Git identity.
    OperationAttribution=Msg(
        actor=F(1, Ref.OperationActor, optional=True),
        # Requested Git author identity.
        git_author=F(2, Ref.GitObjectIdentity, optional=True),
        # Requested Git committer identity.
        git_committer=F(3, Ref.GitObjectIdentity, optional=True),
        # Driver-local credential handle; never a secret value.
        credential_ref=F(4, STR, optional=True)),

    # Member selector applied before an operation plans or executes.
    Selection=Msg(
        # Select all active members; cannot be combined with filters.
        all=F(1, BOOL, optional=True),
        # Select members by manifest id.
        member_ids=F(2, List(STR)),
        # Select members by workspace-relative member path.
        paths=F(3, List(STR)),
        # Select workspace targets by selector token. Examples: @default,
        # @all, @root, member ids, member paths, and future declared @sets.
        targets=F(4, List(STR)),
        # Exclude workspace targets by selector token after include expansion.
        exclude_targets=F(5, List(STR))),

    # Driver-selected operation policy. v0 handlers may support only a subset.
    OperationPolicy=Msg(
        # Whether a selected-member failure aborts the whole operation.
        partial=F(1, Ref.PartialBehavior, optional=True),
        # Whether destructive behavior such as reset is allowed.
        destructive=F(2, Ref.DestructiveBehavior, optional=True),
        # Requested sync strategy for head materialization.
        sync=F(3, Ref.SyncBehavior, optional=True),
        # How to treat members whose source kind cannot execute the action.
        unsupported_member=F(4, Ref.UnsupportedMemberBehavior, optional=True),
        # Preferred remote name for fetch/push when the request omits one.
        remote=F(5, STR, optional=True),
        # Driver-requested maximum member concurrency.
        concurrency=F(6, INT, optional=True),
        # Minimum milliseconds between member_progress events per member.
        # Coalesces high-frequency Git transfer updates at emit time; absent or
        # 0 means no limit (emit every update).
        progress_min_interval_ms=F(7, INT, optional=True),
        # Maximum concurrent network operations per remote host. Protects each
        # host from too many simultaneous connections; members whose host cannot
        # be parsed are bounded only by `concurrency`.
        max_connections_per_host=F(8, INT, optional=True)),

    # Common operation metadata supplied by every request.
    RequestMeta=Msg(
        # Caller-owned correlation id; echoed in every response/event.
        request_id=F(1, STR),
        # Protocol version expected by the caller.
        schema_version=F(2, STR),
        workspace=F(3, Ref.WorkspaceRef, optional=True),
        selection=F(4, Ref.Selection, optional=True),
        policy=F(5, Ref.OperationPolicy, optional=True),
        # Plan without mutation when supported by the handler.
        dry_run=F(6, BOOL, optional=True),
        attribution=F(7, Ref.OperationAttribution, optional=True)),

    # Common operation metadata returned by responses.
    ResponseMeta=Msg(
        request_id=F(1, STR),
        schema_version=F(2, STR),
        # Action inferred from the request type.
        action=F(3, Ref.ActionKind),
        # Whole-operation result across all selected members.
        aggregate_status=F(4, Ref.AggregateStatus),
        # Runtime operation id used for events and final result lookup.
        operation_id=F(5, STR, optional=True),
        # Human-readable summary, not a machine contract.
        message=F(6, STR, optional=True),
        attribution=F(7, Ref.OperationAttribution, optional=True)),

    # Typed error with optional member context.
    GwzError=Msg(
        code=F(1, Ref.GwzErrorCode),
        # Human-readable diagnostic.
        message=F(2, STR),
        member_id=F(3, STR, optional=True),
        member_path=F(4, STR, optional=True),
        # Optional implementation detail for logs/debugging.
        detail=F(5, STR, optional=True),
        # Concrete target kind for member_id/member_path when present.
        target_kind=F(6, Ref.TargetKind, optional=True)),

    # ---- model projections ------------------------------------------------
    # Remote declaration recorded for a member.
    RemoteSpec=Msg(
        # Git remote name, usually origin.
        name=F(1, STR),
        url=F(2, STR),
        # Whether this remote is eligible for fetch operations.
        fetch=F(3, BOOL, optional=True),
        # Whether this remote is eligible for push operations.
        push=F(4, BOOL, optional=True)),

    # Desired member target stored in manifest-like data.
    DesiredRef=Msg(
        branch=F(1, STR, optional=True),
        commit=F(2, STR, optional=True),
        # Git tag name; distinct from GWZ workspace tags.
        git_tag=F(3, STR, optional=True),
        # Member has no authoritative remote target.
        local_only=F(4, BOOL, optional=True)),

    # Source URL used when initializing a workspace from existing repositories.
    SourceUrl=Msg(
        url=F(1, STR),
        # Optional target member path; derived from the URL when absent.
        path=F(2, STR, optional=True),
        # Remote name to record for this source; defaults by driver/handler.
        remote_name=F(3, STR, optional=True),
        # Requested branch for clone/materialization when supported.
        branch=F(4, STR, optional=True)),

    # Manifest member projection.
    MemberSpec=Msg(
        member_id=F(1, STR),
        # Workspace-relative member root.
        path=F(2, STR),
        # Source catalog id or local source id.
        source_id=F(3, STR),
        source_kind=F(4, Ref.SourceKind),
        # Inactive members remain recorded but are skipped by default selection.
        active=F(5, BOOL),
        desired=F(6, Ref.DesiredRef, optional=True),
        remotes=F(7, List(Ref.RemoteSpec))),

    # Explicit workspace materialization target.
    MaterializeTarget=Msg(
        kind=F(1, Ref.MaterializeTargetKind),
        # Snapshot/tag/branch name, depending on kind.
        name=F(2, STR, optional=True),
        # Exact Git commit target when kind=commit.
        commit=F(3, STR, optional=True)),

    # Source branch semantics for snapshot capture. When omitted, handlers use
    # the existing observed current-worktree source.
    SnapshotSource=Msg(
        kind=F(1, Ref.SnapshotSourceKind),
        # Branch name when kind=branch.
        branch=F(2, STR, optional=True)),

    # Resolved state captured in the lock, snapshots, tags, and responses.
    ResolvedMemberState=Msg(
        member_id=F(1, STR),
        path=F(2, STR),
        source_id=F(3, STR),
        source_kind=F(4, Ref.SourceKind),
        # Current resolved commit when the member has one.
        commit=F(5, STR, optional=True),
        # Current branch name when not detached.
        branch=F(6, STR, optional=True),
        detached=F(7, BOOL, optional=True),
        # Upstream ref used for pull/head comparisons.
        upstream=F(8, STR, optional=True),
        # True when index, worktree, or untracked state is not clean.
        dirty=F(9, BOOL, optional=True),
        # False when the member is declared but absent locally.
        materialized=F(10, BOOL),
        remotes=F(11, List(Ref.RemoteSpec))),

    # Per-member Git summary used in status responses.
    GitStatus=Msg(
        member_id=F(1, STR),
        branch=F(2, STR, optional=True),
        detached=F(3, BOOL),
        # Current HEAD commit.
        head=F(4, STR, optional=True),
        # Configured upstream ref, if any.
        upstream=F(5, STR, optional=True),
        # Commits local HEAD is ahead of upstream.
        ahead=F(6, INT, optional=True),
        # Commits local HEAD is behind upstream.
        behind=F(7, INT, optional=True),
        # Count of staged file changes.
        staged=F(8, INT),
        # Count of unstaged file changes.
        unstaged=F(9, INT),
        # Count of untracked files.
        untracked=F(10, INT),
        dirty=F(11, BOOL)),

    # File-level Git porcelain projection with member context.
    GitFileChange=Msg(
        member_id=F(1, STR),
        member_path=F(2, STR),
        # Path relative to the member repository root.
        repo_path=F(3, STR),
        # Path relative to the workspace root.
        workspace_path=F(4, STR),
        # Index status character from porcelain-style status.
        index_status=F(5, STR),
        # Worktree status character from porcelain-style status.
        worktree_status=F(6, STR),
        # Rename/copy source path relative to member root.
        original_repo_path=F(7, STR, optional=True)),

    # Git transfer counters for an in-flight member, surfaced in
    # member_progress events. Counts come from libgit2 progress callbacks;
    # totals may be absent until the remote reports them.
    GitTransferProgress=Msg(
        phase=F(1, Ref.GitProgressPhase),
        # Objects received so far (clone/fetch) or written (push).
        received_objects=F(2, INT, optional=True),
        # Total objects in the transfer, when known.
        total_objects=F(3, INT, optional=True),
        # Bytes received/written so far.
        received_bytes=F(4, INT, optional=True),
        # Deltas resolved so far.
        indexed_deltas=F(5, INT, optional=True),
        # Total deltas to resolve, when known.
        total_deltas=F(6, INT, optional=True)),

    # Root workspace Git summary used in status responses.
    WorkspaceRootGitStatus=Msg(
        branch=F(1, STR, optional=True),
        detached=F(2, BOOL),
        # Current root HEAD commit.
        head=F(3, STR, optional=True),
        # Count of staged file changes in the root repository.
        staged=F(4, INT),
        # Count of unstaged file changes in the root repository.
        unstaged=F(5, INT),
        # Count of untracked files in the root repository.
        untracked=F(6, INT),
        dirty=F(7, BOOL),
        # True when the root repo has no commits yet.
        unborn=F(8, BOOL)),

    # File-level Git porcelain projection for the workspace root repository.
    WorkspaceRootFileChange=Msg(
        # Path relative to the workspace root repository.
        repo_path=F(1, STR),
        # Path relative to the workspace root.
        workspace_path=F(2, STR),
        # Index status character from porcelain-style status.
        index_status=F(3, STR),
        # Worktree status character from porcelain-style status.
        worktree_status=F(4, STR),
        # Rename/copy source path relative to workspace root.
        original_repo_path=F(5, STR, optional=True)),

    # Branch/head state for one member in combined status output.
    GitMemberBranchStatus=Msg(
        member_id=F(1, STR),
        member_path=F(2, STR),
        # Normalized label used for grouping branch state.
        label=F(3, STR),
        branch=F(4, STR, optional=True),
        detached=F(5, BOOL),
        # True when the repo has no commits yet.
        unborn=F(6, BOOL),
        head=F(7, STR, optional=True),
        upstream=F(8, STR, optional=True),
        ahead=F(9, INT, optional=True),
        behind=F(10, INT, optional=True)),

    # Members sharing the same branch/head label.
    GitBranchGroup=Msg(
        label=F(1, STR),
        member_ids=F(2, List(STR)),
        member_paths=F(3, List(STR))),

    # Branch/head group that differs from the majority label.
    GitBranchDifference=Msg(
        label=F(1, STR),
        majority_label=F(2, STR, optional=True),
        member_ids=F(3, List(STR)),
        member_paths=F(4, List(STR)),
        message=F(5, STR, optional=True)),

    # Combined status projection across selected members.
    WorkspaceGitStatus=Msg(
        clean=F(1, BOOL),
        file_changes=F(2, List(Ref.GitFileChange)),
        branches=F(3, List(Ref.GitMemberBranchStatus)),
        branch_groups=F(4, List(Ref.GitBranchGroup)),
        branch_differences=F(5, List(Ref.GitBranchDifference)),
        root_status=F(6, Ref.WorkspaceRootGitStatus, optional=True),
        root_file_changes=F(7, List(Ref.WorkspaceRootFileChange))),

    # Dirty-state summary captured before a coordinated stash push.
    StashDirtySummary=Msg(
        staged=F(1, BOOL),
        unstaged=F(2, BOOL),
        untracked=F(3, BOOL),
        ignored=F(4, BOOL)),

    # Push lifecycle error detail persisted with a stash member record.
    StashErrorDetail=Msg(
        code=F(1, STR),
        message=F(2, STR)),

    # Warning associated with a stash bundle or member.
    StashWarning=Msg(
        code=F(1, STR),
        message=F(2, STR),
        member_id=F(3, STR, optional=True)),

    # Drift found while reconciling bundle metadata with native stash payloads.
    StashDrift=Msg(
        code=F(1, STR),
        message=F(2, STR),
        member_id=F(3, STR)),

    # One selected member's participation, native push lifecycle, and restore state.
    StashBundleMember=Msg(
        member_id=F(1, STR),
        path=F(2, STR),
        participation=F(3, Ref.StashParticipation),
        push_lifecycle=F(4, Ref.StashPushLifecycle),
        restore_state=F(5, Ref.StashRestoreState),
        branch_before=F(6, STR, optional=True),
        head_before=F(7, STR, optional=True),
        full_stash_message=F(8, STR),
        dirty_summary=F(9, Ref.StashDirtySummary),
        native_stash_object_id=F(10, STR, optional=True),
        native_stash_display_ref=F(11, STR, optional=True),
        error=F(12, Ref.StashErrorDetail, optional=True)),

    # Durable coordinated stash bundle projection.
    StashBundle=Msg(
        schema=F(1, STR),
        workspace_id=F(2, STR),
        stash_id=F(3, STR),
        created_at=F(4, STR),
        message_suffix=F(5, STR),
        include_untracked=F(6, BOOL),
        include_ignored=F(7, BOOL),
        members=F(8, List(Ref.StashBundleMember)),
        warnings=F(9, List(Ref.StashWarning)),
        drift=F(10, List(Ref.StashDrift)),
        selected_members=F(11, List(STR))),

    # Branch state and action result for one selected repository.
    BranchRepoSummary=Msg(
        member_id=F(1, STR),
        member_path=F(2, STR),
        source_kind=F(3, Ref.SourceKind),
        result=F(4, Ref.BranchActionResult),
        branch=F(5, STR, optional=True),
        current_branch=F(6, STR, optional=True),
        detached=F(7, BOOL),
        unborn=F(8, BOOL),
        head=F(9, STR, optional=True),
        upstream=F(10, STR, optional=True),
        ahead=F(11, INT, optional=True),
        behind=F(12, INT, optional=True),
        # Merge source ref requested for this repository.
        source_ref=F(13, STR, optional=True),
        # Attached target branch into which the source was merged.
        target_branch=F(14, STR, optional=True),
        # Resulting HEAD commit after a clean merge or fast-forward.
        resulting_commit=F(15, STR, optional=True),
        # Conflict paths relative to the member repository root.
        conflict_paths=F(16, List(STR))),

    # Counts by durable participant lifecycle state.
    MergeParticipantCounts=Msg(
        total=F(1, INT),
        planned=F(2, INT),
        up_to_date=F(3, INT),
        fast_forwarded=F(4, INT),
        merged=F(5, INT),
        conflicted=F(6, INT),
        failed=F(7, INT),
        unattempted=F(8, INT),
        continued=F(9, INT),
        aborted=F(10, INT),
        rolled_back=F(11, INT)),

    # Structured live-state mismatch for one frozen participant.
    MergeParticipantDrift=Msg(
        kind=F(1, Ref.MergeParticipantDriftKind),
        message=F(2, STR),
        expected_branch=F(3, STR, optional=True),
        live_branch=F(4, STR, optional=True),
        expected_head=F(5, STR, optional=True),
        live_head=F(6, STR, optional=True),
        expected_merge_head=F(7, STR, optional=True),
        live_merge_head=F(8, STR, optional=True)),

    # Structured workspace/record mismatch for a coordinated merge.
    MergeOperationDrift=Msg(
        kind=F(1, Ref.MergeOperationDriftKind),
        message=F(2, STR)),

    # Durable evidence created by an explicit preserve-abort attempt.
    MergePreservation=Msg(
        target_id=F(1, STR),
        path=F(2, STR),
        backup_ref=F(3, STR, optional=True),
        backup_commit=F(4, STR, optional=True),
        stash_id=F(5, STR, optional=True),
        stash_object_id=F(6, STR, optional=True)),

    # Status projection for an action journaled before a Git mutation whose
    # durable participant outcome has not yet been published.
    MergePendingActionSummary=Msg(
        kind=F(1, Ref.MergePendingActionKind),
        state=F(2, Ref.MergePendingActionState),
        message=F(3, STR, optional=True)),

    # Plan, result, and live recovery projection for one merge participant.
    MergeRepoSummary=Msg(
        target_id=F(1, STR),
        target_kind=F(2, Ref.TargetKind),
        path=F(3, STR),
        source_ref=F(4, STR),
        source_commit=F(5, STR),
        target_branch=F(6, STR),
        before_commit=F(7, STR),
        resulting_commit=F(8, STR, optional=True),
        live_commit=F(9, STR, optional=True),
        state=F(10, Ref.MergeParticipantState),
        predicted=F(11, Ref.MergeAnalysisKind, optional=True),
        # False means a true-merge prediction did not simulate content conflicts.
        prediction_complete=F(12, BOOL, optional=True),
        conflict_paths=F(13, List(STR)),
        continue_eligible=F(14, BOOL, optional=True),
        abort_eligible=F(15, BOOL, optional=True),
        drift=F(16, List(Ref.MergeParticipantDrift)),
        error=F(17, Ref.GwzError, optional=True),
        pending_action=F(18, Ref.MergePendingActionSummary, optional=True)),

    # Planned member mutation returned by dry-run or accepted responses.
    PlannedChange=Msg(
        action=F(1, Ref.PlannedAction),
        # Source ref before the planned change, if known.
        from_ref=F(2, STR, optional=True),
        # Target ref after the planned change, if known.
        to_ref=F(3, STR, optional=True),
        message=F(4, STR, optional=True)),

    # Per-member result or plan in an operation response.
    MemberResponse=Msg(
        member_id=F(1, STR),
        member_path=F(2, STR),
        source_kind=F(3, Ref.SourceKind),
        status=F(4, Ref.MemberStatus),
        # Member-scoped error when this member failed or was rejected.
        error=F(5, Ref.GwzError, optional=True),
        # Planned mutation for dry-run or accepted responses.
        planned=F(6, Ref.PlannedChange, optional=True),
        # Resolved member state after execution or planning.
        state=F(7, Ref.ResolvedMemberState, optional=True),
        git_status=F(8, Ref.GitStatus, optional=True),
        # Whether the current member state matches the lock.
        lock_match=F(9, Ref.LockMatch, optional=True),
        # Concrete target kind for this response.
        target_kind=F(10, Ref.TargetKind, optional=True)),

    # Standard response payload for request/response operations.
    ResponseEnvelope=Msg(
        meta=F(1, Ref.ResponseMeta),
        members=F(2, List(Ref.MemberResponse)),
        errors=F(3, List(Ref.GwzError))),

    # Durable/progressive operation event.
    OperationEvent=Msg(
        operation_id=F(1, STR),
        request_id=F(2, STR),
        # Monotonic sequence number within an operation.
        sequence=F(3, INT),
        # Unix epoch timestamp in milliseconds.
        timestamp_ms=F(4, INT),
        kind=F(5, Ref.EventKind),
        severity=F(6, Ref.Severity),
        member_id=F(7, STR, optional=True),
        member_path=F(8, STR, optional=True),
        message=F(9, STR, optional=True),
        # Optional member snapshot associated with this event.
        member=F(10, Ref.MemberResponse, optional=True),
        error=F(11, Ref.GwzError, optional=True),
        attribution=F(12, Ref.OperationAttribution, optional=True),
        # Git transfer counters for member_progress events.
        progress=F(13, Ref.GitTransferProgress, optional=True),
        # Concrete target kind for member_id/member_path when present.
        target_kind=F(14, Ref.TargetKind, optional=True),
        # Structured state for operation_state_changed events.
        merge_state=F(15, Ref.MergeOperationState, optional=True),
        # Merge-specific durable participant outcome for member_finished.
        merge_member=F(16, Ref.MergeRepoSummary, optional=True),
        # Workspace-relative durable artifact path for artifact_written.
        artifact_path=F(17, STR, optional=True)),

    # Final operation record returned by operation.result.
    OperationResult=Msg(
        operation_id=F(1, STR),
        request_id=F(2, STR),
        action=F(3, Ref.ActionKind),
        aggregate_status=F(4, Ref.AggregateStatus),
        # Unix epoch milliseconds when execution started.
        started_at_ms=F(5, INT),
        # Unix epoch milliseconds when execution finished.
        finished_at_ms=F(6, INT),
        members=F(7, List(Ref.MemberResponse)),
        errors=F(8, List(Ref.GwzError)),
        attribution=F(9, Ref.OperationAttribution, optional=True)),

    # ---- action requests --------------------------------------------------
    # Create an empty workspace at workspace_root.
    CreateWorkspaceRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Target directory that will receive workspace/gwz.yml and lock.
        workspace_root=F(2, STR),
        # Optional workspace id; defaults when absent.
        workspace_id=F(3, STR, optional=True)),

    # Create or plan a workspace from one or more source URLs.
    InitFromSourcesRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Target workspace root; empty string means use the handler start path.
        workspace_root=F(2, STR),
        sources=F(3, List(Ref.SourceUrl)),
        # Initial materialization target; v0 requires head semantics.
        target=F(4, Ref.MaterializeTarget, optional=True),
        # Optional workspace id for create or existing-workspace verification.
        workspace_id=F(5, STR, optional=True)),

    # Clone a workspace root Git repository and materialize locked members.
    CloneWorkspaceRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Git URL of the workspace root repository.
        url=F(2, STR),
        # Target directory for the cloned workspace.
        target=F(3, STR)),

    # Add an already-cloned Git repository to the workspace manifest and lock.
    AddExistingRepoRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Filesystem path to the existing Git repository.
        repository_path=F(2, STR),
        # Workspace-relative member path; derived when absent.
        member_path=F(3, STR, optional=True),
        # Optional explicit member id; derived from member_path when absent.
        member_id=F(4, STR, optional=True),
        # Optional explicit source id; derived from member_path when absent.
        source_id=F(5, STR, optional=True)),

    # Initialize a new local Git repository and add it as a workspace member.
    CreateRepoRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Workspace-relative member path to create.
        member_path=F(2, STR),
        # Requested initial branch; v0 supports main only.
        initial_branch=F(3, STR, optional=True),
        member_id=F(4, STR, optional=True),
        source_id=F(5, STR, optional=True)),

    # Refresh configured member metadata from local Git config.
    RepoSyncRequest=Msg(
        meta=F(1, Ref.RequestMeta)),

    # Clone and register one repository as a workspace member.
    CloneRepoMemberRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Canonical clone-source shape: URL plus optional path/remote/branch.
        source=F(2, Ref.SourceUrl),
        member_id=F(3, STR, optional=True),
        source_id=F(4, STR, optional=True)),

    # Detach targeting is carried by RequestMeta.selection.
    DetachRepoMemberRequest=Msg(
        meta=F(1, Ref.RequestMeta)),

    # Attach targeting is carried by RequestMeta.selection.
    AttachRepoMemberRequest=Msg(
        meta=F(1, Ref.RequestMeta)),

    # Move selected members to an explicit target.
    MaterializeRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        target=F(2, Ref.MaterializeTarget)),

    # Report workspace/member Git and lock status.
    StatusRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Summary or combined output mode.
        mode=F(2, Ref.StatusMode, optional=True),
        # Include file-level Git changes when supported.
        include_file_changes=F(3, BOOL, optional=True),
        # Include branch grouping/difference projection when supported.
        include_branch_summary=F(4, BOOL, optional=True),
        # How file paths should be rendered in status projections.
        path_style=F(5, Ref.StatusPathStyle, optional=True)),

    # List the workspace's members (read-only; manifest + lock, no git).
    LsRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Include configured-but-unmaterialized members.
        include_unmaterialized=F(2, BOOL, optional=True)),

    # One member in an `ls` listing. Reused by ExecRequest (forall).
    MemberEntry=Msg(
        # Member id (e.g. mem_app).
        id=F(1, STR),
        # Workspace-relative path (e.g. repos/app).
        path=F(2, STR),
        # Absolute path on this host.
        abspath=F(3, STR),
        # Whether the member is cloned/materialized on disk.
        materialized=F(4, BOOL),
        # Concrete target kind for this list entry.
        target_kind=F(5, Ref.TargetKind, optional=True)),

    LsResponse=Msg(
        response=F(1, Ref.ResponseEnvelope),
        members=F(2, List(Ref.MemberEntry), optional=True)),

    # ---- gwz forall (CLI-local; gwz-core MUST NOT handle these) -----------------
    # `gwz forall` runs a command in each selected member. These messages live here only because
    # the taut module system (TautModules.md) isn't built yet — relocate to a gwz-cli-owned IR when
    # it lands. There is NO gwz-core handler and no `service` method: gwz-core never executes
    # commands; the gwz-cli executor dispatches them locally.
    #
    # Per-member process outcome. `exit_code` is absent for signal-killed processes (Unix, → signal);
    # `spawn_error` records a failure before a process existed (missing binary, bad cwd).
    ExecResult=Msg(
        id=F(1, STR),
        path=F(2, STR),
        exit_code=F(3, INT, optional=True),
        signal=F(4, INT, optional=True),
        spawn_error=F(5, STR, optional=True)),

    # Run a command across the given members.
    ExecRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        mode=F(2, Ref.ExecMode),
        # argv (argv mode), or a single shell string (shell mode).
        command=F(3, List(STR)),
        members=F(4, List(Ref.MemberEntry)),
        # Continue past a failing member (the `--partial` global); default stops.
        continue_on_fail=F(5, BOOL, optional=True)),

    ExecResponse=Msg(
        response=F(1, Ref.ResponseEnvelope),
        results=F(2, List(Ref.ExecResult), optional=True)),

    # Write a named snapshot for the selected members.
    SnapshotRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        snapshot_id=F(2, STR),
        source=F(3, Ref.SnapshotSource, optional=True)),

    # List named snapshots for the workspace.
    ListSnapshotsRequest=Msg(
        meta=F(1, Ref.RequestMeta)),

    # Manage git tags (`refs/tags/<name>`) across the selected members.
    TagRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        op=F(2, Ref.TagOp),
        # Tag name (omit for list / fetch-all).
        name=F(3, STR, optional=True),
        # Annotation message — annotated tag when set.
        message=F(4, STR, optional=True),
        # Create a signed tag.
        signed=F(5, BOOL, optional=True),
        # Remote override for fetch / push / list-remote.
        remote=F(6, STR, optional=True),
        # Operate on all tags (push --all, list remote).
        all=F(7, BOOL, optional=True)),

    # Capture live observed member state into the lock (no worktree mutation).
    CaptureRequest=Msg(
        meta=F(1, Ref.RequestMeta)),

    # Commit staged (or, with all, tracked-modified) changes across members + root.
    CommitRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Commit message applied to every committed repo.
        message=F(2, STR),
        # Stage tracked modifications first (git commit -a).
        all=F(3, BOOL, optional=True),
        # Create/persist GWZ commit marker metadata. Omitted means core default.
        commit_marker=F(4, BOOL, optional=True)),

    # Stage pathspecs across the owning member/root repos (multi-repo git add).
    StageRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Absolute working directory the pathspecs are resolved against (git cwd).
        cwd=F(2, STR),
        # Pathspecs to stage; resolved cwd-relative, then routed to the owning repo.
        pathspecs=F(3, List(STR)),
        # Stage everything across every repo (git add -A), ignoring pathspecs.
        all=F(4, BOOL, optional=True)),

    # Fetch and fast-forward selected members to their upstream heads.
    PullHeadRequest=Msg(
        meta=F(1, Ref.RequestMeta)),

    # Materialize selected members to a named snapshot.
    PullSnapshotRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        snapshot_id=F(2, STR)),

    # Push selected members.
    PushRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Remote override for this request.
        remote=F(2, STR, optional=True),
        # Refspec override for this request.
        refspec=F(3, STR, optional=True)),

    # Coordinate native Git stash operations across selected members.
    StashRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        op=F(2, Ref.StashOp),
        # Stash id for apply/pop/drop; list may omit, push generates one.
        stash_id=F(3, STR, optional=True),
        # Message suffix for push.
        message=F(4, STR, optional=True),
        # Include untracked files during push.
        include_untracked=F(5, BOOL, optional=True),
        # Include ignored files during push.
        include_ignored=F(6, BOOL, optional=True),
        # Include expanded per-member bundle detail in list responses.
        expanded=F(7, BOOL, optional=True),
        # Attempt to reinstate the index during apply/pop; default is handler-defined.
        preserve_index=F(8, BOOL, optional=True)),

    # Manage local Git branches across selected members.
    BranchRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        op=F(2, Ref.BranchOp),
        # Branch name for create/delete; list may omit.
        name=F(3, STR, optional=True),
        # Start point for create, such as HEAD or refs/heads/main.
        start_ref=F(4, STR, optional=True),
        # Switch selected members to the branch after create.
        switch_after_create=F(5, BOOL, optional=True)),

    # Coordinate or inspect a durable workspace merge lifecycle.
    MergeRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        op=F(2, Ref.MergeOp),
        # Required only for start; independently resolved in every participant.
        source_ref=F(3, STR, optional=True),
        # Optional recovery/archived-record guard; forbidden for start.
        merge_id=F(4, STR, optional=True),
        mode=F(5, Ref.MergeMode, optional=True),
        # Reserved until the custom-message delivery phase.
        message=F(6, STR, optional=True),
        # Accepted only for abort; true is reserved until preserve-abort lands.
        preserve=F(7, BOOL, optional=True)),

    # ---- action responses -------------------------------------------------
    # Response wrapper for create_workspace.
    CreateWorkspaceResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for init_from_sources.
    InitFromSourcesResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for clone_workspace.
    CloneWorkspaceResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for add_existing_repo.
    AddExistingRepoResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for create_repo.
    CreateRepoResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),

    RepoSyncResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    CloneRepoMemberResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    DetachRepoMemberResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    AttachRepoMemberResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for materialize.
    MaterializeResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Status response with optional combined Git projection.
    StatusResponse=Msg(
        response=F(1, Ref.ResponseEnvelope),
        workspace_git_status=F(2, Ref.WorkspaceGitStatus, optional=True)),
    # Response wrapper for snapshot.
    SnapshotResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # A snapshot entry returned by list_snapshots.
    SnapshotInfo=Msg(
        # Snapshot name without the artifact path/extension.
        name=F(1, STR),
        created_at=F(2, STR),
        created_by=F(3, STR),
        # Number of member states captured in this snapshot.
        members=F(4, INT)),
    # Response wrapper for list_snapshots.
    ListSnapshotsResponse=Msg(
        response=F(1, Ref.ResponseEnvelope),
        snapshots=F(2, List(Ref.SnapshotInfo), optional=True)),
    # A tag entry returned by tag list operations.
    TagInfo=Msg(
        # Tag name without the refs/tags/ prefix.
        name=F(1, STR),
        # Number of member repos carrying this tag.
        members=F(2, INT)),
    # Response wrapper for tag — `tags` is populated for list operations.
    TagResponse=Msg(
        response=F(1, Ref.ResponseEnvelope),
        tags=F(2, List(Ref.TagInfo), optional=True)),
    # Response wrapper for capture.
    CaptureResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for commit.
    CommitResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for stage.
    StageResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for pull_head.
    PullHeadResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for pull_snapshot.
    PullSnapshotResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for push.
    PushResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for stash operations.
    StashResponse=Msg(
        response=F(1, Ref.ResponseEnvelope),
        bundles=F(2, List(Ref.StashBundle), optional=True)),
    # Response wrapper for branch operations.
    BranchResponse=Msg(
        response=F(1, Ref.ResponseEnvelope),
        repos=F(2, List(Ref.BranchRepoSummary), optional=True)),
    # Response wrapper for every merge lifecycle operation.
    MergeResponse=Msg(
        response=F(1, Ref.ResponseEnvelope),
        merge_id=F(2, STR, optional=True),
        state=F(3, Ref.MergeOperationState),
        # Derived convenience projection; state remains authoritative.
        open=F(4, BOOL),
        participant_counts=F(5, Ref.MergeParticipantCounts),
        repos=F(6, List(Ref.MergeRepoSummary)),
        operation_drift=F(7, List(Ref.MergeOperationDrift)),
        preservation=F(8, List(Ref.MergePreservation), optional=True),
        publication_step=F(9, Ref.MergePublicationStep, optional=True)),

    # ---- diff request messages --------------------------------------------
    # One resolved comparison for one target repo: kind + resolved endpoints.
    # left/right are the raw revision tokens as classified for THIS repo; the
    # resolved object ids live on DiffParsedTarget. merge_base covers both
    # --merge-base and the A...B form after per-repo operand resolution.
    DiffComparison=Msg(
        kind=F(1, Ref.DiffComparisonKind),
        # Left/old-side revision token, interpreted inside each target repo.
        left=F(2, STR, optional=True),
        # Right/new-side revision token, interpreted inside each target repo.
        right=F(3, STR, optional=True),
        # Use merge-base(left, HEAD-or-right) as the old side. Set for
        # --merge-base and for A...B after core lowers the range per repo.
        merge_base=F(4, BOOL, optional=True)),

    # git2 diff option knobs. All optional; absent = libgit2/GWZ default. These
    # affect which bytes core emits, so every client must agree on them (they are
    # protocol, not client-local). Pager/color/exit-code are deliberately NOT
    # here (core stays headless).
    DiffOptions=Msg(
        output_format=F(1, Ref.DiffOutputFormat, optional=True),
        context_lines=F(2, INT, optional=True),
        interhunk_lines=F(3, INT, optional=True),
        algorithm=F(4, Ref.DiffAlgorithm, optional=True),
        whitespace=F(5, Ref.DiffWhitespaceMode, optional=True),
        find_renames=F(6, BOOL, optional=True),
        # Deferred. v0 REJECTS this with unsupported_operation until a copy
        # project can reproduce copy source sets and render copy headers.
        find_copies=F(7, BOOL, optional=True),
        rename_threshold=F(8, INT, optional=True),
        rename_limit=F(9, INT, optional=True),
        binary=F(10, BOOL, optional=True),
        text=F(11, BOOL, optional=True),
        full_index=F(12, BOOL, optional=True),
        abbrev=F(13, INT, optional=True),
        reverse=F(14, BOOL, optional=True),
        # -z / NUL-terminated name output. Byte fidelity is a diff.output/log
        # concern; this flag only selects the format.
        null_terminated=F(15, BOOL, optional=True),
        src_prefix=F(16, STR, optional=True),
        dst_prefix=F(17, STR, optional=True),
        no_prefix=F(18, BOOL, optional=True),
        line_prefix=F(19, STR, optional=True),
        ignore_submodules=F(20, STR, optional=True),
        diff_filter=F(21, STR, optional=True),
        # full vs any_difference (--quiet). Absent = full.
        manifest_mode=F(22, Ref.DiffManifestMode, optional=True),
        # Opt-in echo of each changed file's manifest entry on the matching
        # DiffOutputRecord (DiffOutputRecord.entry). Absent/false = correlate by
        # scope+file_id only; true = core populates entry so a streaming consumer
        # need not hold the whole manifest. Default off to avoid duplicating
        # manifest bytes on the wire.
        echo_manifest_entries=F(23, BOOL, optional=True)),

    # The diff planning request. Parsed comparison flags are FIRST-CLASS fields
    # (cached, merge_base); raw ambiguous operands stay in operands for per-repo
    # core classification; explicit pathspecs after `--` stay separate.
    DiffRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        # Workspace-relative logical cwd: "", "gwz-core", "gwz-core/src".
        # Relative path operands resolve against this, NOT a client abspath.
        workspace_cwd=F(2, STR, optional=True),
        # Positional tokens before `--`. Core classifies rev-vs-path per target
        # repo. A token of the form +<snapshot_id> is a GWZ snapshot operand,
        # resolved by core, never passed to Git.
        operands=F(3, List(STR)),
        # Pathspecs after `--`; resolved relative to workspace_cwd. A leading
        # `+` here is a literal path, never a snapshot operand.
        explicit_pathspecs=F(4, List(STR)),
        options=F(5, Ref.DiffOptions, optional=True),
        # --cached / --staged. Selects index-vs-tree forms. First-class, not an
        # operand tunnel.
        cached=F(6, BOOL, optional=True),
        # --merge-base. First-class. The A...B syntax is still parsed from
        # operands and lowered to DiffComparison.merge_base per repo.
        merge_base=F(7, BOOL, optional=True)),

    # ---- diff manifest / summary / output messages ------------------------
    # Which repo an entry/target/record belongs to. root xor (member_id +
    # member_path). source_kind reuses the existing SourceKind enum.
    DiffRepoScope=Msg(
        # True for the workspace root repository.
        root=F(1, BOOL, optional=True),
        member_id=F(2, STR, optional=True),
        member_path=F(3, STR, optional=True),
        source_kind=F(4, Ref.SourceKind, optional=True)),

    # A candidate target intentionally excluded before diff execution, with the
    # reason and (for snapshot narrowing) the snapshot id that caused it.
    # Required so a member absent from a referenced snapshot is explained, not
    # dropped.
    DiffExcludedTarget=Msg(
        scope=F(1, Ref.DiffRepoScope),
        reason=F(2, Ref.DiffTargetExclusionReason),
        # Snapshot operand that caused the exclusion, without the leading `+`.
        snapshot_id=F(3, STR, optional=True),
        message=F(4, STR, optional=True)),

    # A scope-addressable, reusable per-repo classification of the request. The
    # output renderer reuses this by scope/target_id WITHOUT reclassifying raw
    # operands. Resolved oids are populated where a side has one; worktree sides
    # omit an oid. Snapshot-derived sides preserve their snapshot ids.
    DiffParsedTarget=Msg(
        # Stable within the manifest; scoped to exactly one root/member repo.
        target_id=F(1, STR),
        scope=F(2, Ref.DiffRepoScope),
        # Resolved by core per repository from operands, cached/merge_base, and
        # pathspecs.
        comparison=F(3, Ref.DiffComparison),
        # Repo-relative pathspecs after workspace routing (member prefix
        # stripped).
        pathspecs=F(4, List(STR)),
        # Resolved object ids where available. Worktree sides may omit an oid.
        left_oid=F(5, STR, optional=True),
        right_oid=F(6, STR, optional=True),
        merge_base_oid=F(7, STR, optional=True),
        # Present when a side came from a GWZ snapshot operand such as +start.
        left_snapshot_id=F(8, STR, optional=True),
        right_snapshot_id=F(9, STR, optional=True)),

    # One changed file, workspace-relative. file_id is opaque (never parsed as a
    # path); scope/status/old_path/new_path are the structured identity. Rename
    # entries carry BOTH paths + similarity and must not degrade to add/delete.
    DiffFileEntry=Msg(
        file_id=F(1, STR),
        scope=F(2, Ref.DiffRepoScope),
        status=F(3, Ref.DiffStatus),
        # Workspace-relative. new_path == old_path except for rename/copy.
        old_path=F(4, STR, optional=True),
        new_path=F(5, STR, optional=True),
        old_mode=F(6, INT, optional=True),
        new_mode=F(7, INT, optional=True),
        # 0..100 similarity for rename/copy entries.
        similarity=F(8, INT, optional=True),
        insertions=F(9, INT, optional=True),
        deletions=F(10, INT, optional=True),
        is_binary=F(11, BOOL, optional=True)),

    # Per-repo rollup. files_changed counts changes before GWZ root/member
    # filtering where libgit2 reports it cheaply; files_manifested counts entries
    # actually surfaced after pathspec/selection/root-exclusion filtering.
    DiffRepoSummary=Msg(
        scope=F(1, Ref.DiffRepoScope),
        has_differences=F(2, BOOL),
        files_changed=F(3, INT),
        insertions=F(4, INT),
        deletions=F(5, INT),
        files_manifested=F(6, INT)),

    # Workspace aggregate. has_differences drives the client --exit-code/--quiet
    # decision; it is a client contract, not the core aggregate_status.
    DiffSummary=Msg(
        has_differences=F(1, BOOL),
        repos_examined=F(2, INT),
        repos_with_differences=F(3, INT),
        files_changed=F(4, INT),
        insertions=F(5, INT),
        deletions=F(6, INT),
        repo_summaries=F(7, List(Ref.DiffRepoSummary))),

    # The opaque handle + advertised shape of the byte output log. log_id is the
    # taut-shape log handle: holding it is the authority to read diff.output.
    # format/encoding tell the client what the bytes are. NO cursor/close/max_*
    # fields — those are the shape_log LogReadRequest surface.
    DiffOutputLogRef=Msg(
        log_id=F(1, STR),
        format=F(2, Ref.DiffOutputFormat),
        encoding=F(3, Ref.DiffChunkEncoding, optional=True)),

    # The `diff` response. Metadata + manifest + scoped targets + excluded
    # targets + an optional output-log ref (omitted for no-byte modes). Reuses
    # the existing ResponseEnvelope (meta/members/errors).
    DiffManifestResponse=Msg(
        response=F(1, Ref.ResponseEnvelope),
        files=F(2, List(Ref.DiffFileEntry)),
        summary=F(3, Ref.DiffSummary, optional=True),
        # Scope-addressable per-repo operand classification resolved by core.
        targets=F(4, List(Ref.DiffParsedTarget)),
        # Omitted when the requested mode has no patch/byte output.
        output=F(5, Ref.DiffOutputLogRef, optional=True),
        # Candidates intentionally excluded before diff execution (snapshot
        # narrowing). Absent-from-snapshot members appear here.
        excluded_targets=F(6, List(Ref.DiffExcludedTarget))),

    # The append/out type of the diff.output log: the payload of the generic
    # taut-shape LogRecord. Patch bytes ride `data` (BYTES, NUL-safe).
    # Boundary/stale/diagnostic kinds let machine consumers frame per-file output
    # without parsing patch text. Correlation is scope+file_id(+entry); file_id
    # is NEVER parsed as a path. There is NO cursor/EOF/close field here — the
    # log engine owns all of that.
    DiffOutputRecord=Msg(
        kind=F(1, Ref.DiffOutputRecordKind),
        # Structured correlation for file-scoped records (file_started/
        # file_finished/patch_bytes/stale_file).
        scope=F(2, Ref.DiffRepoScope, optional=True),
        file_id=F(3, STR, optional=True),
        # Echo of the manifest entry so a streaming consumer need not hold the
        # whole manifest to interpret a record. Populated only when the request
        # sets DiffOptions.echo_manifest_entries; absent by default (correlate by
        # scope+file_id).
        entry=F(4, Ref.DiffFileEntry, optional=True),
        # patch_bytes payload. Exact bytes, including NULs and binary hunks.
        data=F(5, BYTES, optional=True),
        # True on a stale_file record: the planned entry could not be rendered
        # because the worktree changed before output materialization.
        stale=F(6, BOOL, optional=True),
        diagnostic=F(7, STR, optional=True)),
)

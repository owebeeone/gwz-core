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
        # Stream operation events by operation id.
        method("events.subscribe", role="out", shape="log",
               params=Params(operation_id=STR),
               out=Ref.OperationEvent),
        # Read the final operation result by operation id.
        method("operation.result", role="out",
               params=Params(operation_id=STR),
               out=Ref.OperationResult)),

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
         clone_workspace=19),

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
         reset=14),

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
         reset=6),

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
         stash_conflict=35),


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
        paths=F(3, List(STR))),

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
        detail=F(5, STR, optional=True)),

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
        lock_match=F(9, Ref.LockMatch, optional=True)),

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
        progress=F(13, Ref.GitTransferProgress, optional=True)),

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
        materialized=F(4, BOOL)),

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
        all=F(3, BOOL, optional=True)),

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
)

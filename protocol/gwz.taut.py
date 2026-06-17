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
        # Register an existing local Git repository.
        method("add_existing_repo", role="in",
               params=Params(request=Ref.AddExistingRepoRequest),
               out=Ref.AddExistingRepoResponse),
        # Create a new local member repository.
        method("create_repo", role="in",
               params=Params(request=Ref.CreateRepoRequest),
               out=Ref.CreateRepoResponse),
        # Move members to a lock/snapshot/tag/commit target.
        method("materialize", role="in",
               params=Params(request=Ref.MaterializeRequest),
               out=Ref.MaterializeResponse),
        # Read member Git and lock status.
        method("status", role="in",
               params=Params(request=Ref.StatusRequest),
               out=Ref.StatusResponse),
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
         capture=11),

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
         failed=5),

    # Per-member result status.
    MemberStatus=Enum(
         planned=0,
         ok=1,
         noop=2,
         skipped=3,
         rejected=4,
         failed=5),

    # Explicit materialization target kind.
    MaterializeTargetKind=Enum(
         lock=0,
         head=1,
         snapshot=2,
         tag=3,
         commit=4),

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
         push=11),

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
         internal_error=29),


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

    # Write a named snapshot for the selected members.
    SnapshotRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        snapshot_id=F(2, STR)),

    # Write a named GWZ tag for the selected members.
    TagRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        tag_name=F(2, STR)),

    # Capture live observed member state into the lock (no worktree mutation).
    CaptureRequest=Msg(
        meta=F(1, Ref.RequestMeta)),

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

    # ---- action responses -------------------------------------------------
    # Response wrapper for create_workspace.
    CreateWorkspaceResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for init_from_sources.
    InitFromSourcesResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for add_existing_repo.
    AddExistingRepoResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for create_repo.
    CreateRepoResponse=Msg(
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
    # Response wrapper for tag.
    TagResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    # Response wrapper for capture.
    CaptureResponse=Msg(
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
)

"""GWS v0 taut protocol schema."""

from taut.ir.dsl import BOOL, BYTES, INT, STR, Enum, F, List, Msg, Params, Ref, method, schema, service

SCHEMA = schema(
    # ---- service ----------------------------------------------------------
    service("GwsCore",
        method("create_workspace", role="in",
               params=Params(request=Ref.CreateWorkspaceRequest),
               out=Ref.CreateWorkspaceResponse),
        method("init_from_sources", role="in",
               params=Params(request=Ref.InitFromSourcesRequest),
               out=Ref.InitFromSourcesResponse),
        method("add_existing_repo", role="in",
               params=Params(request=Ref.AddExistingRepoRequest),
               out=Ref.AddExistingRepoResponse),
        method("create_repo", role="in",
               params=Params(request=Ref.CreateRepoRequest),
               out=Ref.CreateRepoResponse),
        method("materialize", role="in",
               params=Params(request=Ref.MaterializeRequest),
               out=Ref.MaterializeResponse),
        method("status", role="in",
               params=Params(request=Ref.StatusRequest),
               out=Ref.StatusResponse),
        method("snapshot", role="in",
               params=Params(request=Ref.SnapshotRequest),
               out=Ref.SnapshotResponse),
        method("tag", role="in",
               params=Params(request=Ref.TagRequest),
               out=Ref.TagResponse),
        method("pull_head", role="in",
               params=Params(request=Ref.PullHeadRequest),
               out=Ref.PullHeadResponse),
        method("pull_snapshot", role="in",
               params=Params(request=Ref.PullSnapshotRequest),
               out=Ref.PullSnapshotResponse),
        method("push", role="in",
               params=Params(request=Ref.PushRequest),
               out=Ref.PushResponse),
        method("events.subscribe", role="out", shape="log",
               params=Params(operation_id=STR),
               out=Ref.OperationEvent),
        method("operation.result", role="out",
               params=Params(operation_id=STR),
               out=Ref.OperationResult)),

    # ---- enums ------------------------------------------------------------
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
         push=10),

    SourceKind=Enum(
         git=0,
         archive=1,
         package=2,
         local=3,
         generated=4),

    AggregateStatus=Enum(
         accepted=0,
         ok=1,
         noop=2,
         rejected=3,
         partial=4,
         failed=5),

    MemberStatus=Enum(
         planned=0,
         ok=1,
         noop=2,
         skipped=3,
         rejected=4,
         failed=5),

    MaterializeTargetKind=Enum(
         lock=0,
         head=1,
         snapshot=2,
         tag=3,
         commit=4),

    SyncBehavior=Enum(
         fetch_only=0,
         ff_only=1,
         merge=2,
         rebase=3,
         reset=4,
         driver_selected=5),

    PartialBehavior=Enum(
         atomic=0,
         partial=1),

    DestructiveBehavior=Enum(
         refuse=0,
         allow=1),

    UnsupportedMemberBehavior=Enum(
         fail=0,
         skip=1),

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

    LockMatch=Enum(
         unknown=0,
         matches=1,
         differs=2,
         missing=3),

    StatusMode=Enum(
         summary=0,
         combined=1),

    StatusPathStyle=Enum(
         member_relative=0,
         workspace_relative=1),

    EventKind=Enum(
         operation_started=0,
         member_started=1,
         member_progress=2,
         member_finished=3,
         artifact_written=4,
         operation_finished=5,
         reset=6),

    Severity=Enum(
         debug=0,
         info=1,
         warn=2,
         error=3),

    GwsErrorCode=Enum(
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
    WorkspaceRef=Msg(
        root=F(1, STR, optional=True),
        workspace_id=F(2, STR, optional=True)),

    OperationActor=Msg(
        actor_id=F(1, STR),
        display_name=F(2, STR, optional=True),
        email=F(3, STR, optional=True),
        authority=F(4, STR, optional=True)),

    GitObjectIdentity=Msg(
        name=F(1, STR),
        email=F(2, STR),
        time_ms=F(3, INT, optional=True),
        timezone_offset_minutes=F(4, INT, optional=True)),

    OperationAttribution=Msg(
        actor=F(1, Ref.OperationActor, optional=True),
        git_author=F(2, Ref.GitObjectIdentity, optional=True),
        git_committer=F(3, Ref.GitObjectIdentity, optional=True),
        credential_ref=F(4, STR, optional=True)),

    Selection=Msg(
        all=F(1, BOOL, optional=True),
        member_ids=F(2, List(STR)),
        paths=F(3, List(STR))),

    OperationPolicy=Msg(
        partial=F(1, Ref.PartialBehavior, optional=True),
        destructive=F(2, Ref.DestructiveBehavior, optional=True),
        sync=F(3, Ref.SyncBehavior, optional=True),
        unsupported_member=F(4, Ref.UnsupportedMemberBehavior, optional=True),
        remote=F(5, STR, optional=True),
        concurrency=F(6, INT, optional=True)),

    RequestMeta=Msg(
        request_id=F(1, STR),
        schema_version=F(2, STR),
        workspace=F(3, Ref.WorkspaceRef, optional=True),
        selection=F(4, Ref.Selection, optional=True),
        policy=F(5, Ref.OperationPolicy, optional=True),
        dry_run=F(6, BOOL, optional=True),
        attribution=F(7, Ref.OperationAttribution, optional=True)),

    ResponseMeta=Msg(
        request_id=F(1, STR),
        schema_version=F(2, STR),
        action=F(3, Ref.ActionKind),
        aggregate_status=F(4, Ref.AggregateStatus),
        operation_id=F(5, STR, optional=True),
        message=F(6, STR, optional=True),
        attribution=F(7, Ref.OperationAttribution, optional=True)),

    GwsError=Msg(
        code=F(1, Ref.GwsErrorCode),
        message=F(2, STR),
        member_id=F(3, STR, optional=True),
        member_path=F(4, STR, optional=True),
        detail=F(5, STR, optional=True)),

    # ---- model projections ------------------------------------------------
    RemoteSpec=Msg(
        name=F(1, STR),
        url=F(2, STR),
        fetch=F(3, BOOL, optional=True),
        push=F(4, BOOL, optional=True)),

    DesiredRef=Msg(
        branch=F(1, STR, optional=True),
        commit=F(2, STR, optional=True),
        git_tag=F(3, STR, optional=True),
        local_only=F(4, BOOL, optional=True)),

    SourceUrl=Msg(
        url=F(1, STR),
        path=F(2, STR, optional=True),
        remote_name=F(3, STR, optional=True),
        branch=F(4, STR, optional=True)),

    MemberSpec=Msg(
        member_id=F(1, STR),
        path=F(2, STR),
        source_id=F(3, STR),
        source_kind=F(4, Ref.SourceKind),
        active=F(5, BOOL),
        desired=F(6, Ref.DesiredRef, optional=True),
        remotes=F(7, List(Ref.RemoteSpec))),

    MaterializeTarget=Msg(
        kind=F(1, Ref.MaterializeTargetKind),
        name=F(2, STR, optional=True),
        commit=F(3, STR, optional=True)),

    ResolvedMemberState=Msg(
        member_id=F(1, STR),
        path=F(2, STR),
        source_id=F(3, STR),
        source_kind=F(4, Ref.SourceKind),
        commit=F(5, STR, optional=True),
        branch=F(6, STR, optional=True),
        detached=F(7, BOOL, optional=True),
        upstream=F(8, STR, optional=True),
        dirty=F(9, BOOL, optional=True),
        materialized=F(10, BOOL),
        remotes=F(11, List(Ref.RemoteSpec))),

    GitStatus=Msg(
        member_id=F(1, STR),
        branch=F(2, STR, optional=True),
        detached=F(3, BOOL),
        head=F(4, STR, optional=True),
        upstream=F(5, STR, optional=True),
        ahead=F(6, INT, optional=True),
        behind=F(7, INT, optional=True),
        staged=F(8, INT),
        unstaged=F(9, INT),
        untracked=F(10, INT),
        dirty=F(11, BOOL)),

    GitFileChange=Msg(
        member_id=F(1, STR),
        member_path=F(2, STR),
        repo_path=F(3, STR),
        workspace_path=F(4, STR),
        index_status=F(5, STR),
        worktree_status=F(6, STR),
        original_repo_path=F(7, STR, optional=True)),

    GitMemberBranchStatus=Msg(
        member_id=F(1, STR),
        member_path=F(2, STR),
        label=F(3, STR),
        branch=F(4, STR, optional=True),
        detached=F(5, BOOL),
        unborn=F(6, BOOL),
        head=F(7, STR, optional=True),
        upstream=F(8, STR, optional=True),
        ahead=F(9, INT, optional=True),
        behind=F(10, INT, optional=True)),

    GitBranchGroup=Msg(
        label=F(1, STR),
        member_ids=F(2, List(STR)),
        member_paths=F(3, List(STR))),

    GitBranchDifference=Msg(
        label=F(1, STR),
        majority_label=F(2, STR, optional=True),
        member_ids=F(3, List(STR)),
        member_paths=F(4, List(STR)),
        message=F(5, STR, optional=True)),

    WorkspaceGitStatus=Msg(
        clean=F(1, BOOL),
        file_changes=F(2, List(Ref.GitFileChange)),
        branches=F(3, List(Ref.GitMemberBranchStatus)),
        branch_groups=F(4, List(Ref.GitBranchGroup)),
        branch_differences=F(5, List(Ref.GitBranchDifference))),

    PlannedChange=Msg(
        action=F(1, Ref.PlannedAction),
        from_ref=F(2, STR, optional=True),
        to_ref=F(3, STR, optional=True),
        message=F(4, STR, optional=True)),

    MemberResponse=Msg(
        member_id=F(1, STR),
        member_path=F(2, STR),
        source_kind=F(3, Ref.SourceKind),
        status=F(4, Ref.MemberStatus),
        error=F(5, Ref.GwsError, optional=True),
        planned=F(6, Ref.PlannedChange, optional=True),
        state=F(7, Ref.ResolvedMemberState, optional=True),
        git_status=F(8, Ref.GitStatus, optional=True),
        lock_match=F(9, Ref.LockMatch, optional=True)),

    ResponseEnvelope=Msg(
        meta=F(1, Ref.ResponseMeta),
        members=F(2, List(Ref.MemberResponse)),
        errors=F(3, List(Ref.GwsError))),

    OperationEvent=Msg(
        operation_id=F(1, STR),
        request_id=F(2, STR),
        sequence=F(3, INT),
        timestamp_ms=F(4, INT),
        kind=F(5, Ref.EventKind),
        severity=F(6, Ref.Severity),
        member_id=F(7, STR, optional=True),
        member_path=F(8, STR, optional=True),
        message=F(9, STR, optional=True),
        member=F(10, Ref.MemberResponse, optional=True),
        error=F(11, Ref.GwsError, optional=True),
        attribution=F(12, Ref.OperationAttribution, optional=True)),

    OperationResult=Msg(
        operation_id=F(1, STR),
        request_id=F(2, STR),
        action=F(3, Ref.ActionKind),
        aggregate_status=F(4, Ref.AggregateStatus),
        started_at_ms=F(5, INT),
        finished_at_ms=F(6, INT),
        members=F(7, List(Ref.MemberResponse)),
        errors=F(8, List(Ref.GwsError)),
        attribution=F(9, Ref.OperationAttribution, optional=True)),

    # ---- action requests --------------------------------------------------
    CreateWorkspaceRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        workspace_root=F(2, STR),
        workspace_id=F(3, STR, optional=True)),

    InitFromSourcesRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        workspace_root=F(2, STR),
        sources=F(3, List(Ref.SourceUrl)),
        target=F(4, Ref.MaterializeTarget, optional=True),
        workspace_id=F(5, STR, optional=True)),

    AddExistingRepoRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        repository_path=F(2, STR),
        member_path=F(3, STR, optional=True),
        member_id=F(4, STR, optional=True),
        source_id=F(5, STR, optional=True)),

    CreateRepoRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        member_path=F(2, STR),
        initial_branch=F(3, STR, optional=True),
        member_id=F(4, STR, optional=True),
        source_id=F(5, STR, optional=True)),

    MaterializeRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        target=F(2, Ref.MaterializeTarget)),

    StatusRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        mode=F(2, Ref.StatusMode, optional=True),
        include_file_changes=F(3, BOOL, optional=True),
        include_branch_summary=F(4, BOOL, optional=True),
        path_style=F(5, Ref.StatusPathStyle, optional=True)),

    SnapshotRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        snapshot_id=F(2, STR)),

    TagRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        tag_name=F(2, STR)),

    PullHeadRequest=Msg(
        meta=F(1, Ref.RequestMeta)),
    PullSnapshotRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        snapshot_id=F(2, STR)),

    PushRequest=Msg(
        meta=F(1, Ref.RequestMeta),
        remote=F(2, STR, optional=True),
        refspec=F(3, STR, optional=True)),

    # ---- action responses -------------------------------------------------
    CreateWorkspaceResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    InitFromSourcesResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    AddExistingRepoResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    CreateRepoResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    MaterializeResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    StatusResponse=Msg(
        response=F(1, Ref.ResponseEnvelope),
        workspace_git_status=F(2, Ref.WorkspaceGitStatus, optional=True)),
    SnapshotResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    TagResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    PullHeadResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    PullSnapshotResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
    PushResponse=Msg(
        response=F(1, Ref.ResponseEnvelope)),
)

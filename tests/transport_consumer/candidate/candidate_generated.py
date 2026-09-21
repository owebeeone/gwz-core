"""GENERATED native Python types — do not edit."""
from __future__ import annotations
from dataclasses import dataclass
from enum import Enum

class MessageKind(Enum):
    bind = 1
    bind_rejected = 3
    bound = 2
    cancel = 14
    check_identity = 16
    close = 12
    closed = 13
    data = 7
    end_write = 11
    failed = 15
    flush = 9
    flushed = 10
    identity_check_failed = 18
    identity_checked = 17
    open = 4
    open_failed = 6
    opened = 5
    window = 8

class EndpointRole(Enum):
    driver = 2
    local = 1

class Scheme(Enum):
    https = 2
    ssh = 1

class AuthPolicy(Enum):
    anonymous = 3
    gh = 4
    ssh_ambient = 1
    ssh_explicit = 2

class GitService(Enum):
    receive_pack_advertisement = 3
    receive_pack_exchange = 4
    upload_pack_advertisement = 1
    upload_pack_exchange = 2

class IdentityMode(Enum):
    ambient = 1
    credentials_disabled = 3
    explicit_key = 2

class ErrorCode(Enum):
    authentication = 8
    cancelled = 7
    capacity = 5
    carrier_lost = 12
    invalid_request = 4
    io = 10
    protocol = 11
    repository_refused = 13
    timeout = 6
    trust = 9
    unavailable = 3
    unsupported_operation = 2
    unsupported_version = 1

class Effect(Enum):
    none = 1
    possible = 2

class Disposition(Enum):
    discarded = 2
    reusable = 1

class AuthMethod(Enum):
    gh = 4
    none = 1
    ssh_agent = 2
    ssh_key = 3

class TransportPlacement(Enum):
    local = 1
    cli = 2

class ActionKind(Enum):
    create_workspace = 0
    init_from_sources = 1
    add_existing_repo = 2
    create_repo = 3
    materialize = 4
    status = 5
    snapshot = 6
    tag = 7
    pull_head = 8
    pull_snapshot = 9
    push = 10
    capture = 11
    commit = 12
    stage = 13
    ls = 14
    forall = 15
    repo_sync = 16
    stash = 17
    branch = 18
    clone_workspace = 19
    list_snapshots = 20
    diff = 21
    clone_repo_member = 22
    detach_repo_member = 23
    attach_repo_member = 24
    merge = 25
    log = 26
    clone_local_workspace = 27
    local_family = 28
    remote_identity = 29
    fetch = 30

class TagOp(Enum):
    create = 0
    list = 1
    fetch = 2
    push = 3
    delete = 4

class StashOp(Enum):
    push = 0
    list = 1
    apply = 2
    pop = 3
    drop = 4

class StashParticipation(Enum):
    stashed = 0
    empty = 1
    skipped = 2

class StashPushLifecycle(Enum):
    unattempted = 0
    saving = 1
    saved = 2
    empty = 3
    failed = 4

class StashRestoreState(Enum):
    pending = 0
    applied = 1
    popped = 2
    dropped = 3
    noop = 4
    missing = 5

class BranchOp(Enum):
    list = 0
    create = 1
    delete = 2
    merge = 3

class MergeOp(Enum):
    start = 0
    resume = 1
    abort = 2
    status = 3
    gc = 4

class MergeMode(Enum):
    normal = 0
    ff_only = 1
    no_ff = 2

class LocalCloneMode(Enum):
    verbatim = 0
    clean = 1
    bare = 2

class LocalFamilyOp(Enum):
    list = 0
    dispose = 1
    disband = 2

class LocalMemberKind(Enum):
    checkout = 0
    bare = 1

class LocalMemberState(Enum):
    creating = 0
    ready = 1
    disposing = 2

class LocalObservedState(Enum):
    ready = 0
    incomplete = 1
    interrupted_disposal = 2
    missing = 3
    pointer_removed = 4
    mismatched = 5
    malformed = 6
    unobserved = 7

class MergeAnalysisKind(Enum):
    up_to_date = 0
    fast_forward = 1
    true_merge = 2
    unknown = 3

class MergePendingActionKind(Enum):
    verify_up_to_date = 0
    fast_forward = 1
    true_merge = 2
    resolve_conflict = 3

class MergePendingActionState(Enum):
    not_started = 0
    expected_conflict = 1
    completed_exactly = 2
    ambiguous = 3

class MergeParticipantState(Enum):
    planned = 0
    up_to_date = 1
    fast_forwarded = 2
    merged = 3
    conflicted = 4
    failed = 5
    unattempted = 6
    continued = 7
    aborted = 8
    rolled_back = 9

class MergeOperationState(Enum):
    executing = 0
    awaiting_resolution = 1
    halted = 2
    finalizing = 3
    preserving = 4
    rolling_back = 5
    completed = 6
    aborted = 7
    recovery_required = 8
    idle = 9

class MergeParticipantDriftKind(Enum):
    branch_changed = 0
    head_advanced = 1
    head_rewound = 2
    target_ref_changed = 3
    worktree_modified = 4
    index_modified = 5
    merge_state_missing = 6
    merge_head_changed = 7
    new_integration_state = 8
    repository_missing = 9
    head_diverged = 10
    object_missing = 11
    foreign_integration_state = 12
    pending_action_ambiguous = 13

class MergeOperationDriftKind(Enum):
    baseline_lock_changed = 0
    baseline_manifest_changed = 1
    root_candidate_metadata_invalid = 2
    root_candidate_state_changed = 3
    record_unreadable = 4

class MergePublicationStep(Enum):
    not_started = 0
    validating_results = 1
    preparing_candidate = 2
    committing_evidence = 3
    publishing_candidate = 4
    verifying_publication = 5
    complete = 6

class MergeRecordVersion(Enum):
    v0 = 0
    v1 = 1

class MergeTerminalOutcome(Enum):
    completed = 0
    aborted = 1

class MergeAcceptanceKind(Enum):
    supported_persisted = 0
    legacy_complete = 1
    legacy_unavailable = 2
    not_accepted = 3

class MergeInstalledAcceptedWorkspaceKind(Enum):
    v1 = 0

class MergeLegacyAcceptanceSource(Enum):
    candidate = 0
    baseline_no_publication = 1

class MergeLegacyAcceptanceGap(Enum):
    exact_lock_bytes = 0
    complete_member_audit = 1
    accepted_root_input = 2
    publication_evidence = 3

class MergeAcceptedMemberKind(Enum):
    selected = 0
    unselected_present = 1
    absent = 2

class MergeAcceptedRootKind(Enum):
    born_attached = 0
    born_detached = 1
    unborn_attached = 2

class MergeAcceptedMetadataSource(Enum):
    operation_baseline = 0
    selected_root_result = 1

class MergeRecoveryOriginState(Enum):
    executing = 0
    awaiting_resolution = 1
    halted = 2
    finalizing = 3
    preserving = 4
    rolling_back = 5

class MergeCompatibilityBasePhase(Enum):
    pre_acceptance = 0
    pre_candidate = 1
    candidate_persisted = 2
    evidence_unrecorded = 3
    evidence_recorded = 4
    publishing_prefix = 5
    published = 6
    no_publication_complete = 7

class MergeCompatibilityNextAction(Enum):
    reconcile_pending_participant = 0
    execute_next_participant = 1
    await_resolution = 2
    validate_results = 3
    persist_acceptance = 4
    prepare_candidate = 5
    create_or_adopt_evidence = 6
    publish_candidate = 7
    verify_publication = 8
    complete_no_publication = 9
    resume_preservation = 10
    resume_rollback = 11
    archive_completed = 12
    archive_aborted = 13
    report_recovery_required = 14

class MergeCrashRecoveryGap(Enum):
    no_durable_identity = 0
    remote_filesystem = 1
    volatile_filesystem = 2

class BranchActionResult(Enum):
    listed = 0
    created = 1
    exists = 2
    deleted = 3
    switched = 4
    noop = 5
    skipped = 6
    merged = 7
    conflicted = 8

class ExecMode(Enum):
    argv = 0
    shell = 1

class SourceKind(Enum):
    git = 0
    archive = 1
    package = 2
    local = 3
    generated = 4

class TargetKind(Enum):
    root = 0
    member = 1

class AggregateStatus(Enum):
    accepted = 0
    ok = 1
    noop = 2
    rejected = 3
    partial = 4
    failed = 5
    dirty = 6
    conflicted = 7

class MemberStatus(Enum):
    planned = 0
    ok = 1
    noop = 2
    skipped = 3
    rejected = 4
    failed = 5
    conflicted = 6

class MaterializeTargetKind(Enum):
    lock = 0
    head = 1
    snapshot = 2
    tag = 3
    commit = 4
    branch = 5

class SnapshotSourceKind(Enum):
    current = 0
    branch = 1

class SyncBehavior(Enum):
    fetch_only = 0
    ff_only = 1
    merge = 2
    rebase = 3
    reset = 4
    driver_selected = 5

class PartialBehavior(Enum):
    atomic = 0
    partial = 1

class DestructiveBehavior(Enum):
    refuse = 0
    allow = 1

class UnsupportedMemberBehavior(Enum):
    fail = 0
    skip = 1

class PlannedAction(Enum):
    noop = 0
    clone = 1
    fetch = 2
    fast_forward = 3
    checkout = 4
    init_repo = 5
    add_manifest_member = 6
    write_manifest = 7
    write_lock = 8
    write_snapshot = 9
    write_tag = 10
    push = 11
    merge = 12
    rebase = 13
    reset = 14
    detach_member = 15
    attach_member = 16

class LockMatch(Enum):
    unknown = 0
    matches = 1
    differs = 2
    missing = 3

class LockDifferenceReason(Enum):
    dirty_worktree = 0
    commit = 1
    branch = 2
    attachment = 3
    missing_lock_entry = 4
    unavailable_observations = 5

class GitProgressPhase(Enum):
    enumerating = 0
    counting = 1
    compressing = 2
    receiving = 3
    resolving = 4
    checking_out = 5
    writing = 6

class StatusMode(Enum):
    summary = 0
    combined = 1

class StatusPathStyle(Enum):
    member_relative = 0
    workspace_relative = 1

class EventKind(Enum):
    operation_started = 0
    member_started = 1
    member_progress = 2
    member_finished = 3
    artifact_written = 4
    operation_finished = 5
    reset = 6
    operation_state_changed = 7
    diagnostic = 8

class Severity(Enum):
    debug = 0
    info = 1
    warn = 2
    error = 3

class GwzErrorCode(Enum):
    ok = 0
    invalid_request = 1
    workspace_not_found = 2
    workspace_already_exists = 3
    nested_workspace = 4
    manifest_not_found = 5
    manifest_invalid = 6
    schema_unsupported = 7
    member_not_found = 8
    member_inactive = 9
    path_escape = 10
    path_collision = 11
    path_reserved = 12
    unsupported_source_kind = 13
    unsupported_operation = 14
    dirty_member = 15
    diverged_member = 16
    missing_remote = 17
    snapshot_not_found = 18
    lock_not_found = 19
    tag_not_found = 20
    tag_invalid = 21
    remote_rejected = 22
    git_command_failed = 23
    external_tool_missing = 24
    operation_not_found = 25
    attribution_denied = 26
    permission_denied = 27
    io_error = 28
    internal_error = 29
    branch_detached_head = 30
    branch_unborn_head = 31
    branch_mixed = 32
    stash_not_found = 33
    stash_incomplete = 34
    stash_conflict = 35
    source_identity_mismatch = 36
    deprecated_operation = 37
    merge_validation_failed = 38
    merge_id_mismatch = 39
    merge_drift = 40
    open_operation = 41
    merge_recovery_required = 42
    merge_phase_unsupported = 43
    root_merge_not_yet_supported = 44
    merge_record_unreadable = 45
    unsupported_record_version = 46
    unsupported_legacy_mode = 47
    archived_record_unreadable = 48
    unexpected_acceptance_evidence = 49
    acceptance_input_drift = 50
    candidate_integrity_mismatch = 51
    ambiguous_evidence_commit = 52
    recorded_evidence_drift = 53
    publication_prefix_mismatch = 54
    published_candidate_mismatch = 55
    preservation_evidence_mismatch = 56
    rollback_evidence_mismatch = 57
    unexpected_publication_evidence = 58
    terminal_evidence_mismatch = 59
    recovery_evidence_mismatch = 60
    terminal_rollback_mismatch = 61
    unknown_local = 62
    unsupported_source_layout = 63
    copy_failed = 64
    source_drift = 65
    destination_incomplete = 66
    pairing_mismatch = 67
    import_incomplete = 68
    unwaived_hazard = 69
    unknown_evidence = 70
    disposal_incomplete = 71
    url_scheme_unavailable = 72

class MergeRecordRequiredWave(Enum):
    a1 = 0
    a2 = 1
    a3 = 2
    a4 = 3

class DiffComparisonKind(Enum):
    worktree_vs_index = 0
    index_vs_tree = 1
    worktree_vs_tree = 2
    tree_vs_tree = 3

class DiffOutputFormat(Enum):
    patch = 0
    raw = 1
    name_only = 2
    name_status = 3
    stat = 4
    numstat = 5
    shortstat = 6
    summary = 7
    patch_with_raw = 8
    patch_with_stat = 9
    no_patch = 10

class DiffManifestMode(Enum):
    full = 0
    any_difference = 1

class DiffAlgorithm(Enum):
    default = 0
    myers = 1
    minimal = 2
    patience = 3

class DiffWhitespaceMode(Enum):
    default = 0
    ignore_all = 1
    ignore_change = 2
    ignore_eol = 3
    ignore_blank_lines = 4

class DiffStatus(Enum):
    added = 0
    modified = 1
    deleted = 2
    renamed = 3
    copied = 4
    type_changed = 5
    unmerged = 6

class DiffChunkEncoding(Enum):
    utf8 = 0
    bytes = 1

class DiffOutputRecordKind(Enum):
    patch_bytes = 0
    file_started = 1
    file_finished = 2
    stale_file = 3
    diagnostic = 4

class DiffTargetExclusionReason(Enum):
    snapshot_missing = 0
    snapshot_missing_commit = 1
    root_not_in_snapshot = 2
    tag_missing = 3

class LogMergeKind(Enum):
    none = 0
    marker = 1
    heuristic = 2

class LogDegradationReason(Enum):
    repository_unreadable = 0
    repository_missing = 1
    unborn = 2
    revision_unresolved = 3
    snapshot_entry_missing = 4
    lock_entry_missing = 5
    unsupported_source_kind = 6

class LogOutputRecordKind(Enum):
    entry = 0
    degradation = 1

class RemoteIdentityOp(Enum):
    get = 0
    set = 1
    unset = 2

class TransportCredentialMethod(Enum):
    unknown = 0
    file = 1
    agent = 2
    helper = 3

class TransportSelectionSource(Enum):
    ambient = 0
    invocation_remote = 1
    invocation_default = 2
    local_configuration = 3

class TransportOperation(Enum):
    clone = 0
    fetch = 1
    push = 2
    read_advertisement = 3

class UrlScheme(Enum):
    manifest = 0
    ssh = 1
    https = 2

class UrlSchemeSource(Enum):
    default = 0
    request = 1
    workspace = 2

class FetchResult(Enum):
    updated = 0
    unchanged = 1
    no_upstream = 2
    failed = 3
    planned = 4

class RemoteCheck(Enum):
    changed = 0
    always = 1

@dataclass(slots=True)
class Limits:
    encoded_frame: int
    data_payload: int
    metadata_bytes: int
    nesting: int
    collection_entries: int
    decode_allocation: int
    queued_bytes: int
    queued_frames: int
    receive_window: int
    control_reserve_bytes: int
    control_reserve_frames: int

@dataclass(slots=True)
class Bind:
    versions: list[int]
    role: EndpointRole
    schemes: list[Scheme]
    policies: list[AuthPolicy]
    receive_limits: Limits

@dataclass(slots=True)
class Bound:
    version: int
    endpoint_id: str
    role: EndpointRole
    schemes: list[Scheme]
    policies: list[AuthPolicy]
    receive_limits: Limits
    trust_owner: str

@dataclass(slots=True)
class Failure:
    code: ErrorCode
    effect: Effect
    facts: Facts | None

@dataclass(slots=True)
class Destination:
    scheme: Scheme
    host: str
    port: int
    path: str
    ssh_username: str | None

@dataclass(slots=True)
class Identity:
    mode: IdentityMode
    key_path: str | None
    path_base: str | None

@dataclass(slots=True)
class Deadlines:
    allocation_ms: int
    connect_ms: int
    io_ms: int
    interaction_ms: int
    cleanup_ms: int

@dataclass(slots=True)
class Open:
    endpoint_id: str
    operation_id: str
    destination: Destination
    service: GitService
    identity: Identity
    policy: AuthPolicy
    deadlines: Deadlines
    receive_limits: Limits

@dataclass(slots=True)
class CheckIdentity:
    endpoint_id: str
    operation_id: str
    identity: Identity
    timeout_ms: int

@dataclass(slots=True)
class IdentityChecked:
    pass

@dataclass(slots=True)
class Facts:
    method: AuthMethod
    credential_offered: bool
    authenticated: bool | None
    key_fingerprint: str | None
    http_status: int | None
    ssh_exit_status: int | None

@dataclass(slots=True)
class Opened:
    connection_id: str
    reused: bool
    endpoint_id: str
    trust_owner: str
    facts: Facts
    receive_limits: Limits

@dataclass(slots=True)
class Data:
    offset: int
    payload: bytes

@dataclass(slots=True)
class Window:
    max_offset: int

@dataclass(slots=True)
class Barrier:
    barrier_id: int
    offset: int

@dataclass(slots=True)
class EndWrite:
    final_offset: int

@dataclass(slots=True)
class Close:
    final_offset: int

@dataclass(slots=True)
class Closed:
    disposition: Disposition
    unread_response_discarded: bool
    facts: Facts
    failure: Failure | None

@dataclass(slots=True)
class Cancel:
    reason: ErrorCode

@dataclass(slots=True)
class Envelope:
    version: int
    session_id: str
    stream_id: int
    kind: MessageKind
    bind: Bind | None
    bound: Bound | None
    bind_rejected: Failure | None
    open: Open | None
    opened: Opened | None
    open_failed: Failure | None
    data: Data | None
    window: Window | None
    flush: Barrier | None
    flushed: Barrier | None
    end_write: EndWrite | None
    close: Close | None
    closed: Closed | None
    cancel: Cancel | None
    failed: Failure | None
    check_identity: CheckIdentity | None
    identity_checked: IdentityChecked | None
    identity_check_failed: Failure | None

@dataclass(slots=True)
class WorkspaceRef:
    root: str | None
    workspace_id: str | None

@dataclass(slots=True)
class OperationActor:
    actor_id: str
    display_name: str | None
    email: str | None
    authority: str | None

@dataclass(slots=True)
class GitObjectIdentity:
    name: str
    email: str
    time_ms: int | None
    timezone_offset_minutes: int | None

@dataclass(slots=True)
class OperationAttribution:
    actor: OperationActor | None
    git_author: GitObjectIdentity | None
    git_committer: GitObjectIdentity | None
    credential_ref: str | None

@dataclass(slots=True)
class Selection:
    all: bool | None
    member_ids: list[str]
    paths: list[str]
    targets: list[str]
    exclude_targets: list[str]

@dataclass(slots=True)
class OperationPolicy:
    partial: PartialBehavior | None
    destructive: DestructiveBehavior | None
    sync: SyncBehavior | None
    unsupported_member: UnsupportedMemberBehavior | None
    remote: str | None
    concurrency: int | None
    progress_min_interval_ms: int | None
    max_connections_per_host: int | None

@dataclass(slots=True)
class RemoteSshIdentity:
    remote: str
    private_key_path: str

@dataclass(slots=True)
class RemoteIdentityRequest:
    meta: RequestMeta
    remote: str
    op: RemoteIdentityOp
    private_key_path: str | None

@dataclass(slots=True)
class RemoteIdentityEntry:
    member_id: str
    member_path: str
    remote: str
    private_key_path: str | None

@dataclass(slots=True)
class RemoteIdentityResponse:
    response: ResponseEnvelope
    identities: list[RemoteIdentityEntry]

@dataclass(slots=True)
class TransportRuntimeRequest:
    server_timeout_ms: int
    schema_version: str

@dataclass(slots=True)
class TransportRuntimeResponse:
    server_timeout_ms: int

@dataclass(slots=True)
class TransportCapabilitiesRequest:
    schema_version: str

@dataclass(slots=True)
class TransportCapabilitiesResponse:
    file_identity: bool
    exact_agent_identity: bool
    message_versions: list[int] | None
    placements: list[TransportPlacement] | None
    schemes: list[Scheme] | None
    auth_policies: list[AuthPolicy] | None
    message_limits: Limits | None

@dataclass(slots=True)
class TransportObservation:
    repository_path: str
    remote: str
    operation: TransportOperation
    credential_method: TransportCredentialMethod
    selection_source: TransportSelectionSource
    credential_offered: bool
    authenticated: bool | None
    public_key_fingerprint: str | None
    endpoint_id: str | None
    connection_id: str | None
    stream_id: int | None
    reused: bool | None

@dataclass(slots=True)
class MemberUrlResolution:
    manifest_url: str
    effective_url: str
    scheme: UrlScheme
    source: UrlSchemeSource
    derived: bool
    host_known: bool

@dataclass(slots=True)
class TransportOptions:
    default_identity: str | None
    remote_identities: list[RemoteSshIdentity]
    url_scheme: UrlScheme | None
    placement: TransportPlacement | None
    endpoint_path_base: str | None

@dataclass(slots=True)
class InvocationContext:
    caller_cwd: str

@dataclass(slots=True)
class RequestMeta:
    request_id: str
    schema_version: str
    workspace: WorkspaceRef | None
    selection: Selection | None
    policy: OperationPolicy | None
    dry_run: bool | None
    attribution: OperationAttribution | None
    transport: TransportOptions | None
    invocation: InvocationContext | None
    transport_message: Envelope | None

@dataclass(slots=True)
class ResponseMeta:
    request_id: str
    schema_version: str
    action: ActionKind
    aggregate_status: AggregateStatus
    operation_id: str | None
    message: str | None
    attribution: OperationAttribution | None
    transport: list[TransportObservation] | None
    transport_message: Envelope | None

@dataclass(slots=True)
class MergeRecordCompatibilityContext:
    merge_id: str
    schema: str | None
    record_schema_version: int | None
    required_wave: MergeRecordRequiredWave | None
    legacy_mode: str | None

@dataclass(slots=True)
class GwzError:
    code: GwzErrorCode
    message: str
    member_id: str | None
    member_path: str | None
    detail: str | None
    target_kind: TargetKind | None
    record_context: MergeRecordCompatibilityContext | None

@dataclass(slots=True)
class RemoteSpec:
    name: str
    url: str
    fetch: bool | None
    push: bool | None

@dataclass(slots=True)
class DesiredRef:
    branch: str | None
    commit: str | None
    git_tag: str | None
    local_only: bool | None

@dataclass(slots=True)
class SourceUrl:
    url: str
    path: str | None
    remote_name: str | None
    branch: str | None

@dataclass(slots=True)
class MemberSpec:
    member_id: str
    path: str
    source_id: str
    source_kind: SourceKind
    active: bool
    desired: DesiredRef | None
    remotes: list[RemoteSpec]
    private: bool | None

@dataclass(slots=True)
class MaterializeTarget:
    kind: MaterializeTargetKind
    name: str | None
    commit: str | None

@dataclass(slots=True)
class SnapshotSource:
    kind: SnapshotSourceKind
    branch: str | None

@dataclass(slots=True)
class ResolvedMemberState:
    member_id: str
    path: str
    source_id: str
    source_kind: SourceKind
    commit: str | None
    branch: str | None
    detached: bool | None
    upstream: str | None
    dirty: bool | None
    materialized: bool
    remotes: list[RemoteSpec]

@dataclass(slots=True)
class GitStatus:
    member_id: str
    branch: str | None
    detached: bool
    head: str | None
    upstream: str | None
    ahead: int | None
    behind: int | None
    staged: int
    unstaged: int
    untracked: int
    dirty: bool

@dataclass(slots=True)
class GitFileChange:
    member_id: str
    member_path: str
    repo_path: str
    workspace_path: str
    index_status: str
    worktree_status: str
    original_repo_path: str | None

@dataclass(slots=True)
class GitTransferProgress:
    phase: GitProgressPhase
    received_objects: int | None
    total_objects: int | None
    received_bytes: int | None
    indexed_deltas: int | None
    total_deltas: int | None

@dataclass(slots=True)
class WorkspaceRootGitStatus:
    branch: str | None
    detached: bool
    head: str | None
    staged: int
    unstaged: int
    untracked: int
    dirty: bool
    unborn: bool

@dataclass(slots=True)
class WorkspaceRootFileChange:
    repo_path: str
    workspace_path: str
    index_status: str
    worktree_status: str
    original_repo_path: str | None

@dataclass(slots=True)
class GitMemberBranchStatus:
    member_id: str
    member_path: str
    label: str
    branch: str | None
    detached: bool
    unborn: bool
    head: str | None
    upstream: str | None
    ahead: int | None
    behind: int | None

@dataclass(slots=True)
class GitBranchGroup:
    label: str
    member_ids: list[str]
    member_paths: list[str]

@dataclass(slots=True)
class GitBranchDifference:
    label: str
    majority_label: str | None
    member_ids: list[str]
    member_paths: list[str]
    message: str | None

@dataclass(slots=True)
class WorkspaceGitStatus:
    clean: bool
    file_changes: list[GitFileChange]
    branches: list[GitMemberBranchStatus]
    branch_groups: list[GitBranchGroup]
    branch_differences: list[GitBranchDifference]
    root_status: WorkspaceRootGitStatus | None
    root_file_changes: list[WorkspaceRootFileChange]

@dataclass(slots=True)
class StashDirtySummary:
    staged: bool
    unstaged: bool
    untracked: bool
    ignored: bool

@dataclass(slots=True)
class StashErrorDetail:
    code: str
    message: str

@dataclass(slots=True)
class StashWarning:
    code: str
    message: str
    member_id: str | None

@dataclass(slots=True)
class StashDrift:
    code: str
    message: str
    member_id: str

@dataclass(slots=True)
class StashBundleMember:
    member_id: str
    path: str
    participation: StashParticipation
    push_lifecycle: StashPushLifecycle
    restore_state: StashRestoreState
    branch_before: str | None
    head_before: str | None
    full_stash_message: str
    dirty_summary: StashDirtySummary
    native_stash_object_id: str | None
    native_stash_display_ref: str | None
    error: StashErrorDetail | None

@dataclass(slots=True)
class StashBundle:
    schema: str
    workspace_id: str
    stash_id: str
    created_at: str
    message_suffix: str
    include_untracked: bool
    include_ignored: bool
    members: list[StashBundleMember]
    warnings: list[StashWarning]
    drift: list[StashDrift]
    selected_members: list[str]

@dataclass(slots=True)
class BranchRepoSummary:
    member_id: str
    member_path: str
    source_kind: SourceKind
    result: BranchActionResult
    branch: str | None
    current_branch: str | None
    detached: bool
    unborn: bool
    head: str | None
    upstream: str | None
    ahead: int | None
    behind: int | None
    source_ref: str | None
    target_branch: str | None
    resulting_commit: str | None
    conflict_paths: list[str]

@dataclass(slots=True)
class FetchRepoSummary:
    member_id: str
    member_path: str
    source_kind: SourceKind
    result: FetchResult
    remote: str | None
    branch: str | None
    before: str | None
    after: str | None
    upstream: str | None
    ahead: int | None
    behind: int | None

@dataclass(slots=True)
class MergeParticipantCounts:
    total: int
    planned: int
    up_to_date: int
    fast_forwarded: int
    merged: int
    conflicted: int
    failed: int
    unattempted: int
    continued: int
    aborted: int
    rolled_back: int

@dataclass(slots=True)
class MergeParticipantDrift:
    kind: MergeParticipantDriftKind
    message: str
    expected_branch: str | None
    live_branch: str | None
    expected_head: str | None
    live_head: str | None
    expected_merge_head: str | None
    live_merge_head: str | None

@dataclass(slots=True)
class MergeOperationDrift:
    kind: MergeOperationDriftKind
    message: str

@dataclass(slots=True)
class MergePreservation:
    target_id: str
    path: str
    backup_ref: str | None
    backup_commit: str | None
    stash_id: str | None
    stash_object_id: str | None

@dataclass(slots=True)
class MergePendingActionSummary:
    kind: MergePendingActionKind
    state: MergePendingActionState
    message: str | None

@dataclass(slots=True)
class MergeRecordProjection:
    source_version: MergeRecordVersion
    archived: bool
    terminal_outcome: MergeTerminalOutcome | None
    acceptance: MergeAcceptanceProjection | None
    recovery: MergeRecoveryProjection | None

@dataclass(slots=True)
class MergeAcceptanceProjection:
    kind: MergeAcceptanceKind
    supported_persisted: MergeInstalledAcceptedWorkspaceProjection | None
    legacy_complete: MergeLegacyAcceptedWorkspace | None
    legacy_source: MergeLegacyAcceptanceSource | None
    legacy_evidence: MergeLegacyAcceptanceEvidence | None
    missing_gaps: list[MergeLegacyAcceptanceGap]

@dataclass(slots=True)
class MergeInstalledAcceptedWorkspaceProjection:
    kind: MergeInstalledAcceptedWorkspaceKind
    v1: MergeAcceptedWorkspaceV1Projection | None

@dataclass(slots=True)
class MergeRecoveryProjection:
    origin_state: MergeRecoveryOriginState
    base_phase: MergeCompatibilityBasePhase
    next_action: MergeCompatibilityNextAction
    resume_action: MergeCompatibilityNextAction

@dataclass(slots=True)
class MergeAcceptedWorkspaceV1Projection:
    operation_baseline_lock_sha256: str
    metadata_base: MergeAcceptedMetadataBaseProjection
    lock_yaml: str
    lock_sha256: str
    members: list[MergeAcceptedMemberV1Projection]
    root: MergeAcceptedRootProjection

@dataclass(slots=True)
class MergeAcceptedMetadataBaseProjection:
    source: MergeAcceptedMetadataSource
    source_commit: str | None
    manifest_yaml: str
    manifest_sha256: str
    lock_yaml: str
    lock_sha256: str

@dataclass(slots=True)
class MergeAcceptedMemberV1Projection:
    member_id: str
    kind: MergeAcceptedMemberKind
    integration: MergeAcceptedIntegrationProjection | None
    final_checkout: MergeAcceptedCheckoutProjection | None
    lock_member: MergeAcceptedLockMemberProjection | None

@dataclass(slots=True)
class MergeAcceptedIntegrationProjection:
    branch: str
    before_commit: str
    resulting_commit: str

@dataclass(slots=True)
class MergeAcceptedCheckoutProjection:
    branch: str
    commit: str

@dataclass(slots=True)
class MergeAcceptedLockMemberProjection:
    path: str
    source_id: str
    source_kind: SourceKind
    commit: str | None
    branch: str | None
    detached: bool | None
    upstream: str | None
    dirty: bool | None
    materialized: bool | None

@dataclass(slots=True)
class MergeAcceptedRootProjection:
    kind: MergeAcceptedRootKind
    commit: str | None
    symbolic_branch: str | None
    publication_branch: str | None
    lock_worktree_sha256: str
    manifest_worktree_sha256: str
    lock_commit_sha256: str | None
    manifest_commit_sha256: str | None

@dataclass(slots=True)
class MergeLegacyAcceptedWorkspace:
    baseline_lock_sha256: str
    lock_yaml: str
    lock_sha256: str
    members: list[MergeAcceptedMemberV1Projection]
    root: MergeAcceptedRootProjection

@dataclass(slots=True)
class MergeLegacyAcceptanceEvidence:
    lock_yaml: str | None
    lock_sha256: str | None
    members: list[MergeLegacyMemberEvidence]
    root: MergeAcceptedRootProjection | None
    composition_commit: str | None
    composition_tree: str | None
    candidate_hashes: list[MergeAcceptedCandidateHashProjection]

@dataclass(slots=True)
class MergeLegacyMemberEvidence:
    member_id: str
    selected: bool
    state: MergeParticipantState | None
    integration: MergeAcceptedIntegrationProjection | None
    lock_member: MergeAcceptedLockMemberProjection | None

@dataclass(slots=True)
class MergeAcceptedCandidateHashProjection:
    path: str
    sha256: str

@dataclass(slots=True)
class MergeCrashRecovery:
    supported: bool
    filesystem: str | None
    gap: MergeCrashRecoveryGap | None
    handles_ok: bool | None

@dataclass(slots=True)
class MergeRepoSummary:
    target_id: str
    target_kind: TargetKind
    path: str
    source_ref: str
    source_commit: str
    target_branch: str
    before_commit: str
    resulting_commit: str | None
    live_commit: str | None
    state: MergeParticipantState
    predicted: MergeAnalysisKind | None
    prediction_complete: bool | None
    conflict_paths: list[str]
    continue_eligible: bool | None
    abort_eligible: bool | None
    drift: list[MergeParticipantDrift]
    error: GwzError | None
    pending_action: MergePendingActionSummary | None

@dataclass(slots=True)
class PlannedChange:
    action: PlannedAction
    from_ref: str | None
    to_ref: str | None
    message: str | None

@dataclass(slots=True)
class MemberResponse:
    member_id: str
    member_path: str
    source_kind: SourceKind
    status: MemberStatus
    error: GwzError | None
    planned: PlannedChange | None
    state: ResolvedMemberState | None
    git_status: GitStatus | None
    lock_match: LockMatch | None
    target_kind: TargetKind | None
    lock_difference_reasons: list[LockDifferenceReason] | None
    url_resolution: MemberUrlResolution | None

@dataclass(slots=True)
class ResponseEnvelope:
    meta: ResponseMeta
    members: list[MemberResponse]
    errors: list[GwzError]

@dataclass(slots=True)
class OperationEvent:
    operation_id: str
    request_id: str
    sequence: int
    timestamp_ms: int
    kind: EventKind
    severity: Severity
    member_id: str | None
    member_path: str | None
    message: str | None
    member: MemberResponse | None
    error: GwzError | None
    attribution: OperationAttribution | None
    progress: GitTransferProgress | None
    target_kind: TargetKind | None
    merge_state: MergeOperationState | None
    merge_member: MergeRepoSummary | None
    artifact_path: str | None

@dataclass(slots=True)
class OperationResult:
    operation_id: str
    request_id: str
    action: ActionKind
    aggregate_status: AggregateStatus
    started_at_ms: int
    finished_at_ms: int
    members: list[MemberResponse]
    errors: list[GwzError]
    attribution: OperationAttribution | None
    transport: list[TransportObservation] | None

@dataclass(slots=True)
class CreateWorkspaceRequest:
    meta: RequestMeta
    workspace_root: str
    workspace_id: str | None

@dataclass(slots=True)
class InitFromSourcesRequest:
    meta: RequestMeta
    workspace_root: str
    sources: list[SourceUrl]
    target: MaterializeTarget | None
    workspace_id: str | None

@dataclass(slots=True)
class CloneWorkspaceRequest:
    meta: RequestMeta
    url: str
    target: str

@dataclass(slots=True)
class AddExistingRepoRequest:
    meta: RequestMeta
    repository_path: str
    member_path: str | None
    member_id: str | None
    source_id: str | None

@dataclass(slots=True)
class CreateRepoRequest:
    meta: RequestMeta
    member_path: str
    initial_branch: str | None
    member_id: str | None
    source_id: str | None

@dataclass(slots=True)
class RepoSyncRequest:
    meta: RequestMeta
    private: bool | None

@dataclass(slots=True)
class CloneRepoMemberRequest:
    meta: RequestMeta
    source: SourceUrl
    member_id: str | None
    source_id: str | None

@dataclass(slots=True)
class DetachRepoMemberRequest:
    meta: RequestMeta

@dataclass(slots=True)
class AttachRepoMemberRequest:
    meta: RequestMeta

@dataclass(slots=True)
class MaterializeRequest:
    meta: RequestMeta
    target: MaterializeTarget

@dataclass(slots=True)
class StatusRequest:
    meta: RequestMeta
    mode: StatusMode | None
    include_file_changes: bool | None
    include_branch_summary: bool | None
    path_style: StatusPathStyle | None

@dataclass(slots=True)
class LsRequest:
    meta: RequestMeta
    include_unmaterialized: bool | None

@dataclass(slots=True)
class MemberEntry:
    id: str
    path: str
    abspath: str
    materialized: bool
    target_kind: TargetKind | None
    note: str | None

@dataclass(slots=True)
class LsResponse:
    response: ResponseEnvelope
    members: list[MemberEntry] | None

@dataclass(slots=True)
class ExecResult:
    id: str
    path: str
    exit_code: int | None
    signal: int | None
    spawn_error: str | None

@dataclass(slots=True)
class ExecRequest:
    meta: RequestMeta
    mode: ExecMode
    command: list[str]
    members: list[MemberEntry]
    continue_on_fail: bool | None

@dataclass(slots=True)
class ExecResponse:
    response: ResponseEnvelope
    results: list[ExecResult] | None

@dataclass(slots=True)
class SnapshotRequest:
    meta: RequestMeta
    snapshot_id: str
    source: SnapshotSource | None

@dataclass(slots=True)
class ListSnapshotsRequest:
    meta: RequestMeta

@dataclass(slots=True)
class TagRequest:
    meta: RequestMeta
    op: TagOp
    name: str | None
    message: str | None
    signed: bool | None
    remote: str | None
    all: bool | None

@dataclass(slots=True)
class CaptureRequest:
    meta: RequestMeta

@dataclass(slots=True)
class CommitRequest:
    meta: RequestMeta
    message: str
    all: bool | None
    commit_marker: bool | None

@dataclass(slots=True)
class StageRequest:
    meta: RequestMeta
    cwd: str
    pathspecs: list[str]
    all: bool | None

@dataclass(slots=True)
class PullHeadRequest:
    meta: RequestMeta

@dataclass(slots=True)
class PullSnapshotRequest:
    meta: RequestMeta
    snapshot_id: str

@dataclass(slots=True)
class PushRequest:
    meta: RequestMeta
    remote: str | None
    refspec: str | None
    remote_check: RemoteCheck | None

@dataclass(slots=True)
class FetchRequest:
    meta: RequestMeta

@dataclass(slots=True)
class StashRequest:
    meta: RequestMeta
    op: StashOp
    stash_id: str | None
    message: str | None
    include_untracked: bool | None
    include_ignored: bool | None
    expanded: bool | None
    preserve_index: bool | None

@dataclass(slots=True)
class BranchRequest:
    meta: RequestMeta
    op: BranchOp
    name: str | None
    start_ref: str | None
    switch_after_create: bool | None

@dataclass(slots=True)
class MergeRequest:
    meta: RequestMeta
    op: MergeOp
    source_ref: str | None
    merge_id: str | None
    mode: MergeMode | None
    message: str | None
    preserve: bool | None
    filesystem_strict: bool | None
    local_source_name: str | None
    wait_seconds: int | None

@dataclass(slots=True)
class CloneLocalWorkspaceRequest:
    meta: RequestMeta
    name: str
    dest: str | None
    mode: LocalCloneMode
    branch: str | None
    copy_source: str | None
    owner: str | None
    wait_seconds: int | None

@dataclass(slots=True)
class LocalFamilyRequest:
    meta: RequestMeta
    op: LocalFamilyOp
    name: str | None
    keep: bool | None
    force_hazards: list[str]
    wait_seconds: int | None

@dataclass(slots=True)
class CreateWorkspaceResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class InitFromSourcesResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class CloneWorkspaceResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class AddExistingRepoResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class CreateRepoResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class RepoSyncResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class CloneRepoMemberResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class DetachRepoMemberResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class AttachRepoMemberResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class MaterializeResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class StatusResponse:
    response: ResponseEnvelope
    workspace_git_status: WorkspaceGitStatus | None

@dataclass(slots=True)
class SnapshotResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class SnapshotInfo:
    name: str
    created_at: str
    created_by: str
    members: int

@dataclass(slots=True)
class ListSnapshotsResponse:
    response: ResponseEnvelope
    snapshots: list[SnapshotInfo] | None

@dataclass(slots=True)
class TagInfo:
    name: str
    members: int

@dataclass(slots=True)
class TagResponse:
    response: ResponseEnvelope
    tags: list[TagInfo] | None

@dataclass(slots=True)
class CaptureResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class CommitResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class StageResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class PullHeadResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class PullSnapshotResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class PushResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class FetchResponse:
    response: ResponseEnvelope
    repos: list[FetchRepoSummary] | None

@dataclass(slots=True)
class StashResponse:
    response: ResponseEnvelope
    bundles: list[StashBundle] | None

@dataclass(slots=True)
class BranchResponse:
    response: ResponseEnvelope
    repos: list[BranchRepoSummary] | None

@dataclass(slots=True)
class MergeResponse:
    response: ResponseEnvelope
    merge_id: str | None
    state: MergeOperationState
    open: bool
    participant_counts: MergeParticipantCounts
    repos: list[MergeRepoSummary]
    operation_drift: list[MergeOperationDrift]
    preservation: list[MergePreservation] | None
    publication_step: MergePublicationStep | None
    record: MergeRecordProjection | None
    crash_recovery: MergeCrashRecovery | None

@dataclass(slots=True)
class CloneLocalWorkspaceResponse:
    response: ResponseEnvelope

@dataclass(slots=True)
class LocalFamilyMemberEntry:
    name: str
    kind: LocalMemberKind
    recorded_state: LocalMemberState
    observed_state: LocalObservedState
    path: str
    last_error: str | None
    owner: str | None

@dataclass(slots=True)
class LocalFamilyResponse:
    response: ResponseEnvelope
    members: list[LocalFamilyMemberEntry]
    root_path: str | None

@dataclass(slots=True)
class DiffComparison:
    kind: DiffComparisonKind
    left: str | None
    right: str | None
    merge_base: bool | None

@dataclass(slots=True)
class DiffOptions:
    output_format: DiffOutputFormat | None
    context_lines: int | None
    interhunk_lines: int | None
    algorithm: DiffAlgorithm | None
    whitespace: DiffWhitespaceMode | None
    find_renames: bool | None
    find_copies: bool | None
    rename_threshold: int | None
    rename_limit: int | None
    binary: bool | None
    text: bool | None
    full_index: bool | None
    abbrev: int | None
    reverse: bool | None
    null_terminated: bool | None
    src_prefix: str | None
    dst_prefix: str | None
    no_prefix: bool | None
    line_prefix: str | None
    ignore_submodules: str | None
    diff_filter: str | None
    manifest_mode: DiffManifestMode | None
    echo_manifest_entries: bool | None

@dataclass(slots=True)
class DiffRequest:
    meta: RequestMeta
    workspace_cwd: str | None
    operands: list[str]
    explicit_pathspecs: list[str]
    options: DiffOptions | None
    cached: bool | None
    merge_base: bool | None
    tagged: bool | None

@dataclass(slots=True)
class DiffRepoScope:
    root: bool | None
    member_id: str | None
    member_path: str | None
    source_kind: SourceKind | None

@dataclass(slots=True)
class DiffExcludedTarget:
    scope: DiffRepoScope
    reason: DiffTargetExclusionReason
    snapshot_id: str | None
    message: str | None

@dataclass(slots=True)
class DiffParsedTarget:
    target_id: str
    scope: DiffRepoScope
    comparison: DiffComparison
    pathspecs: list[str]
    left_oid: str | None
    right_oid: str | None
    merge_base_oid: str | None
    left_snapshot_id: str | None
    right_snapshot_id: str | None

@dataclass(slots=True)
class DiffFileEntry:
    file_id: str
    scope: DiffRepoScope
    status: DiffStatus
    old_path: str | None
    new_path: str | None
    old_mode: int | None
    new_mode: int | None
    similarity: int | None
    insertions: int | None
    deletions: int | None
    is_binary: bool | None

@dataclass(slots=True)
class DiffRepoSummary:
    scope: DiffRepoScope
    has_differences: bool
    files_changed: int
    insertions: int
    deletions: int
    files_manifested: int

@dataclass(slots=True)
class DiffSummary:
    has_differences: bool
    repos_examined: int
    repos_with_differences: int
    files_changed: int
    insertions: int
    deletions: int
    repo_summaries: list[DiffRepoSummary]

@dataclass(slots=True)
class DiffOutputLogRef:
    log_id: str
    format: DiffOutputFormat
    encoding: DiffChunkEncoding | None

@dataclass(slots=True)
class DiffManifestResponse:
    response: ResponseEnvelope
    files: list[DiffFileEntry]
    summary: DiffSummary | None
    targets: list[DiffParsedTarget]
    output: DiffOutputLogRef | None
    excluded_targets: list[DiffExcludedTarget]

@dataclass(slots=True)
class DiffOutputRecord:
    kind: DiffOutputRecordKind
    scope: DiffRepoScope | None
    file_id: str | None
    entry: DiffFileEntry | None
    data: bytes | None
    stale: bool | None
    diagnostic: str | None

@dataclass(slots=True)
class LogOptions:
    max_entries: int | None
    since: str | None
    until: str | None
    author: str | None
    grep: str | None
    no_merges: bool | None
    first_parent: bool | None
    strict: bool | None
    coalesce: bool | None
    include_body: bool | None

@dataclass(slots=True)
class LogRequest:
    meta: RequestMeta
    workspace_cwd: str | None
    operands: list[str]
    explicit_pathspecs: list[str]
    options: LogOptions | None
    tagged: bool | None

@dataclass(slots=True)
class LogEntryMember:
    member_id: str
    member_path: str
    source_kind: SourceKind | None
    commit: str
    parents: list[str]

@dataclass(slots=True)
class LogMergeProvenance:
    kind: LogMergeKind
    gwz_commit_id: str | None

@dataclass(slots=True)
class LogEntry:
    members: list[LogEntryMember]
    provenance: LogMergeProvenance
    author: GitObjectIdentity
    committer: GitObjectIdentity
    subject: str
    body: str | None
    ordering_timestamp_ms: int | None
    author_timestamp_seconds: int
    committer_timestamp_seconds: int
    ordering_timestamp_seconds: int
    lossy: bool | None

@dataclass(slots=True)
class LogDegradation:
    member_id: str
    member_path: str
    source_kind: SourceKind | None
    reason: LogDegradationReason
    operand: str | None
    message: str | None

@dataclass(slots=True)
class LogOutputRecord:
    kind: LogOutputRecordKind
    entry: LogEntry | None
    degradation: LogDegradation | None

@dataclass(slots=True)
class LogOutputLogRef:
    log_id: str

@dataclass(slots=True)
class LogResponse:
    response: ResponseEnvelope
    output: LogOutputLogRef


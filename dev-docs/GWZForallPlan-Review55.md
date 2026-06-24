# GWZ Forall Plan Review 55

Status: review of `dev-docs/GWZForallPlan.md`
Date: 2026-06-24
Reviewer: Codex

Verdict: revise before implementation.

The plan is substantially closer to the current codebase than the earlier shape:
it correctly identifies the parse/execute/render split, makes `ls` a real core
operation, and keeps command execution in the local CLI. The remaining problems
are mostly places where the plan still crosses an existing boundary or leaves a
machine-output/runtime edge case unspecified.

## Findings

### P1: `gwz ls --unmaterialized` cannot use `resolve_locked_selection`

Reference: `dev-docs/GWZForallPlan.md:98-107`,
`src/workspace_ops/handle_materialize.rs:387-399`,
`src/workspace_ops/push_member.rs:145-202`

The plan tells `handle_ls` to filter through `resolve_locked_selection`, then
also says `--unmaterialized` should show manifest members that may have no lock
entry. Those two requirements conflict. `resolve_locked_selection` calls
`resolve_manifest_selection`, then rejects every selected member missing from
`lock.members` with `LockNotFound`.

Required plan change: `handle_ls` needs a manifest-tolerant selection path. Use
`resolve_manifest_selection` directly, or add a new helper with the same active
member/id/path validation but no lock-presence requirement. After that, join the
selected manifest members to optional lock entries and compute:

- `materialized = lock_entry.materialized == Some(true)` when a lock entry
  exists;
- `materialized = false` when the lock entry is missing;
- default output filters to materialized entries;
- `include_unmaterialized` keeps selected active manifest entries even when the
  lock entry is absent.

The plan should also define the explicit-selection behavior for
`gwz --member <unmaterialized> ls` without `--unmaterialized`: either return an
empty listing because the default filter is materialized-only, or return a clear
"selected member is not materialized" error. Do not leave it to the helper.

### P1: The proposed `CliResponse::listing` path discards the real `ls` envelope

Reference: `dev-docs/GWZForallPlan.md:63-72`,
`dev-docs/GWZForallPlan.md:111-119`,
`gwz-cli/src/append_branch_summary.rs:10-49`

The plan makes `gwz ls` a proper core op with `LsResponse{response, members}`,
but the CLI execution step returns
`CliResponse::listing(ArtifactListing::Members(response.members))`. The current
`CliResponse::listing` constructor fabricates a trivial `Status` envelope with
empty request/schema/action fields. That is acceptable for the existing
CLI-side tag/snapshot listing shortcut, but wrong for a real `ActionKind::Ls`
operation.

If implemented as written, `gwz ls` would lose the `LsResponse.response`
metadata, including `request_id`, `schema_version`, `operation_id`,
`ActionKind::Ls`, attribution, and any future envelope-level status/errors. It
would also make `exit_code_for_response(&response.envelope)` reason about a fake
success envelope.

Required plan change: add a response shape that preserves both the real envelope
and the listing payload, for example:

```text
CliResponse::listing_with_envelope(response.response, ArtifactListing::Members(...))
```

Then decide what JSON mode means for `ls`: either keep the existing listing JSON
shape but include/retain envelope metadata, or render a normal response-shaped
JSON with `members` in a dedicated listing field. The current `listing_json`
path is not enough for a protocol-backed operation.

### P1: `forall` inherited stdio breaks `--json` and `--jsonl`

Reference: `dev-docs/GWZForallPlan.md:80`,
`dev-docs/GWZForallPlan.md:140-145`,
`dev-docs/GWZForallPlan.md:157-160`,
`gwz-cli/src/main.rs:103-106`,
`gwz-cli/src/append_branch_summary.rs:584-605`

The plan says child commands inherit stdio and `render_response` prints only the
trailing summary. That works for human output, but it invalidates the CLI's
machine-output contract. In `--json`, child stdout will be interleaved before
the final JSON response printed by `main.rs`. In `--jsonl`, child stdout will be
interleaved with the response/event/result JSON lines. A command as simple as
`gwz --json forall -- echo hi` would no longer emit parseable JSON.

Required plan change: choose one v0 policy:

- reject `--json` and `--jsonl` for `forall` while stdio is inherited;
- or capture/group child stdout/stderr in machine modes and include structured
  output references or payloads;
- or route child stdout away from stdout in machine modes, with a documented
  compatibility tradeoff.

The first option is the smallest and matches the plan's "stream live" goal. Add
parse/validation tests proving machine modes are rejected or an end-to-end test
proving emitted stdout remains valid JSON/JSONL.

### P1: Unknown `projects` cannot be a parse error

Reference: `dev-docs/GWZForallPlan.md:26-37`,
`dev-docs/GWZForallPlan.md:151-162`,
`dev-docs/GWZForallPlan.md:164-166`

The plan correctly says parse has no filesystem/backend access and member
resolution happens in execute. It later asks for an "unknown-project" parse test
and a DoD where an unknown project errors at parse. That cannot be correct: the
CLI parser can validate only the argv shape, not whether `app` or `repos/app`
exists in the manifest.

Required plan change: move unknown-project validation to the execute phase after
`handle_ls(...)` returns the member list. Parse tests should cover only:

- argv after `--`;
- `-c`;
- mutually exclusive argv/`-c`;
- no-command error;
- positional `projects` preservation.

Unknown project tests should be execute tests with a fixture workspace.

### P1: `ExecResult{name, exit_code}` cannot represent real process outcomes

Reference: `dev-docs/GWZForallPlan.md:128-132`,
`dev-docs/GWZForallPlan.md:140-145`

`std::process::ExitStatus::code()` is optional on Unix because a process can be
terminated by signal. `Command::new(...).status()` can also fail before a child
process exists, such as when the executable is missing in argv mode or the cwd
is invalid. The proposed `ExecResult{name, exit_code}` with a non-optional exit
code cannot represent either case.

Required plan change: make the process result explicit. One workable shape:

```text
ExecResult {
  id,
  path,
  exit_code?: INT,
  signal?: INT,
  spawn_error?: STR,
}
```

If the plan intentionally wants shell-like coercion, define it directly, such as
"spawn error in argv mode is recorded as exit code 127." Otherwise implementers
will invent incompatible mappings. The response envelope should also include
per-member errors for spawn failures so JSON consumers do not have to infer
failure solely from a missing/odd exit code.

### P2: "member name" is undefined and not present in `MemberEntry`

Reference: `dev-docs/GWZForallPlan.md:13`,
`dev-docs/GWZForallPlan.md:78`,
`dev-docs/GWZForallPlan.md:84`,
`dev-docs/GWZForallPlan.md:128`,
`dev-docs/GWZForallPlan.md:141-145`,
`gwz-cli/src/progress_detail.rs:154-159`

The protocol entry is `MemberEntry{id, path, abspath, materialized}`, but the
behavior talks about member "name" for `{@}`, `GWZ_MEMBER_NAME`, banners,
summaries, and `ExecResult{name, exit_code}`. Existing GWZ member ids look like
`mem_app`; paths look like `repos/app`; the CLI also has a display-only
`member_short_name(path)` helper that would produce `app`.

Those are three different identifiers with different collision and stability
properties. The plan needs to pick one.

Recommended plan change:

- Use `id` and `path` for machine semantics and matching.
- Add `GWZ_MEMBER_ID` and keep `GWZ_MEMBER_PATH`.
- If west-style "name" is desired, add an explicit `display_name` or
  `short_name` field derived from path, document that it is display-only, and
  test duplicate short names.
- Define whether `{@}` substitutes id, path, or display label.

Without this, `gwz forall repos/app -- ...`, banners, failure summaries, and env
vars can all refer to different things.

### P2: CLI-side `Forall` envelope stamping needs a concrete public path

Reference: `dev-docs/GWZForallPlan.md:131-134`,
`src/workspace_ops/handle_create_repo.rs:417-432`,
`src/operation/push_event.rs:83-118`

The plan says `ExecResponse` keeps a standard `ResponseEnvelope` and that the
executor stamps it CLI-side with `ActionKind::Forall`. That is reasonable, but
the existing envelope helpers are not directly available to `gwz-cli`:
`response_envelope` is `pub(crate)` inside `workspace_ops`, and
`OperationContext::from_meta` is also crate-internal.

Required plan change: spell out one of these implementation paths:

- `gwz-cli` manually constructs `ResponseEnvelope` from `ExecRequest.meta`,
  generated `ActionKind::Forall`, the new operation id, result-derived aggregate
  status, and converted attribution;
- or `gwz-core` exposes a small public helper for constructing response
  envelopes from `RequestMeta` and `ActionKind`;
- or `Forall` becomes an `OperationRequest` only for context construction, while
  still not being registered as a service method.

The first option is smallest but duplicates envelope construction details. The
second is cleaner if more CLI-local operations are expected.

### P2: The plan conflicts with the existing `gwz run` design doc

Reference: `dev-docs/GWZForallPlan.md:1-20`,
`gwz-cli/dev-docs/GwzMemberGitSpec.md:170-239`

`GwzMemberGitSpec.md` already describes this escape hatch as `gwz run`, with
selection, `--jobs`, per-member exit codes, and shell execution. The new plan
chooses `gwz forall` and different v0 behavior. That may be the right product
decision, but the plan should explicitly supersede or reconcile the older spec.

Required plan change: add a naming/compatibility decision:

- `forall` replaces the planned `run`;
- `run` remains the canonical verb and `forall` is an alias;
- or both exist with different semantics.

Also clarify how existing global `--jobs` and `--partial` apply. The plan says
sequential and continue-through-all, while the older spec tied these to jobs and
partial policy.

## Implementation Notes

- New test files in `gwz-core/src/workspace_ops/tests/` and
  `gwz-cli/src/tests/` must be registered in their local `tests/mod.rs`; this
  repo does not auto-discover the numbered `gXX.rs` modules.
- Protocol changes need both `protocol/gwz.taut.py` and regenerated
  `src/protocol/generated.rs` plus corpus updates. The plan says this, but it is
  worth keeping as a phase gate because `Ls`, `MemberEntry`, `Exec*`, and
  `ActionKind::Forall` all touch generated code.
- Add one end-to-end test for `main`-level behavior if possible: `forall` is the
  first command whose primary output is produced before `render_response`, so
  unit tests around `run_forall` alone will not catch stdout/JSON collisions.

## Recommended Plan Edits

1. Replace `resolve_locked_selection` in `handle_ls` with a manifest-tolerant
   selection helper and define selected-but-unmaterialized behavior.
2. Preserve the real `LsResponse.response` envelope in the CLI listing response.
3. Decide and test the `forall` policy for `--json`/`--jsonl` before streaming
   child stdout.
4. Move unknown-project validation out of parse tests and into execute tests.
5. Expand `ExecResult` beyond a mandatory `exit_code`.
6. Define member `name` versus `id` versus `path`.
7. Specify how the CLI constructs a standard `Forall` response envelope.
8. Reconcile `forall` with the existing `gwz run` spec and global `--jobs` /
   `--partial` semantics.

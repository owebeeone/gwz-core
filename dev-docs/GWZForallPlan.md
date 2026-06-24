# GWZ Forall + Ls — `gwz ls` (list members) & `gwz forall` (run-in-each) (Plan)

Status: **proposed** (2026-06-24) — design converged with Gianni; **revised after two reviews**
(a codebase review + Codex Review55): the parse/execute split, envelope preservation, machine-output
policy, process-result modelling, member identity, and the prior `gwz run` spec are addressed below.
Owner: Gianni. `GWZDesign.md` stays authoritative; companion to `GWZTagPlan.md` / `GWZAddPlan.md`.

## 1. Goal & shape

Two CLI verbs for "do something across every member repo," replacing the hand-rolled
`for i in repoa repob; do (cd $i; echo "===$i==="; <cmd>); done`:

- **`gwz ls`** — list the workspace's members (name + location). Read-only; manifest + lock only,
  **no git**. Default = absolute paths; `--local` = relative.
- **`gwz forall [projects…] -- <cmd> [args…]`** (or `-c "<shell>"`) — run a command in each
  member, modelled on `west forall` / `repo forall`. **CLI-dispatched; gwz-core never executes.**

Prior art we copy: west/`repo forall` = shell-string `-c` + per-project env vars + banner;
vcstool `custom` = argv + parallel. We take both: argv default (portable) + `-c` (shell), env
vars over a `{@}` token, banner, sequential.

### 1.1 How the CLI pipeline actually works (the frame both phases must fit)

Verified against the code — every verb flows through three fixed stages:

1. **Parse (no I/O):** `Cli::command_request` (gwz-cli `clirequest.rs`) turns clap args into a
   `CliRequest` enum value. **No backend, no filesystem, returns only a `CliRequest`.**
2. **Execute (I/O here):** `execute_invocation` (gwz-cli `globalargs.rs`) matches `CliRequest`
   exhaustively, constructs the `Git2Backend`/`EventSink`, calls the relevant `handle_*` (or, for
   reads like `ListSnapshots`, does the read inline), and returns one buffered `CliResponse`.
3. **Render + exit:** `main.rs` calls `render_response(&response)` once, then
   `exit_code_for_response(&response.envelope)` — the process exit code is derived **solely from
   the envelope's `AggregateStatus`** (`Failed`/`Partial` → non-zero).

Consequences baked into the phases below: a new verb needs **both** a `CliRequest` variant **and**
an `execute_invocation` arm (the match is exhaustive). Member resolution and process spawning
happen in **execute**, never in parse. (`status` is the model for a real op through this pipeline:
parse → `CliRequest::Status`; execute → `handle_status` → `CliResponse`. `ListSnapshots` shows the
same parse/execute split but as a CLI-side read — useful for the mechanics, not the architecture to copy.)

### 1.2 Protocol placement & why it's still safe

We punt the taut module system (`TautModules.md`); the exec ("run a command") message lives in
gwz-core's `gwz.taut.py` for now. The worry: does defining a "run a command" message inside core's
protocol let someone — eventually a remote grazel client — make gwz-core run commands? **No**, for
three plain reasons:

1. **Nothing in core acts on the message.** It's just a data type. There is no `handle_exec`, and
   no code path reads an exec message and runs anything — and (per the review) gwz-core has no
   central "receive a request → call its handler" dispatcher at all, so the message can't even be
   handed in to be executed. The only thing that acts on it is the CLI, locally.
2. **It's not a callable endpoint.** In taut a message is either plain data or wired up as an RPC
   method a client can invoke. `status` is wired as a method; the exec message is left as plain
   data on purpose, so there is no endpoint for anyone — local or remote — to call.
3. **There's no server to attack yet.** grazel/remote access does not exist in the tree today, so
   nothing is exposed. *When* that server is built it MUST refuse exec (and anything that isn't a
   safe read like `ls`) — a requirement for then, not a protection that exists now.

In one line: the exec message is a type with no code behind it and no remote endpoint, so nothing
can make core run a command through it — only the local CLI does. A marker comment in the IR flags
it CLI-local and to be moved into a gwz-cli-owned protocol once the taut module system lands.

### 1.3 `gwz ls` is a gwz-core op (`LsRequest`/`LsResponse` + `handle_ls`)

Per the design: operations are gwz-core protocol ops with handlers, and `gwz ls` talks to gwz-core
and gets a response. So `ls` is a typed `LsRequest`/`LsResponse` **service method** with a real
`handle_ls` in gwz-core — the same shape as `status`, and callable by grazel over the wire. The
review flagged that this needs an `ActionKind::Ls` (the IR enum + the mirrored model enum + its
`From` impl + an `OperationRequest::Ls` context arm + regen); that is the cost of doing it the
documented way — budgeted into Phase 1.1, not a reason to bypass the protocol. `forall` resolves
its members by calling this op, not a side channel. (`ListSnapshots` is a CLI-side read; that's a
pre-existing shortcut, not a precedent to extend.)

### 1.4 Behaviour decisions (locked)

- **forall syntax:** `gwz forall [projects…] -- argv…` (portable) **or** `gwz forall [projects…] -c "string"` (shell). `--` required before argv (clap `#[arg(last = true)]`); `-c` needs none. Exactly one form; neither → error "no command (use `--` or `-c`)" (validated in parse).
- **projects** match member `id` **or** relative `path`; unknown → error; omitted → all.
- **member identity (pinned):** `MemberEntry` has `id` (`mem_app`) and `path` (`repos/app`) — the machine identifiers; the `member_short_name(path)` display helper (`app`) is a *third*, display-only thing. Machine semantics + `projects` matching use `id`/`path`; there is **no bare "name"** (`ExecResult`/banners key off `id`/`path`). `{@}` substitutes the member **`path`** in argv mode. Env: `GWZ_MEMBER_ID`, `GWZ_MEMBER_PATH`, `GWZ_MEMBER_ABSPATH`, `GWZ_ROOT` for `-c` and any child. (An explicit `short_name` field on `MemberEntry`, derived from `path`, is an easy add if wanted — document display-only + test duplicate short names.)
- **shell:** `sh -c` (Unix) / `cmd /C` (Windows). String not portable across them (documented, like west).
- **output:** inherit stdio (live stream); banner `=== <member-path> ===` to **stderr** as each member runs; `--no-banner` to silence. forall already streams, so the summary render must **not** re-print member output.
- **machine modes:** streamed child stdout would corrupt `--json`/`--jsonl` (it interleaves with / precedes the response JSON `main.rs` prints). **v0 policy: reject `--json`/`--jsonl` for `forall`** (smallest correct option; matches "stream live"). Capture-and-group for machine modes is deferred with `-j`. Test: machine modes rejected, **and** a `main`-level e2e proving non-machine stdout stays clean.
- **failure/exit:** **stop at the first failing member by default**; the existing global `--partial` continues through the rest (sets `ExecRequest.continue_on_fail`). Either way the `CliResponse` envelope carries `AggregateStatus` = `Failed`/`Partial` when any member failed, so `exit_code_for_response` yields non-zero; trailing summary `2/5 failed: …` (`(stopped after <member>)` when not `--partial`). cwd = member abspath.
- **members:** materialized only by default (so `cd` can't fail); root excluded. `gwz ls --unmaterialized` also shows configured-but-unmaterialized members. **(`--all` is taken — it's the global "select all members" flag — so the toggle is `--unmaterialized`, not `--all`.)**
- **selection:** both verbs use the existing global `--member`/`--member-path`/`-A` (`meta.selection`); **no** redundant request-level `selection` field. (forall *additionally* narrows by its positional `[projects…]`.)
- **exec result (per member):** `ExecResult{id, path, exit_code?, signal?, spawn_error?}` — `exit_code` is **optional** (`ExitStatus::code()` is `None` for signal-killed processes on Unix → `signal`), and `spawn_error` records failures *before* a process exists (missing executable in argv mode, bad cwd). Summary coercion is explicit: **a spawn failure counts as failed and shows as exit 127**. **No stdout field** (streamed); the envelope carries per-member errors for spawn failures so JSON consumers don't infer failure from an odd/missing code. Request self-contained (members + command) so it's remote-ready.
- **sequential** now; parallel `-j` deferred.

## Phase 1 — `gwz ls` (foundational; forall reuses its helper)

**1.1 Protocol — Ls op + `MemberEntry` + `ActionKind::Ls`** · gwz-core · ~140 LOC
- `gwz.taut.py`: `MemberEntry{id, path, abspath, materialized}` (reused by `ExecRequest` in 2.1);
  `LsRequest{meta, include_unmaterialized?}`; `LsResponse{response, members: List(Ref.MemberEntry)}`;
  add `ls` to the `ActionKind` enum; register `ls` as a `service` `method` (like `status`).
  Selection rides in `meta.selection` — **no** request-level selection field.
- Mirror the new `ActionKind::Ls` in the model enum (`operation/push_event.rs`) + its
  `From<ActionKind>` impl, and add an `OperationRequest::Ls` arm to `context()`.
- Regen `src/protocol/generated.rs` + corpus; corpus clean.

**1.2 Core handler `handle_ls`** · gwz-core `workspace_ops` · ~180 LOC + tests
- `pub fn handle_ls(start, request: LsRequest, operation_id) -> ModelResult<LsResponse>` — the real
  op. Resolve root (`workspace_ops::resolve_workspace_root`, the pub one `ListSnapshots` uses) +
  `artifact::read_manifest`/`read_lock`; stamp the envelope via `OperationRequest::Ls(...).context()`
  (`ActionKind::Ls`). **No git, no `GitBackend`.**
- **Selection must be manifest-tolerant, not lock-gated.** `resolve_locked_selection` rejects any
  selected member missing from the lock (`LockNotFound`) — but `--unmaterialized` is exactly that
  case. Use `resolve_manifest_selection` directly (or a new helper with the same active/id/path
  validation but no lock-presence check), then join to *optional* lock entries. Explicit selection of
  an unmaterialized member **without** `--unmaterialized` returns a clear "selected member is not
  materialized" error (don't silently drop it) — decided in `handle_ls`, not the helper.
- Join: iterate **manifest** members; for each look up the **lock** entry by id;
  `materialized = lock entry present && member.materialized == Some(true)` (lock flag is
  `Option<bool>`; an unmaterialized member may have **no** lock entry). Default keeps only
  materialized; `include_unmaterialized` keeps all. `abspath = root.join(path)`.
- Tests (new g-file): abspath correct, materialized join + `Option` semantics, `include_unmaterialized`,
  selection scoping, empty workspace → empty list.

**1.3 CLI `gwz ls`** · gwz-cli · ~180 LOC + tests
- `LsArgs { --local, --unmaterialized }`; selection via global `--member`/`-A`/`--member-path`/`--root`.
- Parse: `command_request` → `CliRequest::Ls { local }` (new enum variant); `--unmaterialized` →
  `LsRequest.include_unmaterialized`.
- Execute: new `execute_invocation` arm → `handle_ls(start, LsRequest{meta, include_unmaterialized}, op_id)`
  → a `CliResponse` that **preserves the real `LsResponse.response` envelope**. The existing
  `CliResponse::listing` fabricates a trivial `Status` envelope — wrong for a real `ActionKind::Ls`
  op (it would drop `request_id`/`schema_version`/`operation_id`/`ActionKind::Ls`/attribution and make
  `exit_code_for_response` reason about a fake envelope). Add
  `CliResponse::listing_with_envelope(response.response, ArtifactListing::Members(response.members))`
  (or equivalent) carrying both.
- Render: **new `ArtifactListing::Members` variant** + arms in `render_listing_text` (one path per
  line; absolute, or relative `path` with `--local`). For **JSON mode**, render a normal
  response-shaped JSON (the real envelope) with `members` in a dedicated listing field — not the
  trivial-envelope `listing_json` shape. (The existing listing JSON is hand-rolled per variant, so
  this is real wiring, not free.)
- Parse tests (`ls`, `ls --local`, `ls --unmaterialized`, selection) + a render test.

**DoD P1:** `gwz ls` lists members; `for i in $(gwz ls); do (cd $i; …); done` works; `--local`
gives relative names; `--root /x` re-roots; `--unmaterialized` adds uncloned members; JSON mode emits them.

## Phase 2 — `gwz forall` (depends on Phase 1's `MemberEntry` + `handle_ls`)

**2.1 Protocol — exec messages (handler-less in core)** · gwz-core · ~110 LOC
- `gwz.taut.py`: `ExecMode = Enum(argv, shell)`;
  `ExecResult{id, path, exit_code?, signal?, spawn_error?}` (per §1.4 — optional code, signal, spawn error);
  `ExecRequest{meta, mode, command: List(STR), members: List(Ref.MemberEntry), continue_on_fail?}`
  (`command` = argv, or a 1-elem shell string in shell mode);
  `ExecResponse{response: ResponseEnvelope, results: List(Ref.ExecResult)}` — a **standard envelope
  like every other response** (carries `AggregateStatus` + per-member errors), keeping it remote-ready.
  Add `forall` to the `ActionKind` enum (+ mirror in the model enum / `From` impl).
- **Envelope construction (concrete path):** `response_envelope` is `pub(crate)` in `workspace_ops`
  and `OperationContext::from_meta` is crate-internal, so gwz-cli can't reuse them. gwz-core
  **exposes a small public helper** — `pub fn response_envelope_for(meta, action, operation_id, status, errors) -> ResponseEnvelope` — and the forall executor stamps the envelope with it (cleaner than
  hand-rolling envelope internals in the CLI, reusable by future CLI-local ops). Still **no gwz-core
  handler**; `ExecRequest`/`ExecResponse` stay bare `Msg`s, not service methods.
- `ExecRequest`/`ExecResponse` are **bare `Msg`s — NOT a service `method`**, so they're off the RPC
  surface (consistent with §1.2). Marker comment: *CLI-local; gwz-core MUST NOT handle these;
  relocate to a gwz-cli IR when taut modules land.*
- Regen + corpus.

**2.2 CLI executor `run_forall`** · gwz-cli · ~240 LOC + tests
- `run_forall(req: &ExecRequest, no_banner: bool) -> ExecResponse`: per member → cwd = abspath; set
  `GWZ_MEMBER_ID`/`GWZ_MEMBER_PATH`/`GWZ_MEMBER_ABSPATH`/`GWZ_ROOT`; **argv** mode substitutes
  `{@}`→`path` then `Command::new(argv[0]).args(rest)`; **shell** mode `sh -c`/`cmd /C` with the
  string; **inherit stdio** (live stream); banner → stderr per member unless `--no-banner`;
  **on a member failure, stop unless `req.continue_on_fail`** (the `--partial` global) — remaining
  members left unrun; collect `ExecResult{id, path, exit_code?, signal?, spawn_error?}` (map
  `ExitStatus`; argv-mode spawn failure → `spawn_error` + exit 127 in the summary).
- Tests over a 2-member fixture (argv `git rev-parse HEAD`; shell `echo $GWZ_MEMBER_PATH`): argv +
  shell, `{@}` + env substitution, a **spawn failure** (missing binary) and a **non-zero exit** both →
  non-zero aggregate; **default stops at the first failure, `continue_on_fail` runs the rest**; banner emitted.

**2.3 CLI `gwz forall` verb + exit wiring** · gwz-cli · ~220 LOC + tests
- `ForallArgs { projects: Vec<String>, #[arg(last = true)] command: Vec<String>, -c/--command-string: Option<String>, --no-banner }`.
- Parse: `command_request` → `CliRequest::Forall { projects, mode, command, no_banner }` (validate
  exactly-one-of argv/`-c`, "no command" error, and **reject `--json`/`--jsonl`** — all argv-shape, no I/O).
- Execute: new `execute_invocation` arm → `handle_ls(...)` for the member list → filter by `projects`
  (`id`/`path`, **error on unknown here — parse can't see the manifest**) → build `ExecRequest`
  (`continue_on_fail` from the global `--partial`) → `run_forall` (streams live) → return a
  `CliResponse` whose **envelope `AggregateStatus`** is
  `Ok`/`Partial`/`Failed` per results, so `exit_code_for_response` gives the right code.
- Render: forall already streamed member output, so its `CliResponse` renders **only** the
  `N/M failed: …` summary. Model it as a dedicated `CliResponse` summary variant that
  `render_response` handles like the existing envelope/listing variants — not a verb-special-case
  hack inside `render_response`.
- Tests — **parse:** argv after `--`, `-c`, mutually-exclusive argv/`-c`, no-command error,
  positional `projects` preserved, `--json`/`--jsonl` rejected. **Execute (fixture workspace):**
  unknown-project error, exit code + summary. **`main`-level e2e:** non-machine stdout stays clean.

**DoD P2:** `gwz forall -- git status` and `gwz forall -c 'echo $GWZ_MEMBER_PATH; git log -1'` run
in each member with live output + per-member banners + a summary; a failing member (or spawn failure)
**stops the run with a non-zero exit, while `--partial` continues through the rest**; an unknown
project errors in **execute**; `--json`/`--jsonl` is rejected.

## Implementation notes
- New numbered test modules (`gXX.rs`) in `gwz-core/src/workspace_ops/tests/` and `gwz-cli/src/tests/`
  must be registered in the local `tests/mod.rs` — no auto-discovery.
- Protocol changes touch `protocol/gwz.taut.py` **and** regenerated `src/protocol/generated.rs` +
  corpus — keep regen as a phase gate (`Ls`, `MemberEntry`, `Exec*`, `ActionKind::{Ls,Forall}` all hit
  generated code).
- forall is the first verb whose primary output precedes `render_response`, so a `main`-level e2e is
  required — `run_forall` unit tests alone won't catch stdout/JSON collisions.

## Reconciliation with the existing `gwz run` spec (RESOLVED)
`gwz-cli/dev-docs/GwzMemberGitSpec.md` §`gwz run` specs this escape hatch as `gwz run` with the runner
in gwz-core. Decisions (Gianni, 2026-06-24):
- **Name:** `gwz forall` **replaces** `gwz run` — the `gwz run` section of that spec is superseded.
- **Executor:** CLI-only (gwz-core never executes), overriding the spec's in-core `GitCliRunner`.
- **Parallelism:** **sequential** for v0; `--jobs`/parallel deferred (keeps streamed output clean).
- **Failure:** **honor the existing `--partial`** — stop at the first failing member by default,
  `--partial` continues. (Matches push/pull/materialize; replaces this plan's earlier continue-by-default.)

## Out of scope (later)
- Parallel `-j` (switch from streamed stdio to capture-and-group).
- Remote execution / routing the exec message off-box (needs a secured channel **and** likely the
  module system to move exec out of core first).
- The TautModules migration.

## Risks / open
- **[consistency]** forall's positional `[projects…]` vs gwz's `--member`/`-A` everywhere else
  (leaning positional, matches west; confirm).
- `cmd /C` quoting of the `-c` string on Windows — document non-portability; argv mode stays portable.
- Banner on **stderr** vs stdout — confirm it won't surprise `2>` redirptors.
- abspath in a core message is **not novel** — `StageRequest.cwd` is already an absolute path on a
  live core request. The real remote concern is that a remote executor resolves its own paths
  (already noted), not abspath-in-protocol per se.

## Appendix — corrections applied from the codebase review
- Member resolution + process spawning moved out of `command_request` (parse-only) into
  `execute_invocation` (I/O stage); both verbs get explicit `CliRequest` variants + execute arms.
- Removed the fictional "core dispatch returns `UnsupportedOperation`" guardrail — no central
  dispatch exists; the boundary is inertness-by-absence; deny-by-default demoted to a future
  requirement.
- `ls` is a proper gwz-core op (`LsRequest`/`LsResponse` + `handle_ls` + `ActionKind::Ls`), per the
  rule that operations go through gwz-core's protocol — the earlier "CLI-side read to dodge the
  `ActionKind` churn" recommendation was wrong and is reverted. `--all` clash resolved →
  `--unmaterialized`; dropped the redundant request-level `selection`; spelled out the manifest⨝lock
  join + `Option<bool> materialized`.
- Added the missing `ArtifactListing::Members` render wiring and the forall exit-code/summary path
  through `AggregateStatus` + a dedicated `CliResponse` summary variant.
- **(2nd-pass shortcut audit)** `ExecResponse` keeps a standard `ResponseEnvelope` (+ `ActionKind::Forall`,
  stamped CLI-side) like every other response — the earlier "drop the envelope for simplicity" note
  was the same trade-design-for-less-work shortcut and is reverted; the envelope also keeps it
  remote-ready. Fixed a stale `list_members` reference (it's `handle_ls`) and reworded §1.1 so
  `ListSnapshots` isn't framed as a precedent to copy.
- Help modules (`*_long`/`*_after`) are optional polish — there is **no** help-completeness test
  that would fail without them.

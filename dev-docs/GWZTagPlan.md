# GWZ Tag Redesign — `gwz tag` = full git-tag management; `gwz snapshot` = workspace state (Plan)

Status: **implemented** (2026-06-24) — P1–P4 landed: local primitives + protocol, op-dispatch
handler, `materialize --tag` re-mean, remote push/fetch/list/delete, full CLI, and the old
`TagArtifact` removed. `gwz tag` operates on real `refs/tags/*` (an earlier `gwztag/` namespace
was dropped so it mirrors `git tag` and surfaces the repos' existing tags).
Owner: Gianni. `GWZDesign.md` stays authoritative; companion to `GWZAddPlan.md`.

**As-built deltas from the spec below** (kept for history; the code is the source of truth):
- Push-all is spelled `gwz tag --push` (no name), not `--push --all`; a `<name>` pushes just that
  tag. The `TagRequest.all` protocol field is unused/reserved (the CLI never sets it).
- `gwz tag --fetch` fetches **all** tags from the remote (no per-name fetch).
- `create` is idempotent (members already carrying the tag are skipped) and a signed tag (`-s`)
  requires a message (`-m`). The action flags `--list/--delete/--push/--fetch` are mutually
  exclusive. `materialize --tag` restores only the members that carry the tag (others skipped).

## 1. Goal & shape

Today `gwz tag` and `gwz snapshot` both write a near-identical workspace-composition artifact
(`gwz.conf/tags/*.yaml` vs `snapshots/*.yaml`) — pure redundancy. Split them, and make `gwz tag`
a **complete git-tag toolkit** (not a partial one), fanned out across the workspace's member
repos — mirroring `gwz commit` (= fan-out `git commit`):

- **`gwz snapshot`** — *unchanged*. The single workspace-composition concept: record
  `{member → commit}` into `gwz.conf/snapshots/`, restore via `gwz materialize --snapshot`.
  gwz-managed, local.
- **`gwz tag …`** — real `git tag` management across selected member repos. The **whole set**:
  - **create** (lightweight / annotated `-m` / signed `-s`)
  - **list local** and **list remote**
  - **fetch** remote tags (one or all)
  - **push** a specific tag (or all)
  - **delete** local and remote
  A tag name is workspace-wide: `v1` means `refs/tags/v1` in each member.
- The `TagArtifact` / `gwz.conf/tags/` concept is **removed** (it was snapshot-by-another-name).

Net: `snapshot` = gwz workspace state (local, gwz-managed); `tag` = git refs across the repos
(git-native, shareable), with the full lifecycle managed in one place.

## 2. Decisions (direction approved; confirm before Phase 2)

| D | Decision | Choice |
|---|----------|--------|
| D1 | Tag flavors | Lightweight (bare), annotated (`-m <msg>`), signed (`-s`, may use the AD1 git-CLI fallback for GPG). |
| D2 | Tag the root repo too? | **Selected members + root** (consistent with `gwz commit`). *Alt: members-only.* |
| D3 | `gwz materialize --tag <name>` | **Re-mean** to "checkout each member to git tag `<name>`". |
| D4 | CLI surface | **Action flags on `gwz tag`** (git-aligned — see §3). *Alt: subcommands `gwz tag create/list/push/…`.* |
| D5 | Remote selection | Default to each member's configured remote; `--remote <name>` (`-r`) overrides — reuse the existing `--remote` plumbing. |
| D6 | Tag missing on a member | Push/delete-remote of a tag a member doesn't have → **skip that member** (reported), not an error. |

## 3. CLI surface (D4 — recommended: flags)

```
gwz tag <name> [-m <msg>] [-s]      create (lightweight | annotated | signed)
gwz tag                             list local tags (aggregated across members)
gwz tag --list --remote [-r <rmt>]  list remote tags
gwz tag --fetch [<name>] [-r <rmt>] fetch remote tags (all, or one) into members
gwz tag --push <name> [-r <rmt>]    push a tag to the remote (per member that has it)
gwz tag --push --all [-r <rmt>]     push all tags
gwz tag --delete <name> [--remote]  delete a tag locally (or on the remote)
```
Action flags `--fetch` / `--push` / `--delete` are mutually exclusive (clap `ArgGroup`); with
none, a `<name>` creates and no name lists. Member scope via the global `--member` / `--all`.

## 4. Backend primitives (AD1 — behind the git boundary, self-verifying, per-primitive CLI fallback)
Local: `tag_create`, `tag_list`, `tag_delete`. Remote: `tag_push`, `tag_fetch`, `tag_list_remote`.
The remote three reuse the existing fetch/push + credential + transfer-progress machinery that
`gwz push` / `gwz pull` already use.

## 5. Protocol
A single `TagRequest { meta, op, name?, message?, signed?, remote?, all? }` action where
`op ∈ {create, list, fetch, push, delete}` (+ a `remote` scope flag distinguishing local vs
remote for list/delete), and a `TagResponse` carrying per-repo results (envelope, like
`commit`/`stage`) plus an optional aggregated tag list for the list ops. Regenerate
`generated.rs` + corpus. (One action keeps daemon/UI + events uniform across the toolkit.)

## 6. Phased plan
Phases are milestones; steps are single goals (aspirational < 500 LOC); foundational-first;
**∥** = independently pick-up-able.

### Phase 1 — Local-tag foundations
- **1.1 ∥ `tag_create` primitive** — `refs/tags/<name>` at HEAD; lightweight / annotated / signed (D1). Contract + unit tests.
- **1.2 ∥ `tag_list` primitive** — a repo's tag names, sorted.
- **1.3 ∥ `tag_delete` primitive** — delete a local tag ref.
- **1.4 ∥ Protocol** — `TagRequest{op,…}` (full op set) + `TagResponse`; regenerate generated.rs + corpus (drift gate green).

### Phase 2 — Local tag management (shippable milestone)
- **2.1 `handle_tag` (local ops)** — dispatch create / list-local / delete-local; fan out to selected members + root (D2); aggregate `TagResponse`. Mirror `handle_commit`/`handle_stage`. **Replaces the TagArtifact write path.** Handler tests.
- **2.2 CLI (local)** — `gwz tag <name> [-m] [-s]`, `gwz tag` (list local), `gwz tag --delete <name>`; help + parse/integration tests.
- **2.3 `gwz materialize --tag <name>`** — re-mean (D3): checkout each member to its git tag; replace the TagArtifact-read path in materialize. Tests.

### Phase 3 — Remote tag management (shippable milestone)
- **3.1 ∥ `tag_push` / `tag_fetch` / `tag_list_remote` primitives** — reuse the remote/credential/event infra; AD1 + tests.
- **3.2 `handle_tag` (remote ops)** — fetch / push (specific + `--all`) / list-remote / delete-remote across members (skip members lacking the tag, D6); emit transfer-progress events.
- **3.3 CLI (remote)** — `--fetch`, `--push [--all]`, `--list --remote`, `--delete --remote`, `-r/--remote`; help + tests.

### Phase 4 — Remove TagArtifact + docs (cleanup milestone)
- **4.1 Delete the artifact** — `TagArtifact`, `TAG_DIR`, `tag_path`, `read_tag`/`write_tag`/`list_tags`, the `gwz.tag/v0` schema + golden corpus, the `TagArtifact` protocol message; regenerate generated.rs + corpus.
- **4.2 Docs** — `GWZDesign.md` (tag = git tag toolkit; snapshot = workspace), README command list, mark this plan implemented; full `cargo test` + clippy + `corpus --check` green.

## 7. Anchors in existing code
- Verb templates: `handle_commit` / `handle_stage` (fan-out, selection, per-repo loop, response).
- Local-primitive pattern: `commit` / `stage_paths` in `src/git/gitbackend.rs` (AD1 contract).
- Remote/network + events: `gwz push` / `gwz pull` paths (credential callback, transfer progress) — the remote tag primitives build on these.
- Removal targets: `artifact::{TagArtifact, tag_path, read_tag, write_tag, list_tags, TAG_DIR}`, the tag side of the item-6 listing, the `gwz.tag/v0` schema, `materialize --tag`'s read.

## 8. Supersedes / out of scope
- **Supersedes** the *tag* side of items 6 & 7 (TagArtifact `.yaml` suffix; `list_tags` artifact listing). The snapshot side stays as built. None of it is committed yet.
- Out of scope (only the genuinely advanced): GPG signature **verification** of fetched tags; tag rename. Everything in §1's "whole set" is in-scope across Phases 1–3.

# GWZ Git capability inventory

Status: **inventory only; implementation paused** (2026-09-20). This report
compares the current checked-out GWZ CLI/core source with Git **2.52.0**. It is
not an implementation plan or a claim that a missing command should be added.
The existing [libgit2 gap inventory](GwzLibgit2Gaps.md) and [no-fallback
plan](GwzNoFallbackPlan.md) remain the context for the four known Git-process
lanes; this report broadens the view to feature completeness, including the
stale omission that `tag --delete` is another subprocess route.

Source revision baseline: workspace `d0edc9611c9866983b4e8b3c30aed0c160384da4`,
`gwz-cli` `7db07bbdefd2897c07fd0e9f550bf032bd8b1314`, and `gwz-core`
`0154d36d3412d26b0a8431ece67f64abd214f5d6`. Source links and line numbers in
this report refer to those checked-out revisions; line numbers can move after
subsequent edits.

## How the command universe was defined

The reproducible local baseline is recorded in
[GwzGitCapabilityInventory-GitCommands.txt](GwzGitCapabilityInventory-GitCommands.txt).
On this host:

| Probe | Result | Meaning |
|---|---:|---|
| `git --version` | 2.52.0 | Compatibility oracle for this inventory |
| `git --list-cmds=builtins` | 146 | Built-in command names, including helpers and aliases |
| `git --list-cmds=main` | 177 | Commands in Git's exec directory, including scripts and protocol helpers |
| `git help -a` | documented categories | User-facing porcelain, plumbing, helpers, guides, and external commands visible on this installation |

`git help -a` is the documented command universe used for the report. `main`
is a useful installed-program cross-check, but it includes internal helpers and
script commands that are not ordinary developer operations. Configured aliases
and `git-*` programs found on `PATH` are deliberately outside the stable
universe: aliases are per-user configuration and external commands are
installation-dependent. This host's `git help -a` reported `filter-repo` as an
external command; it is not treated as Git core. The compact companion retains
the exact raw lists, including command names that are not documented as normal
porcelain.

The official references used for the taxonomy and configuration semantics are
[git 2.52.0](https://git-scm.com/docs/git/2.52.0),
[git-config 2.52.0](https://git-scm.com/docs/git-config/2.52.0),
[gitcli](https://git-scm.com/docs/gitcli),
[githooks](https://git-scm.com/docs/githooks/2.52.0),
[gitattributes](https://git-scm.com/docs/gitattributes/2.52.0), and
[gitignore](https://git-scm.com/docs/gitignore/2.52.0). Git's own reference
distinguishes porcelain from plumbing and says plumbing interfaces are intended
for scripted use; that distinction matters when an obscure plumbing command is
listed as absent even though a narrow native primitive exists in GWZ.

Status labels in this report mean:

- **supported**: the product capability and material mode are exposed and
  evidence does not show a material gap for the scoped GWZ operation.
- **partial**: GWZ exposes a related operation, but important Git modes,
  options, configuration effects, or object/worktree semantics are missing.
- **backend-only**: a core/native primitive exists, but there is no direct
  public CLI or general-purpose API operation for the Git command.
- **absent**: no current GWZ operation or backend primitive was found.
- **explicitly excluded**: the design/requirements document records the scope
  exclusion or deferral.
- **uncertain**: source evidence is insufficient to claim parity; it is kept in
  the gap list rather than inferred from a command name.

“Equivalent” means a different GWZ operation supplies a workspace-scoped
semantic, not that GWZ accepts the Git spelling. A Git command is not counted as
missing merely because the CLI spelling differs; a meaningful mode gap still is.

## Current GWZ surface and scope

The current parser exposes 23 top-level commands in
[`globalargs/parser.rs`](../../gwz-cli/src/globalargs/parser.rs:249):

`auth`, `add`, `branch`, `capture`, `clone`, `commit`, `diff`, `fetch`,
`forall`, `hook`, `init`, `local`, `ls`, `log`, `materialize`, `merge`, `pull`,
`push`, `repo`, `snapshot`, `stash`, `status`, and `tag`. The parser enum is
authoritative for this count; nested surfaces are:

- `repo add|clone|create|detach|attach|sync`
  ([repo.rs](../../gwz-cli/src/clirequest/repo.rs:26));
- `local clone|list|dispose|disband`
  ([local.rs](../../gwz-cli/src/clirequest/local.rs:63));
- `hook claude-code worktree-create|worktree-remove|setup`
  ([hook.rs](../../gwz-cli/src/clirequest/hook.rs:45));
- `auth identity`, and `stash push|list|apply|pop|drop`
  ([parser.rs](../../gwz-cli/src/globalargs/parser.rs:376)); and
- global selection/transport/rendering controls including `--target`,
  `--no-target`, `--all`, `--remote`, `--identity`, `--remote-identity`,
  `--dry-run`, `--partial`, `--force`, `--sync`, `--json`, and `--jsonl`.

Every ordinary workspace Git operation resolves a selected set of repositories;
`--target` can narrow it to one member, `@root`, or `@all`. Structural commands
such as workspace/repository creation have intentional selector refusals. This
report therefore evaluates both one-repository semantics and fan-out behavior.

At the core boundary, [`GitRepository`](../src/git/gitbackend/contract.rs:25)
contains native methods for repository creation, clone/fetch/push, merge and
rebase/reset, branch and checkout, status, diff, staging, commit/tag, stash,
remote advertisement, ancestry, and selected object/ref reads. Default trait
methods that return `UnsupportedOperation` are evidence of a backend/API gap,
not evidence that a similarly named CLI command is implemented.

## User-facing porcelain inventory

The table records every main documented porcelain command from Git 2.52.0. The
“GWZ equivalent” column uses the actual public operation where one exists.

| Git command | Status | GWZ equivalent / current boundary |
|---|---|---|
| `add` | partial | `gwz add`; path-routed and `-A`, but Git add modes are not all exposed |
| `am` | absent | No mailbox-apply operation |
| `archive` | absent | No tree/archive export operation |
| `backfill` | absent | No partial-clone missing-object operation |
| `bisect` | absent | No bisect session or state machine |
| `branch` | partial | `gwz branch`; list/create/delete and a deprecated merge spelling only |
| `bundle` | absent | No bundle import/export operation |
| `checkout` | partial | `gwz materialize`, `gwz branch`, and `--switch`; no file-restore modes |
| `cherry-pick` | absent | No commit replay operation |
| `citool` | absent | No graphical Git UI integration |
| `clean` | absent | No untracked-file deletion operation |
| `clone` | partial | `gwz clone` and `gwz repo clone`; creates a workspace and applies GWZ metadata |
| `commit` | partial | `gwz commit`; workspace fan-out and `-a`, but important Git commit modes/config effects are missing or delegated |
| `describe` | absent | No human-readable object naming operation |
| `diff` | partial | `gwz diff`; repository projection and many output modes, but not every Git diff mode |
| `fetch` | partial | `gwz fetch`; selected remote and progress, with no general refspec/options surface |
| `format-patch` | absent | No email patch series export |
| `gc` | absent | `gwz merge --gc` is merge-record retention, not Git object GC |
| `grep` | absent | No repository content search |
| `init` | partial | `gwz init` and `gwz repo create`; workspace-aware initialization |
| `log` | partial | `gwz log`; cross-repository coalescing and filters, with path-filtered history still delegated to `rev-list` |
| `maintenance` | absent | No maintenance schedule/run operation |
| `merge` | partial | `gwz merge`; coordinated merge/recovery and a remote lane import |
| `mv` | absent | No Git-aware rename operation |
| `notes` | absent | No notes refs or notes editing |
| `pull` | partial | `gwz pull`; target is head or snapshot, with GWZ sync policy |
| `push` | partial | `gwz push`; captured workspace publication and dependencies, not arbitrary push modes |
| `range-diff` | absent | No range comparison |
| `rebase` | partial | Core `rebase_onto` and global `--sync rebase`; no public rebase-specific CLI controls |
| `reset` | partial | Core hard reset and global `--sync reset`; no mixed/soft/path/index modes |
| `restore` | absent | No direct index/worktree path restore |
| `revert` | absent | No revert-commit operation |
| `rm` | absent | No Git-aware remove/stage-deletion operation |
| `shortlog` | absent | No shortlog aggregation |
| `show` | absent | No general object/commit/tree display |
| `sparse-checkout` | absent | No sparse index/worktree mode |
| `stash` | partial | `gwz stash`; coordinated native stash bundle with push/list/apply/pop/drop |
| `status` | partial | `gwz status`; workspace projection and porcelain-like output, not every status mode |
| `submodule` | absent | No public submodule operation. `GWZRequirements.md` REQ-001 says v0 MUST NOT require submodules, and REQ-002 defers non-Git storage backends; these are requirements/deferrals, not an explicit exclusion of the Git command. |
| `switch` | partial | `gwz branch --switch`, `gwz materialize --switch`; no switch-specific option set |
| `tag` | partial | `gwz tag`; create/list/delete/fetch/push, but no verify/list formatting/ref-target modes |
| `worktree` | absent | No public worktree-management operation was found. REQ-002 defers bare/worktree/mirror-cache storage backends, which is narrower than excluding all `git worktree` management. |
| `gitk` / `gui` | absent | No GUI launcher or integration |
| `scalar` | absent | No Scalar repository management |

The partial status for `commit`, `tag`, and local-family merge import also has a
runtime dimension: core currently launches `git` for commit/tag and a conditional
local fetch fallback. The exact call sites are
[`repository.rs`](../src/git/gitbackend/repository.rs:291),
[`refs.rs`](../src/git/gitbackend/refs.rs:164), and
[`transport.rs`](../src/git/gitbackend/transport.rs:296). The stale four-case
document omitted `tag_delete`; it launches `git tag -d` at
[`refs.rs`](../src/git/gitbackend/refs.rs:232).

## Ancillary, foreign, and service commands

These names are all in the Git 2.52.0 documented universe. They are grouped so
server/admin and foreign-SCM utilities are visible without implying that every
internal helper is a first-priority workspace feature.

| Status | Commands |
|---|---|
| absent | `config`, `fast-export`, `fast-import`, `filter-branch`, `mergetool`, `pack-refs`, `prune`, `reflog`, `refs`, `repack`, `replace` |
| backend-only / no direct command | `remote` (core reads remotes and adds them while lifecycle operations run), `merge-tree` (in-memory merge simulation exists but is not a public object), `ls-remote` (core advertises refs for planning) |
| partial equivalents | `version` and `help`: GWZ exposes its own version/help; it does not expose Git build information or the Git manual catalog |
| absent | `annotate`, `blame`, `bugreport`, `count-objects`, `diagnose`, `difftool`, `fsck`, `instaweb`, `rerere`, `show-branch`, `verify-commit`, `verify-tag`, `whatchanged`, `gitweb` |
| absent / low priority | `archimport`, `cvsexportcommit`, `cvsimport`, `cvsserver`, `imap-send`, `p4`, `quiltimport`, `request-pull`, `send-email`, `svn` |
| absent / service or protocol | `daemon`, `http-backend`, `fetch-pack`, `send-pack`, `update-server-info`, `http-fetch`, `http-push`, `receive-pack`, `upload-archive`, `upload-pack`, `shell` |

“Backend-only” is deliberately narrower than “supported”: it identifies a
native primitive used by a GWZ workflow but no stable public operation exposing
the Git command's full behavior. For example, `ls-remote` is used to prove a
publication destination, but users cannot ask `gwz ls-remote` for arbitrary
patterns and output formats.

## Plumbing, helpers, and internal commands

### Native equivalents without a public Git command surface

The following documented plumbing commands have narrow native equivalents in
the core backend or are consumed as implementation details. They are not
general-purpose command support: `cat-file`, `checkout-index`, `commit-tree`,
`diff-files`, `diff-index`, `diff-pairs`, `diff-tree`, `for-each-ref`,
`hash-object`, `ls-files`, `ls-tree`, `merge-base`, `mktag`, `mktree`,
`read-tree`, `rev-list`, `rev-parse`, `show-ref`, `symbolic-ref`,
`update-index`, `update-ref`, and `write-tree`.

Examples of the boundary are the `diff_manifest` and `resolve_comparison`
methods ([comparison.rs](../src/git/gitbackend/comparison.rs:51)), the ref and
ancestry methods ([refs.rs](../src/git/gitbackend/refs.rs:265)), and the staging
method ([repository.rs](../src/git/gitbackend/repository.rs:205)). Path-filtered
`rev-list` remains an external process, so `rev-list` is backend-only for plain
history and a current subprocess dependency for path history.

### Absent or unverified plumbing capabilities

No public GWZ operation or verified backend parity was found for:

`apply`, `commit-graph`, `index-pack`, `merge-file`, `merge-index`,
`multi-pack-index`, `pack-objects`, `pack-redundant`, `prune-packed`, `replay`,
`unpack-objects`, `unpack-file`, `get-tar-commit-id`, `last-modified`,
`name-rev`, `show-index`, `var`, and `verify-pack`.

The internal/helper names `check-attr`, `check-ignore`, `check-mailmap`,
`check-ref-format`, `column`, `credential`, `credential-cache`,
`credential-store`, `fmt-merge-msg`, `hook`, `interpret-trailers`, `mailinfo`,
`mailsplit`, `merge-one-file`, `patch-id`, `sh-i18n`, `sh-setup`, and
`stripspace` are not public GWZ commands. Some related behavior is delegated to
libgit2 or to Git's process for commit/tag; this inventory does not infer full
helper parity from that delegation. `credential` support is specifically
partial: libgit2 credential callbacks can honor configured helpers when the
backend policy allows them, but GWZ does not expose Git's credential helper
management commands.

The remaining installed names that are implementation-only, hidden, aliases, or
protocol helpers are also unsupported as direct GWZ commands: `checkout--worker`,
`credential-cache--daemon`, `fsck-objects`, `fsmonitor--daemon`,
`merge-ours`, `merge-recursive`, `merge-recursive-ours`,
`merge-recursive-theirs`, `merge-subtree`, `pickaxe`, `remote-ext`, and
`remote-fd`. Git's `stage` is a compatibility spelling for `add`, so it is a
**partial-equivalent** through `gwz add`, inheriting the same option gaps.
Git's `init-db` similarly maps to the partial workspace-aware initialization
capability; neither alias is missing merely because its spelling differs. The remaining helper names are `submodule--helper`
and `upload-archive--writer`. The additional installed `main` names
`credential-netrc`, `credential-osxkeychain`, `difftool--helper`,
`merge-octopus`, `merge-resolve`, `remote-ftp`, `remote-ftps`, `remote-http`,
`remote-https`, `sh-i18n--envsubst`, `subtree`, and `web--browse` are likewise
not GWZ capabilities; the complete raw list, including commands already grouped
above, is in the companion file. This makes the inventory exhaustive over the
installed command names without promoting private helpers to product priorities.

## Concrete option and mode gaps in existing GWZ equivalents

This is a **first-pass major-gap inventory**, not an exhaustive comparison of
every Git flag. The rows compare Git-specific modes; global GWZ flags are a
separate layer and do not imply Git flag parity. In particular, GWZ defines
global `--dry-run`, `--force`, and `--all`: `--all` selects all workspace
members, while commit `-a` and add `-A` retain their Git-like staging meanings.
Obscure plumbing flags and output-format aliases remain explicitly uncertain
until a command-specific compatibility matrix is commissioned.

| Surface | Current GWZ behavior | Important missing or different modes |
|---|---|---|
| `add` | `gwz add [paths]` and `-A`; path routing to selected repositories | No Git-add-specific `-n/--dry-run` report, `-u/--update` distinction, `--intent-to-add`, `--refresh`, `--chmod`, interactive/patch modes, `--pathspec-from-file`, or Git's complete pathspec/global magic behavior |
| `commit` | `-m`, `-a`, GWZ marker controls; fan-out over selected repos | No `--amend`, fixup/squash, `--author`, `--date`, signoff/trailer cleanup, `--no-verify`, `--no-post-rewrite`, explicit `--gpg-sign/--no-gpg-sign`, `--allow-empty`, `--allow-empty-message`, `--template`, verbose/status, or Git commit dry-run modes. Ordinary commit currently delegates to `git commit`, while the native merge-resolution path is separate |
| `tag` | create/list/delete plus `-m`, `-s`, fetch/push | No tag sorting/pattern/column/format, `--points-at`, `--contains`, `--merged`, `--no-merged`, `--verify`, `--cleanup`, `--file`, force/update controls, or arbitrary object target. Delete still uses a Git subprocess |
| `branch` / `switch` | list/create/delete, optional `--from` and `--switch` | No branch rename/copy, force deletion, descriptions, merged/contains filters, remote/all branches, tracking/upstream configuration, sort/format/color/column, orphan/start-point modes, or path restore. Dirty-worktree policy is GWZ-owned and can refuse cases Git would resolve interactively |
| `merge` | coordinated merge with `--ff-only`, `--no-ff`, message, continue/abort/status/gc | No `--squash`, `--no-commit`, `--edit/--no-edit`, strategy/strategy-option, ours/theirs strategy, octopus, unrelated-histories, autostash, verify-signatures, signoff/signing, rerere, or merge-driver/tool controls |
| `fetch` / `pull` | named remote and GWZ sync modes; pull to head/snapshot | No Git fetch-specific arbitrary refspecs, `--multiple`, `--append`, prune/prune-tags, tag policy, depth/shallow/filter, negotiation, fetch-specific dry-run/force, `--update-head-ok`, submodule recursion, or fetch-output formatting. Global GWZ `--dry-run` plans the GWZ operation and global `--force` is a GWZ policy flag whose per-command use must be checked; neither is the Git fetch option. Pull does not expose Git's full fetch-plus-merge/rebase option matrix |
| `push` | captured selected refs/dependencies, configured remote/push URL, optional remote checks | No arbitrary refspec input, Git force/force-with-lease, delete/upstream/set-upstream, mirror/all/tags, signed/cert push, atomic, thin/no-thin, push-option, receive-pack, follow-tags, porcelain, or verify/signature modes. Global GWZ `--force` exists but `pushargs.rs:24-26` explicitly says it is not a forced push |
| `remote` | core reads configured remotes and can add one during lifecycle setup | No user-facing remote list/add/remove/rename/set-url/set-head/prune/update; no general remote config editor |
| `log` | `-n`, time, author/message regex, no-merges, first-parent, pathspecs, tag filter, body/full/color | No Git pretty formats, decorations, graph, all/branches/remotes, topo/date/author-date ordering, commit-count/merges/reflog/bisect/objects filters, notes/mailmap, encoding, raw/stat/name modes. Author/grep intentionally use Rust regex and are not Git regex syntax. Path-filtered traversal still uses one `git rev-list` per emitted step |
| `diff` | many comparison, rename, output, whitespace, prefix, binary, exit-code modes | No complete Git diff algorithm/indent/word/anchored/pickaxe/filter/irreversible-delete/submodule/textconv/external-diff family; no full `--diff-filter`, `--find-copies`, `--find-copies-harder`, `--patience`, `--histogram`, `--minimal`, `--anchored`, `--word-diff`, `--color-moved`, `--check`, or `--ita-*` surface |
| `status` | combined/per-repo, porcelain, file/branch suppression | No full short/porcelain-v2 branch/untracked/ignored/column/ ahead-behind/verbose/pathspec and submodule status modes |
| `stash` | coordinated push/list/apply/pop/drop, `-u`/`-a`/message | No pathspec-limited stash, `--patch`, `--keep-index`, `--include-untracked`/`--all` combinations matching all Git edge cases, branch creation, `stash show` patch/stat/name modes, or native stash message/index controls |
| `clone` / `init` | workspace URL clone and member clone/create | No bare/mirror/separate-git-dir/template/initial-branch/object-format/reference/alternates/shallow/filter/sparse/remote-name/branch/upload-pack/checkout controls; bare and worktree storage are explicitly deferred |
| `rebase` / `reset` | backend `rebase_onto` and hard reset selected by sync policy | No interactive/merge/rebase-merges/root/onto/fork-point/autostash/exec/strategy/signing/update-refs/rebase todo controls; no soft/mixed/keep/merge/pathspec/reset-index modes |

Direct source evidence for the current-behavior column and the global-flag
distinction is recorded here for each row (first-pass coverage):

- `add` / `commit`: [`repo.rs`](../../gwz-cli/src/clirequest/repo.rs:8) and
  [`repo.rs`](../../gwz-cli/src/clirequest/repo.rs:143) define `-A`, `-a`, and
  message/staging arguments.
- `tag`: [`nameargs.rs`](../../gwz-cli/src/nameargs.rs:42) defines the exposed
  create/list/delete/fetch/push tag arguments.
- `branch` / `switch` / `merge`: [`parser.rs`](../../gwz-cli/src/globalargs/parser.rs:429)
  and [`parser.rs`](../../gwz-cli/src/globalargs/parser.rs:470) define the
  actual branch and merge modes.
- `fetch` / `pull`: global remote and sync controls are in
  [`parser.rs`](../../gwz-cli/src/globalargs/parser.rs:156), native fetch and
  tag-fetch behavior is in [`transport.rs`](../src/git/gitbackend/transport.rs:69),
  and pull integration selects `FetchOnly`, `FfOnly`, `Merge`, `Rebase`, or
  `Reset` in [`root_lock.rs`](../src/workspace_ops/pull_head_member_preflight/root_lock.rs:45).
- `push`: the only push-specific CLI switch is `--check-remotes` in
  [`pushargs.rs`](../../gwz-cli/src/pushargs.rs:5), whose comment explicitly
  separates global `--force` from forced push at
  [`pushargs.rs`](../../gwz-cli/src/pushargs.rs:24).
- `log`: operand, pathspec, and filter lowering is in
  [`logargs.rs`](../../gwz-cli/src/logargs.rs:18).
- `diff`: operand, pathspec, and presentation/filter lowering is in
  [`diffargs.rs`](../../gwz-cli/src/diffargs.rs:25).
- `status`: the public status argument surface is in
  [`statusargs.rs`](../../gwz-cli/src/statusargs.rs:3).
- `stash`: push/list/apply/pop/drop are declared in
  [`parser.rs`](../../gwz-cli/src/globalargs/parser.rs:548).
- `clone` / `init`: workspace-aware clone/init argument definitions are in
  [`parser.rs`](../../gwz-cli/src/globalargs/parser.rs:249) and
  [`repo.rs`](../../gwz-cli/src/clirequest/repo.rs:27).
- `rebase` / `reset`: native methods are declared in
  [`contract.rs`](../src/git/gitbackend/contract.rs:709), while pull's sync
  selection is evidenced above.
- Global `--all`, `--dry-run`, and `--force` are defined in
  [`parser.rs`](../../gwz-cli/src/globalargs/parser.rs:69), lowered to request
  metadata in [`invocation.rs`](../../gwz-cli/src/clirequest/invocation.rs:61),
  and planned by the core runtime in
  [`operation_runtime.rs`](../src/operation/operation_runtime.rs:246).

## Configuration management and configuration honoring

Git configuration has two distinct questions: can the user manage a setting,
and does an operation honor settings that are already present? They must not be
collapsed into one “config supported” claim.

### Management surface

| Git configuration function | GWZ status | Evidence / boundary |
|---|---|---|
| Read arbitrary config, including effective values | absent as a public command | No `gwz config`; `repo sync` reads selected metadata, not an arbitrary config view |
| Set/unset arbitrary keys | absent | Only `gwz auth identity REMOTE --set/--unset` manages the single local key `remote.<name>.gwzSshIdentity` |
| System/global/local/worktree/file/blob scopes | absent except local identity | Git supports system/global/local/worktree/file scopes for reads or writes; `--blob` is a read-only config source. GWZ identity writes only repository-local config |
| Includes and conditional includes | implicit/uncertain | libgit2 config loading may read its supported config sources, but no GWZ management or parity contract confirms Git's include/includeIf behavior |
| Multi-valued keys, append, value matching, typed canonicalization | absent | No API/CLI equivalent for `git config set/unset --all/--append/--value/--fixed-value/--type` |
| Rename/remove sections and interactive edit | absent | No command or core service |
| URL-scoped config matching | implicit/uncertain | Remote URL/config is read by native transport; complete Git URL-match precedence is not a GWZ contract |
| Repository/host ownership of future config | future design question | The inventory does not invent a policy for whether a setting belongs to a member repository, workspace host, endpoint, or selected operation |

Git's documented default is to read system, global, and repository-local files,
with optional worktree and command scopes; writes default to local and select one
scope/file. These scopes and includes need an explicit GWZ design if config
management becomes product scope.

### Important implicit honoring and gaps

| Configuration family | Current observation | Status |
|---|---|---|
| Remote URLs, push URLs, fetch refspecs | Native remote reads drive clone/fetch/push and publication planning | partial; no general remote editor and no full refspec/options CLI |
| SSH identity | Invocation `--identity`/`--remote-identity` overrides local `remote.<name>.gwzSshIdentity`; `gwz auth identity` manages the local key | partial; exact precedence is GWZ-owned and HTTPS ignores SSH identity as documented |
| Credential helpers | libgit2 callbacks can use configured helpers under `CredentialHelperPolicy::AllowConfigured`; a backend can disable them | partial; no Git credential management and no promise for every helper protocol/config edge |
| `user.name` / `user.email` and environment identity | Git CLI commit/tag resolves the ambient Git identity; native merge/stash paths use libgit2 signatures and GWZ attribution helpers | partial/asymmetric; `GIT_AUTHOR_*`, `GIT_COMMITTER_*`, `user.useConfigOnly`, `author.*`, `committer.*` parity is not one shared policy |
| Hooks (`core.hooksPath`, commit/tag/reference hooks) | Git CLI commit/tag can run hooks; native libgit2 merge/stash/scoped commits do not run ordinary Git hooks | partial and operation-dependent |
| Signing (`commit.gpgSign`, `tag.gpgSign`, `gpg.format`, signer programs) | Git CLI paths can honor configured signing; native merge/stash/scoped commits are not generally signed | partial; no uniform signing contract |
| Attributes/filters (`.gitattributes`, `core.attributesFile`, clean/smudge/filter) | libgit2 checkout/index behavior is used; newly created repositories pin `core.autocrlf=false` and `core.eol=lf`, while foreign `filter=` attributes remain fail-closed | partial and intentionally conservative; complete Git filter/attribute parity is not claimed |
| Ignore (`.gitignore`, `core.excludesFile`, `info/exclude`) | Staging/status use native status/index behavior and ignore-related settings where libgit2 exposes them | partial; no `check-ignore`/ignore-debug operation |
| Diff/log configuration (`diff.*`, `log.*`, mailmap, notes, pager/color) | GWZ has its own rendering and options; some native diff reads are used | partial; Git config does not automatically imply GWZ output parity |
| Transport (`http.*`, `url.*.insteadOf`, proxy, SSH, `remote.*`) | Native transport and GWZ endpoint policy read selected remote/identity data; future remote endpoint work remains planned | partial/uncertain for complete Git config precedence and every transport helper |

The source evidence for the strongest configuration boundaries is
[`identity.rs`](../src/git/gitbackend/transport_support/identity.rs:194),
[`repository_support.rs`](../src/git/gitbackend/repository_support.rs:9),
[`transport_support.rs`](../src/git/gitbackend/transport_support.rs:141), and
the Git backend contract's explicit commit/tag CLI-fallback comments
([`contract.rs`](../src/git/gitbackend/contract.rs:967)).

## Blocked workflows and ordinary-developer priority

For a developer operating one selected member, the highest-impact gaps are:

1. **Commit/tag policy parity.** Hooks, identity, signing, message cleanup,
   amend/empty/signoff/trailer behavior, and tag verification cannot be assumed
   uniform across ordinary, merge-resolution, stash, and scoped commits. Git is
   still a runtime dependency for ordinary commit/tag and a conditional local
   fetch fallback.
2. **Remote publication and fetch modes.** A normal selected-repository push or
   fetch works through GWZ's captured plans, but force-with-lease, arbitrary
   refspecs, deletion, tags/mirroring, shallow/partial fetch, and negotiated
   transport modes are unavailable.
3. **History and change inspection.** `gwz log` and `gwz diff` cover the GWZ
   workspace view, but Git's graph/pretty/pathspec/history simplification,
   blame/show/grep/describe/range-diff, and several diff algorithms are absent.
4. **Index/worktree repair.** There is no direct `restore`, mixed/soft reset,
   revert, rm/mv, clean, interactive add, or sparse-checkout workflow. A user
   who needs these must currently use another tool, which is especially material
   because future remote-core execution removes local Git as a reliable escape
   hatch.
5. **Configuration diagnosis.** Without `gwz config`-equivalent read/list and
   scoped write/unset operations, users cannot inspect or repair arbitrary
   repository/host config through the same selected-member boundary.

Lower-priority gaps are foreign SCM/mail commands, GUI/browser commands, server
daemons, pack maintenance, and obscure object/index plumbing. They remain listed
for completeness; they should not be mistaken for ordinary single-repository
developer requirements.

## Assessment of the proposed coverage tiers

The proposed 80/90/95% tiers are useful prioritization prompts, but this
inventory supplies no empirical coverage denominator or evidence for those
percentages. Retain the proposed groups as discussion inputs:

| Proposed group | Suggested command/mode scope | Assessment |
|---|---|---|
| Daily cycle (labelled 80%) | status, diff, log, show, add, commit, checkout/switch, restore, clone, fetch, pull, push | Add branch creation, upstream setup and basic configuration to make the cycle usable |
| Branch collaboration (labelled 90%) | branch, tag, remote, merge, stash, blame/annotate, ls-files/ls-tree | Include conflict inspection, resolution, continue and abort whenever integration can conflict |
| History editing/recovery (labelled 95%) | rebase, cherry-pick, revert, amend, reset, clean, reflog, merge-base, rev-parse, apply/am | Advanced editing may follow later, but recovery for earlier mutations cannot wait for this tier |

Configuration, identity, hooks and signing cross all groups. File editing in a
remote working copy also needs a supplied editing/filesystem surface; a Git
operation inventory alone does not establish that capability.

A defensible next assessment should measure complete scenarios:
branch-create → edit → stage → commit → push/upstream → rejection handling;
config/identity/hooks/signing across the same path; merge/pull conflict
resolution with continue/abort; and recovery alongside each mutating operation.
Audience-specific scenarios should then cover worktrees, submodules, and
bisect. The verified command and first-pass option gaps above should remain the
baseline; the tiers should not be read as implementation authorization or as
claims that a command count predicts workflow coverage.

## Where GWZ adds value, and how that maps to CLI ↔ core wire mode

These are **discussion notes and proposed capability boundaries**, not verified
implementation, accepted requirements, or authorization to expand the product.
The source inventory above remains the statement of current support. The goal
under discussion is a multi-repository manager whose supported workflows remain
usable when the CLI and repository-owning core are in different processes or on
different machines. Matching every Git command and option is not assumed.

Three kinds of value should be distinguished:

- **Workspace coordination:** select members, preflight changes, preserve work,
  track partial completion and provide coherent continuation or recovery.
- **Workspace routing and presentation:** locate the owning repository from a
  path, qualify results by member and aggregate compatible queries.
- **Remote access:** expose an ordinary single-repository capability through
  the existing service boundary. This can complete a remote workflow without
  providing a new multi-repository algorithm.

The existing `--target` model already supplies single-member selection. A new
standalone repository service is not automatically necessary. The remaining
questions concern missing capabilities and whether their inputs, outputs and
lifecycles are adequately represented by the CLI/core API.

| Capability | Potential GWZ advantage | Boundary or limitation | Implication for wire mode |
|---|---|---|---|
| Daily status, diff, log, stage and commit | Consistent selection, path routing and workspace results | Existing commands remain partial; the proposed daily tier is not proven complete, including `show` and `restore` gaps | Audit each supported mode through request, execution and result; command availability alone is insufficient |
| Blame / annotate | Resolve a file path to its member automatically | Attribution is normally computed within one repository; a batch of files still has separate histories | Carry member-relative path, revision and line selection; return member identity with commit attribution. Historical paths need not exist in the current worktree |
| ls-files / ls-tree / show | Convenient workspace paths and member-qualified inspection | Index entries, historical tree entries and object display are different queries; one revision name may resolve differently in each member | State whether the query addresses index, worktree or a revision; identify the repository for every object and stream large results |
| Branch, tag, remote and configuration management | Consistent operations over selected members; visible differences between members | Multi-member naming or configuration changes need explicit per-member results; not every setting should be broadcast | Represent scopes, effective values and origins, target repositories and failures rather than forwarding an unqualified client-side path |
| Merge and ordinary synchronization | Preflight, preserve work and manage partial integration across members | Existing coordinated merge is not evidence that every proposed integration mode has the same guarantees | Conflict inspection, resolution inputs, status, continue and abort must be reachable remotely whenever the selected mode can conflict |
| Rebase | Potential coordinated rewrite with original-to-new commit mappings and workspace reference updates | More than fan-out: one member may conflict after others finish; coordinated rebase is not established by the existing native rebase primitive | Requires an explicit operation lifecycle, member progress, retained original identities and a defined resume/abort contract |
| Cherry-pick / revert | Apply or reverse a recorded logical change spanning members | Needs a mapping from the logical change to each repository's commits; a single commit ID or workspace snapshot does not necessarily supply it | Accept explicitly identified member commits or a defined workspace change identity; report conflicts and partial outcomes |
| Amend | Convenient targeted access; potentially amend a recorded workspace change | Usually a single-member action. Workspace-wide amend cannot assume every member's HEAD belongs to the same change | Identify the member and expected current commit; coordinated amend would need separate rules for member selection and workspace references |
| Reset / restore | Preview and recover a coherent recorded workspace state while preserving work | HEAD, index and worktree changes must remain distinct; a workspace snapshot does not imply preservation of uncommitted content | Make the affected state and selection explicit; preserve/recovery policy and failure results must survive the remote boundary |
| Clean | Workspace-wide preview and protection of member/metadata boundaries | Arbitrary untracked content may be irreplaceable. This is not automatically a reversible operation | Bind execution to an explicit target/path set and policy, handle changes since preview, and expose failures; never imply that abort reconstructs deleted content |
| Reflog / workspace recovery history | Explain which workspace operation moved which member refs | Repository reflogs are local histories with retention limits, not a durable workspace transaction journal | Distinguish repository history from recorded GWZ operations; identify what can actually be restored and what evidence has expired |
| Apply / am | Route changes to members and coordinate conflict handling | Cross-member patches need an unambiguous path convention; mailbox application also creates commits and has additional semantics | Supply content through the service or a core-accessible reference, not an assumed client-local filename; expose apply/continue/abort results |
| Merge-base / rev-parse and other plumbing | Useful internal primitives and selected diagnostic queries | Little inherent workspace advantage; object identities and revision expressions are repository-specific | Expose only queries needed by supported workflows, with explicit repository context; no requirement to mirror every plumbing command |

### Wire mode completes workflows; it does not supply their semantics

The Git upstream connection and the CLI/core connection solve different problems.
The former transfers repository data; the latter asks core to operate on its
repositories. Message-stream transport and connection pooling do not implement
missing repository operations, edit remote files, or make an existing local
subprocess usable on the CLI machine against a remote working copy.

For each proposed capability, the wire-readiness audit should record:

1. **Repository and path context:** where member selection and path resolution
   occur, how invocation-relative paths are interpreted, and how historical
   paths are addressed. A client's absolute filename must not accidentally
   identify a different file on the core host.
2. **Inputs and results:** options, revision/object identities, binary or patch
   content, member-qualified results, errors and bounded progress/output streams.
3. **Execution environment:** which host owns repository/global configuration,
   identity, hooks, filters and signing tools. Reading configuration and honoring
   it are separate obligations. Client presentation preferences may belong on
   the CLI; repository and endpoint settings need deliberate ownership.
4. **Stateful operation lifecycle:** for mutations, what happens on cancellation,
   connection loss, partial member completion and reconnection. Losing a reply
   must not trigger an automatic replay of a possibly completed mutation;
   status/reconciliation must establish what happened first.
5. **Complete user workflow:** how users inspect conflicts, edit content, stage
   resolutions and continue or recover. A separately supplied remote editor or
   filesystem surface may satisfy editing; this inventory does not assign that
   responsibility to gwz-transport.

These are audit questions, not a new carrier specification. Reuse existing taut
requests/responses and the supplied asynchronous communication layer wherever
they suffice. Any missing semantic fields or operation lifecycle need a scoped
design; this discussion does not alter the current CLI/core interface or add
physical framing to gwz-transport.

### Product boundary and suggested next assessment

Remote execution alone does not force Git CLI replacement. The combination of
remote-only repository access, broad Git compatibility and no Git executable
dependency creates that larger obligation. An explicit endpoint-local Git
execution facility is an alternative product choice, distinct from a hidden
fallback, but has **not** been adopted. Arbitrary execution must not be presented
as a coordinated workspace operation or credited as typed capability coverage.

Use this inventory to choose a managed workflow surface rather than a
177-command replacement checklist. Suggested assessment order:

1. Prove the daily single-target workflow and its options/configuration across
   the API, including identified inspection and restore gaps.
2. Assess path-directed blame and listing, branch/upstream/remote management,
   and complete conflict resolution. These extend familiar GWZ routing and
   coordination with relatively clear capability boundaries.
3. Assess preservation and recovery for each admitted mutation before promising
   advanced coordinated rewriting. Evaluate targeted cherry-pick/revert/amend
   separately from workspace-wide logical-change operations and rebase.

For each scenario, record current evidence, missing command/mode/config behavior,
GWZ-specific advantage, wire/API gap, editing/environment prerequisites and a
completion/recovery test. This is the bridge from inventory to future scoped
designs. The accepted no-fallback plan continues to address existing subprocess
routes; it does not establish this broader workflow or wire-mode completeness.

## Auditable command-universe classification

The table has 184 unique names: all 177 installed `main` names, six
additional documented names (`citool`, `gitk`, `gitweb`, `gui`, `scalar`,
`svn`), and the observed external `filter-repo` supplement. `filter-repo`
is outside the Git-core denominator. These counts describe this installation,
not usage coverage. Alias rows inherit their underlying capability gaps.

| Git name | Status | GWZ equivalent / exposure | Boundary note |
|---|---|---|---|
| `add` | partial | gwz add | — |
| `am` | absent | — | — |
| `annotate` | absent | — | — |
| `apply` | absent | — | — |
| `archimport` | absent | — | — |
| `archive` | absent | — | — |
| `backfill` | absent | — | — |
| `bisect` | absent | — | — |
| `blame` | absent | — | — |
| `branch` | partial | gwz branch | — |
| `bugreport` | absent | — | — |
| `bundle` | absent | — | — |
| `cat-file` | backend-only | — | — |
| `check-attr` | absent | — | — |
| `check-ignore` | absent | — | — |
| `check-mailmap` | absent | — | — |
| `check-ref-format` | absent | — | — |
| `checkout` | partial | gwz materialize / gwz branch --switch | — |
| `checkout--worker` | absent | — | — |
| `checkout-index` | backend-only | — | — |
| `cherry` | absent | — | — |
| `cherry-pick` | absent | — | — |
| `citool` | absent | — | — |
| `clean` | absent | — | — |
| `clone` | partial | gwz clone / gwz repo clone | — |
| `column` | absent | — | — |
| `commit` | partial | gwz commit | — |
| `commit-graph` | absent | — | — |
| `commit-tree` | backend-only | — | — |
| `config` | absent | — | — |
| `count-objects` | absent | — | — |
| `credential` | partial | configured credential callbacks only | configured helpers may be honored; no helper-management command |
| `credential-cache` | absent | — | — |
| `credential-cache--daemon` | absent | — | — |
| `credential-netrc` | absent | — | — |
| `credential-osxkeychain` | absent | — | — |
| `credential-store` | absent | — | — |
| `cvsexportcommit` | absent | — | — |
| `cvsimport` | absent | — | — |
| `cvsserver` | absent | — | — |
| `daemon` | absent | — | — |
| `describe` | absent | — | — |
| `diagnose` | absent | — | — |
| `diff` | partial | gwz diff | — |
| `diff-files` | backend-only | — | — |
| `diff-index` | backend-only | — | — |
| `diff-pairs` | backend-only | — | — |
| `diff-tree` | backend-only | — | — |
| `difftool` | absent | — | — |
| `difftool--helper` | absent | — | — |
| `fast-export` | absent | — | — |
| `fast-import` | absent | — | — |
| `fetch` | partial | gwz fetch | — |
| `fetch-pack` | absent | — | — |
| `filter-branch` | absent | — | — |
| `filter-repo` | uncertain | — | external command reported by git help -a, outside Git core |
| `fmt-merge-msg` | absent | — | — |
| `for-each-ref` | backend-only | — | — |
| `for-each-repo` | partial | gwz forall | GWZ forall provides bounded fan-out, not Git repository-list config semantics |
| `format-patch` | absent | — | — |
| `fsck` | absent | — | — |
| `fsck-objects` | absent | — | — |
| `fsmonitor--daemon` | absent | — | — |
| `gc` | absent | — | — |
| `get-tar-commit-id` | absent | — | — |
| `gitk` | absent | — | — |
| `gitweb` | absent | — | — |
| `grep` | absent | — | — |
| `gui` | absent | — | — |
| `hash-object` | backend-only | — | — |
| `help` | partial | GWZ help/reference | Own command documentation; not the Git manual catalog |
| `hook` | absent | No general Git hook execution operation | gwz hook manages Claude lifecycle integration; it is a different capability |
| `http-backend` | absent | — | — |
| `http-fetch` | absent | — | — |
| `http-push` | absent | — | — |
| `imap-send` | absent | — | — |
| `index-pack` | absent | — | — |
| `init` | partial | gwz init / gwz repo create | — |
| `init-db` | partial | gwz init / gwz repo create | Git alias of init; inherits the initialization gaps |
| `instaweb` | absent | — | — |
| `interpret-trailers` | absent | — | — |
| `last-modified` | absent | — | — |
| `log` | partial | gwz log | — |
| `ls-files` | backend-only | — | — |
| `ls-remote` | backend-only | — | core advertisement used for planning; no public arbitrary query |
| `ls-tree` | backend-only | — | — |
| `mailinfo` | absent | — | — |
| `mailsplit` | absent | — | — |
| `maintenance` | absent | — | — |
| `merge` | partial | gwz merge | — |
| `merge-base` | backend-only | — | — |
| `merge-file` | absent | — | — |
| `merge-index` | absent | — | — |
| `merge-octopus` | absent | — | — |
| `merge-one-file` | absent | — | — |
| `merge-ours` | absent | — | — |
| `merge-recursive` | absent | — | — |
| `merge-recursive-ours` | absent | — | — |
| `merge-recursive-theirs` | absent | — | — |
| `merge-resolve` | absent | — | — |
| `merge-subtree` | absent | — | — |
| `merge-tree` | backend-only | — | — |
| `mergetool` | absent | — | — |
| `mktag` | backend-only | — | — |
| `mktree` | backend-only | — | — |
| `multi-pack-index` | absent | — | — |
| `mv` | absent | — | — |
| `name-rev` | absent | — | — |
| `notes` | absent | — | — |
| `p4` | absent | — | — |
| `pack-objects` | absent | — | — |
| `pack-redundant` | absent | — | — |
| `pack-refs` | absent | — | — |
| `patch-id` | absent | — | — |
| `pickaxe` | absent | — | — |
| `prune` | absent | — | — |
| `prune-packed` | absent | — | — |
| `pull` | partial | gwz pull | — |
| `push` | partial | gwz push | — |
| `quiltimport` | absent | — | — |
| `range-diff` | absent | — | — |
| `read-tree` | backend-only | — | — |
| `rebase` | partial | global --sync rebase / core rebase_onto | — |
| `receive-pack` | absent | — | — |
| `reflog` | absent | — | — |
| `refs` | backend-only | — | — |
| `remote` | backend-only | — | core remote read/add only; no public remote manager |
| `remote-ext` | absent | — | — |
| `remote-fd` | absent | — | — |
| `remote-ftp` | absent | — | — |
| `remote-ftps` | absent | — | — |
| `remote-http` | absent | — | — |
| `remote-https` | absent | — | — |
| `repack` | absent | — | — |
| `replace` | absent | — | — |
| `replay` | absent | — | — |
| `repo` | absent | — | Git low-level repo query; distinct from GWZ repo |
| `request-pull` | absent | — | — |
| `rerere` | absent | — | — |
| `reset` | partial | global --sync reset / core reset_hard | — |
| `restore` | absent | — | — |
| `rev-list` | backend-only | — | path-filtered history still uses a subprocess |
| `rev-parse` | backend-only | — | — |
| `revert` | absent | — | — |
| `rm` | absent | — | — |
| `scalar` | absent | — | — |
| `send-email` | absent | — | — |
| `send-pack` | absent | — | — |
| `sh-i18n--envsubst` | absent | — | — |
| `shell` | absent | — | — |
| `shortlog` | absent | — | — |
| `show` | absent | — | — |
| `show-branch` | absent | — | — |
| `show-index` | absent | — | — |
| `show-ref` | backend-only | — | — |
| `sparse-checkout` | absent | — | — |
| `stage` | partial | gwz add | Git alias of add; inherits the add option gaps |
| `stash` | partial | gwz stash | — |
| `status` | partial | gwz status | — |
| `stripspace` | absent | — | — |
| `submodule` | absent | — | REQ-001 says v0 MUST NOT require submodules; this is not an explicit command exclusion |
| `submodule--helper` | absent | — | — |
| `subtree` | absent | — | — |
| `svn` | absent | — | — |
| `switch` | partial | gwz branch --switch / gwz materialize --switch | — |
| `symbolic-ref` | backend-only | — | — |
| `tag` | partial | gwz tag | — |
| `unpack-file` | absent | — | — |
| `unpack-objects` | absent | — | — |
| `update-index` | backend-only | — | — |
| `update-ref` | backend-only | — | — |
| `update-server-info` | absent | — | — |
| `upload-archive` | absent | — | — |
| `upload-archive--writer` | absent | — | — |
| `upload-pack` | absent | — | — |
| `var` | absent | — | — |
| `verify-commit` | absent | — | — |
| `verify-pack` | absent | — | — |
| `verify-tag` | absent | — | — |
| `version` | partial | GWZ version output | Own build/version; not Git executable build details |
| `web--browse` | absent | — | — |
| `whatchanged` | absent | — | — |
| `worktree` | absent | — | REQ-002 defers storage backends; it does not explicitly exclude worktree management |
| `write-tree` | backend-only | — | — |

## Uncertainty and limits

- This report does not claim that every Git flag, deprecated spelling, config
  variable, environment variable, or output format was exhaustively compared.
  Concrete major options are listed for existing GWZ equivalents; obscure
  plumbing modes are marked absent/unverified or backend-only.
- Native libgit2 behavior can honor some ambient configuration without GWZ
  having a deliberate parity contract. “Implicit/uncertain” is intentionally
  separate from configuration management.
- The command universe is versioned. A later Git release can add builtins or
  scripts; rerun the three probes and update the companion before treating this
  report as current.
- Tests and fixture setup are outside this inventory's product surface. The
  source contains many test-only Git subprocesses; the report calls out only
  production routes relevant to capability and the planned no-fallback work.
- The CLI's deliberate non-GWZ worktree fallback in
  [`gwz-cli/src/hook/fallback.rs`](../../gwz-cli/src/hook/fallback.rs:21) remains
  outside the core no-fallback guarantee.

No implementation, staging, commit, push, repository provisioning, build, or
test was performed for this inventory.

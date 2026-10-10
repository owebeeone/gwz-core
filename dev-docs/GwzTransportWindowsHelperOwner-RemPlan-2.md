# WH2 helper owner: remediation plan (round 2)

Object: gwz-core `115993d7`.

- **Code+Consistency round 2: NO-GO.** All eight round-1 findings are closed. New: N1 (P2), N2 (P2), N3 (P3).
- **Safety+State round 2: GO.** All six round-1 findings are closed. New P3s: N1, N2, N3.

The reports are filed as `-ReviewCodeConsistency-2.md` and `-ReviewSafetyState-2.md`.

**Blind convergence.** Both reviewers independently found the same parked stderr reader (Code N1 as P2, Safety N1 as P3), and both found an understated inheritance window (Code N3, Safety N2). Both are non-architectural. This is the last remediation round under the cap.

| Finding | Disposition | Closure test |
| --- | --- | --- |
| Code N1 / Safety N1: a released survivor holding the helper's stderr parks a blocking-pool thread and gwz's pipe handle, and the runtime cannot drop | Before retire on success, `CancelIoEx` every pending parent-pipe read and write, so no blocking task outlives the lookup. A tokio `NamedPipeServer` is acceptable instead if it is simpler. | A Windows test: the helper answers, then leaves a descendant with stdout redirected to `nul` and stderr inherited. The lookup returns `Ok`, and a current-thread runtime running it drops within a bound (drop on a thread, join with a timeout). |
| Code N2: the environment block refuses `=C:`-style names that the snapshot accepts | Refuse `=` only after the first unit, matching `EnvironmentSnapshot::checked`. Align contract §5. | A unit test of `environment_block` with `=C:`, and a Windows spawn whose snapshot carries one. |
| Code N3 / Safety N2: the inheritable window is understated and real std spawns exist | Create the child ends non-inheritable and set `HANDLE_FLAG_INHERIT` immediately before `CreateProcessW`. Clear it again on every path where creation fails. Restate C9's window and its consequences (a stalled lookup until the deadline, a parked reader, write access to the answer pipe), cite the allowlisted std spawn sites, and tie the spawn gate to their removal. | A unit or ordering test that the ends are non-inheritable until the call. Contract text. |
| Safety N3: the ceiling is unproven at a drive root | A dabeest row: plant `.git/config` at the root of a `subst` drive, run `confine()` from a directory under it with the shipped Git for Windows, and assert the planted helper is never read. Record the minimum Git version for step 4.3. | That row, archived. |

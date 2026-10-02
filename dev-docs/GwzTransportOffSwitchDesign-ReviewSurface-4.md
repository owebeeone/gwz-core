# GWZ transport setting design (TR1.5) — SURFACE-AXIS CONFIRMATION OF REVISION 3

The Surface reviewer's confirmation of the forms revision 3 gives its round-3 P3-10 and P3-11, filed verbatim by the lane owner.

**Hash:** `reviews/GwzTransportOffSwitchDesign-rev3.md` verified `490dac86…8032` at 14:21 and again at 14:23:52 AEST; read §10 (lines 172–231), §2 line 37 and §3 line 60 only.

**P3-10 — confirmed.** The `git -C / config --file '<file>' --includes --show-origin --get-all gwz.transport` form is better than mine. §2 line 37 and §3 line 60 state that gwz opens the global files and the scanned files without a repository, so libgit2 skips every `includeIf`; git run from `/` discovers no repository either, so the command follows `include.path` and applies no conditional include — exactly what gwz read, from any shell directory, and `/` needs no quoting. My round-3 `includeIf "gitdir:"` scenario was wrong on its premise: such a value can never produce the "included" note, because gwz never reads it. The `-C '<dir>'` form I proposed would have listed values gwz does not read; withdrawn.

One line the drafter may want, not an objection and not blocking: that same fact means a user who keeps `gwz.transport = native` behind an `includeIf` sees git honour it and gwz ignore it with no note and `transport: gwz (default; read …)` under `--verbose`. §10's long help already names `GIT_CONFIG_GLOBAL` and system configuration as not read; adding "conditional includes (includeIf) are not applied" beside them closes that in the same way P3-8 closed.

**P3-11 — confirmed.** §10 now pins gwz-py's CLI `--ssh-timeout` help verbatim ("… on gwz's transport and, with GWZ_TRANSPORT=native, as libgit2's connect and read timeout (0 = no timeout, default 9 on either transport)") and has S7.2 rewrite `commands/auth.md:50-53` to state each driver's default. That satisfies my check (gwz-py's help states 9; the shared page states both drivers). The drafter's objection to my wording is right: on gwz's own transport the value is the stall clock, not libgit2's timeout, and the chosen text says so.

Both corrections stand as cleared without a further round; my GO verdict from round 3 is unchanged.

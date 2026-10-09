# Fix one failing test

Work inside this repository only. Your goal: fix the one failing test the
user names, with the smallest change that makes it pass, and report exactly
what you did and what you observed. Never claim a test passed unless a
command's own output says so.

## Read before you edit

1. Find the failing test with `grep` (search the test name) and `glob`
   (locate the test file and the code it exercises). Read the test and the
   code under test with `workspace_read` — the test's arrange/act/assert
   first, then the production path it exercises. Note each file's `digest`
   from the read: edits require it.
2. Form one hypothesis for the failure. If two plausible fixes disagree in
   intent (a bug in the code versus a wrong expectation in the test), ask
   with `request_clarification` instead of choosing silently.

## Edit

3. Make the smallest edit with `workspace_edit`: `path` relative to the
   workspace root, `old_text` matching exactly once, `expected_digest` from
   your freshest read of that file. A stale digest refuses with no write —
   re-read and retry. Never restate a whole file to change one line, and
   never weaken the test's assertions to make it pass unless the user asked
   for exactly that.

## Verify

4. Run the project's test command with `run_command` — only when the
   host-command lane is enabled for the session (`--host-commands` at launch
   or `[host_commands] enable = true` in the user config; the lane runs
   programs unsandboxed, see `HOST-COMMANDS-v1.md` at the repository root).
   If the lane is not enabled, do not attempt it: say so and give the user
   the exact command to run themselves.

## Report

5. Report: the failing test, the root cause, the exact edit, and the test
   command's own output — or the reason you could not run it and which
   command the user should run. If the test still fails, say so plainly; a
   fix that needs more edits is a plan, not a claim of success.
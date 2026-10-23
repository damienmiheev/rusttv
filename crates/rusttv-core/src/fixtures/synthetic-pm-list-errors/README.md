# SYNTHETIC fixtures: `pm list packages` failures

**None of these files were captured from a device.** They are hand-written
stand-ins for the ways `pm list packages` can fail, so the parser can tell a
failure apart from an empty list. Replace each with a real capture, and
update this table, if one is ever recorded.

With adb's shell protocol (`shell_v2`, devices since Android 7), `pm`'s
non-zero exit code reaches adb and the run already fails as
`COMMAND_FAILED`. Without it, adb exits 0 and `pm`'s stderr arrives mixed
into stdout; these files are that stdout. An empty list is no output at all
(`../synthetic-home-takeover/packages-d-none.txt`).

| File | Stands for | Based on |
|---|---|---|
| `cant-find-service.txt` | Package service not up yet (early boot) | AOSP `frameworks/native/cmds/cmd/cmd.cpp` message; not captured |
| `failure-calling-service.txt` | Binder call to the package service failed | Same file |
| `exception.txt` | `pm` threw (here a permission check); stack trace trimmed | AOSP `ShellCommand` "Exception occurred while executing" message; not captured |
| `error-unknown-option.txt` | `pm` rejected an option | AOSP `PackageManagerShellCommand`; not captured |
| `error-not-running.txt` | Pre-Android 7 `pm` could not reach the system server | AOSP `Pm.java` (Android 6 and older); not captured |
| `sh-pm-not-found.txt` | No `pm` binary on the device | `sh` "not found" format, as in `../../error.rs` tests |
| `packages-then-exception.txt` | `pm` died part-way through the list | First two lines of `../synthetic-launcher-install/packages-f-rusttv.txt`, then the `exception.txt` format |

# Security and recovery

## Credential boundary

Restic provides backup encryption, integrity, deduplication and repository
locking. ysynk generates a random password per new repository and stores a
versioned location-bound credential record only in the OS keychain. Configuration
and operation history contain identifiers and results, not credentials.

Linux uses persistent Secret Service, macOS Keychain and Windows Credential
Manager. Unsupported, unavailable, locked, ambiguous or malformed stores fail
closed. Only an authoritative missing-entry result permits generation. There
is no automatic plaintext-file or in-memory-only production fallback.

Linux uses standard Plain Secret Service sessions: secrets travel unencrypted
over the trusted local user session bus. Untrusted or remote buses, same-user
malware and privileged observers are outside this boundary. Persistent storage
protection remains the native keychain's responsibility; ysynk creates no
plaintext credential fallback. The dependency's optional DH transport is disabled
because its private-key and IV generation use non-cryptographic randomness.

Credentials are written before repository initialization. Retry reads the same
record and authenticates against the repository, rather than overwriting the
only password. Lookup, creation and initialization are serialized per repository
across app instances sharing the credential namespace. Changing an existing
repository location requires a new UUID; using a re-added UUID still checks the
stored location binding. Removing configuration does not remove credentials.

Repository URLs containing embedded passwords or percent-encoded userinfo are
rejected, including passwords hidden behind multiple `@` separators. Previously
accepted unsafe URLs can therefore prevent configuration loading after an
update. Preserve the keychain record and verify independent restic recovery
before editing such a locator: blindly rewriting it breaks its exact credential
binding. SCP-style SFTP usernames and remote paths containing `@` are not URL
passwords; their existing accepted strings are preserved.

The renderer cannot request arbitrary credentials or run shell commands. The
typed native API owns restic operations. Secrets are fetched for subprocesses
and passed through `RESTIC_PASSWORD`, never argv, config, logs or UI responses.
Status polling does not query the keychain. Background jobs do not silently
unlock the OS credential store; the user manages its availability in the desktop.

Live command output is bounded and repository passwords are removed before
lines reach the renderer. Output can include source paths and backend diagnostic
messages; it is not a public support log. Recent output remains available in
memory for completed jobs, but is omitted from persisted operation history.
Command arguments and environment variables are not displayed.

Changing the restic executable requires choosing a local file in the native
desktop picker. The app stores the resolved path after checking the configuration
revision and operation state. Ordinary configuration saves must retain the
current executable. Existing installations keep their configured engine, and
a newly selected engine is version-checked when the app restarts.

The signed-in user, OS keychain, restic executable, SSH/client helpers and host
are trusted. An unlocked desktop keychain is not protection against malware
running as that user or root. Child environments and process memory can be
inspected by sufficiently privileged processes. Buffer cleanup is best-effort,
not a guarantee against swap, debuggers or forensic recovery.

## Independent recovery

Keep both a surviving restic repository and its password in independent
recovery storage, such as a password manager or offline record. The OS keychain
alone is not a disaster-recovery strategy. Credential items are stored under
service `io.ysynk.restic`, account repository UUID. The JSON secret record contains
the password and its repository binding; use the native credential manager to
preserve it securely, or use **Settings → Export repository password**.

Export is an explicit exception to keychain-only storage. After confirmation,
a native save dialog selects a new local file. The native core reads only the
configured repository's location-bound keychain record and writes its password
plus a newline, suitable for restic's `RESTIC_PASSWORD_FILE`. Neither the
password nor the selected path is returned to the renderer or operation history.
Cancelling the native dialog does not read the credential or create a file.

The exported file is **not encrypted**. Anyone who can read it and access the
repository can decrypt backups, even after the system keychain is locked.
Choose a private recovery folder outside configured backup sources, repositories
and application data; resolved destination checks reject those locations.
Existing files and symlinks are not replaced. Files are created with mode `0600`
on Linux. Safe export currently requires Linux and rejects symlinked parent
paths. The selected parent must be owned by this user or root and not writable
by other users unless its sticky bit protects owned entries. The parent is
pinned by a file descriptor before credential access; ancestor renames cannot
redirect the staged file or its publication. Export reserves the core operation
slot while reading and writing, without logging a job or blocking status
polling. No keychain entry is changed or generated.
The staged file and published directory entry are synced before success. If
directory sync fails after publication, the error explicitly says the file was
written but its durability is uncertain. That file is preserved, not deleted;
verify it before relying on it, and do not expect a retry to overwrite it.
Move the result to offline storage or a password manager, remove unneeded
plaintext copies, and do not add its folder to a future backup profile.

With the password and any separate SSH/cloud access, the unmodified restic CLI
can inspect and restore the repository. No local application history/catalog is
required. Test recovery independently before relying on it.

## Desktop startup

Start-at-login registration is opt-in and launches the application's executable in
the user's desktop session, not a privileged boot service. Desktop preferences
contain no credentials. Instances using a custom `YSYNK_RESTIC_DATA_DIR` cannot
modify the installed application's login registration. A usable tray host is
required before a window is hidden; manual launches show the window.

## Destructive actions and concurrency

Retention previews are profile-tag scoped, expiring, single-use and bound to
configuration revision and repository identity. Apply rechecks candidates and
survivors and deletes explicit IDs. It never silently broadens the policy or
prunes automatically. External restic clients can still change a repository
between commands; this is not a cross-process transaction. Stop other
maintenance clients first. Failed/interrupted deletion requires a new preview.

Only the latest successful preview is retained, limiting preview memory to one
bounded query result. Creating another successful preview invalidates the
previous token, including for another repository or profile; a failed preview
does not replace an existing valid token. An empty repository has no candidates.

Prune is repository-wide and requires separate confirmation. No stale locks are
removed automatically. Restore creates a new private directory under a safe
parent and uses restic verification. Cancelled/failed output is left incomplete
for inspection, never presented as successful recovery. Quitting drains owned
subprocesses before releasing the application lock.

On Unix, subprocess pipes are cancellable and post-exit draining is limited to
one second. A detached helper that retains a pipe cannot indefinitely block
version detection, cancellation or shutdown; an incomplete drain is an error,
not successful output. The application terminates owned child process groups,
but does not discover or kill helpers that deliberately detach into another
session. Equivalent detached-pipe handling is not implemented for Windows,
which is not a release target.

If configuration or job-state persistence fails, configuration saves, exports
and new jobs are blocked and pending retention previews are discarded. Correct
the filesystem problem and restart the app to reload the published files. A
directory-sync failure can occur after replacement: the disk may already contain
the new configuration or an admission record, even though the operation reported
failure. An admission with no recorded completion is shown as interrupted on
restart; inspect history and run a manual backup if needed. Completed job results
remain visible in the current process when saving their history fails.

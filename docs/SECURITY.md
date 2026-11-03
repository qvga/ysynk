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

The renderer cannot request arbitrary credentials or run shell commands. The
typed native API owns restic operations. Secrets are fetched for subprocesses
and passed through `RESTIC_PASSWORD`, never argv, config, logs or UI responses.
Status polling does not query the keychain. Background jobs do not silently
unlock the OS credential store; the user manages its availability in the desktop.

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
on Unix; on Windows they inherit the destination folder's permissions. Export
reserves the core operation slot while reading and writing, without logging a
job or blocking status polling. No keychain entry is changed or generated.
Move the result to offline storage or a password manager, remove unneeded
plaintext copies, and do not add its folder to a future backup profile.

With the password and any separate SSH/cloud access, the unmodified restic CLI
can inspect and restore the repository. No local application history/catalog is
required. Test recovery independently before relying on it.

## Destructive actions and concurrency

Retention previews are profile-tag scoped, expiring, single-use and bound to
configuration revision and repository identity. Apply rechecks candidates and
survivors and deletes explicit IDs. It never silently broadens the policy or
prunes automatically. External restic clients can still change a repository
between commands; this is not a cross-process transaction. Stop other
maintenance clients first. Failed/interrupted deletion requires a new preview.

Prune is repository-wide and requires separate confirmation. No stale locks are
removed automatically. Restore creates a new private directory under a safe
parent and uses restic verification. Cancelled/failed output is left incomplete
for inspection, never presented as successful recovery. Quitting drains owned
subprocesses before releasing the application lock.

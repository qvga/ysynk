# Security and recovery

Each ysynk repository is a standard encrypted restic repository. Restic handles encryption, integrity checks, deduplication and repository locks. ysynk manages the repository password and runs restic on your behalf. **Keep an independent copy of each password while your desktop keychain is still available.**

## Independent recovery

You can recover on another machine with restic, access to the repository and its password. For a remote repository you also need the separate SSH or backend credentials that grant access to the storage. Recovery works without ysynk's configuration or operation history.

1. In **Settings → Export repository password**, select the repository and acknowledge the plaintext warning.
2. In the native save dialog, choose a **new file** in an existing private folder outside your backup sources, repositories and application data. The file contains the password followed by a newline. Anyone who can read it and access the repository can decrypt your snapshots, even after your keychain locks.
3. Keep the password in a password manager or offline recovery storage. Keep the repository location and backend access details available independently. Remove unneeded plaintext copies and keep the recovery folder out of future backup profiles.
4. Test recovery. For example, set these two paths to your repository and exported password file, then list the snapshots.

   ```sh
   repo=/mnt/backups/documents
   password_file=/media/recovery/documents-password.txt
   restic -r "$repo" --password-file "$password_file" snapshots
   ```

   Select a snapshot ID from the listing and a path that exists in it. This example restores one file to a new temporary directory and verifies its contents.

   ```sh
   snapshot_id=01234567
   target=$(mktemp -d) &&
     restic -r "$repo" --password-file "$password_file" restore "$snapshot_id" \
       --include '/home/alex/Documents/notes.txt' --target "$target" --verify
   ```

   Open the recovered file and confirm it is the version you expected. You can also supply the password file path through `RESTIC_PASSWORD_FILE`.

### How export protects the file

Safe password export and restore currently require Linux. Export creates a file with mode `0600`. The destination's parent must be owned by you or root. A directory writable by other users must have its sticky bit set. ysynk rejects symlinked parent paths and refuses to overwrite existing files or symlinks.

The native picker supplies the destination. Cancelling it leaves both the keychain and filesystem untouched. On acceptance, ysynk checks the configured repository and revision, pins the parent directory by file descriptor, then reads the matching keychain record. An ancestor rename cannot redirect the write. Export occupies the operation slot while status polling remains available. The renderer receives only the outcome. The secret and destination path stay out of the job history.

ysynk syncs the staged file, publishes it and syncs its directory before reporting success. If the final sync fails, the file can already exist. Check the exported file against the repository before relying on it. A retry requires a new file name.

## Credential boundary

The OS keychain stores a versioned record under service `io.ysynk.restic` and the repository's account ID. The record contains a random password and the exact repository location. Removing a repository from ysynk leaves its keychain item in place.

- **Initialization.** ysynk generates a credential only after the keychain reports the item missing. It saves the credential before initializing the repository. A retry authenticates using the saved record. Per-repository locks serialize initialization across app instances that share the credential namespace.
- **Location changes.** A different location requires a new repository ID. Re-adding an old ID checks its stored location. Repository URLs containing embedded passwords or percent-encoded user information are rejected. For an older configuration that fails validation, preserve the native keychain record and verify independent recovery before changing the URL. An edit can break the exact location binding. Existing supported SCP-style SFTP usernames and paths containing `@` keep their original spelling.
- **Keychain access.** Linux uses the default persistent Secret Service collection. An unavailable or locked store, duplicate entries, or a malformed credential stops the operation. Background jobs leave unlocking to the user. The Linux client uses Plain sessions on the trusted local user bus, so those messages carry secrets without transport encryption. A remote or untrusted bus is outside the supported setup. The macOS Keychain and Windows Credential Manager adapters are present in source. Linux x86-64 is the release target. [Dependency details](DEPENDENCIES.md)
- **Executable selection.** Only the native file picker can change the configured restic executable. The app checks the configuration revision and operation state, saves the selected path in resolved form, and probes its version at the next startup. Existing installations retain their configured executable.
- **Processes and output.** Native code accepts typed operations, looks up the bound credential and supplies it to restic through `RESTIC_PASSWORD`. Status polling reads application state without consulting the keychain. Displayed output is bounded and filtered for the repository password. It can still include private paths and backend diagnostics. Recent output and query results stay in memory and are omitted from the on-disk job journal.

The host, signed-in user, operating system keychain, restic binary and transport helpers are trusted. Same-user malware and privileged observers can inspect process memory and child environments. Buffer cleanup offers limited protection against swap, debugging or forensic recovery.

## Desktop startup

Login registration is opt-in and runs in your desktop session. Desktop preferences contain no credentials. A custom `YSYNK_RESTIC_DATA_DIR` isolates app configuration and cannot change the installed application's login registration. It still shares the user's keychain namespace. Manual launches open the window, and hiding requires an available tray host. [Login and schedule settings](guide.md#schedules-and-desktop-login)

## Destructive actions and concurrency

**Restore.** ysynk validates the selected repository credential before creating the output directory and uses that same credential for the restore. The target must be a new private directory under a trusted parent, outside repositories and application data. Restic verifies the restored content. If the job fails or is cancelled, inspect the incomplete output and use a new directory for a retry.

**Snapshot deletion.** A retention preview applies to a single profile tag and repository, expires after five minutes, and can be applied once. Only the most recent successful preview remains valid, even across repositories. A failed preview leaves the previous token valid until expiry. An empty repository has no candidates. On apply, ysynk rechecks the configuration revision, repository identity, candidate tags and survivors before deleting the explicitly previewed snapshot IDs. After an interrupted or failed apply, generate a new preview.

**Pruning and other clients.** Prune operates across the whole repository and requires separate confirmation. ysynk leaves stale locks in place. Stop other maintenance clients before applying retention or pruning. An external client can change the repository between ysynk's checks and its next command.

**Cancellation and shutdown.** ysynk drains owned subprocesses before releasing its application lock. On Unix it terminates owned process groups and bounds post-exit pipe draining to one second. A detached helper that holds a pipe open can cause an operation to fail. Helpers that create their own session may outlive the app. Windows lacks the equivalent detached-pipe handling.

**State write failures.** If ysynk cannot durably save configuration or job state, it blocks saves, exports and new jobs and clears retention previews. Correct the filesystem problem, restart, then inspect operation history before retrying. A disk error after file replacement can leave the new configuration or an admission record on disk. A recorded admission without a completion appears as interrupted after restart. If saving a completed job fails, its result remains available in the running process.

ysynk — encrypted backups for the Linux desktop

Run ./ysynk-desktop in your desktop session. You need Linux x86-64, restic
0.17.1 or later, GTK3, WebKitGTK 4.1, AppIndicator and a persistent Secret
Service keychain. Keep a recovery copy of each repository password.

To register the application in your desktop menu, run this from the extracted
directory, then reopen ysynk:

    python3 tools/install-desktop-entry.py ./ysynk-desktop

Keep this directory in place while its launcher is registered. Turn off
Start at login before moving it. If it has already moved, Settings explains
how to register the new location.

Guide: https://github.com/qvga/ysynk#your-first-backup
Downloads and checksums: https://github.com/qvga/ysynk/releases

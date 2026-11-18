Guardian 3.0 agent for Windows
==============================

Install on a student computer (Windows 10 or 11, 32 or 64 bit):

1. Unpack this whole folder (guardian-setup.exe needs the bin\ folder next to it).
2. Double-click guardian-setup.exe and accept the administrator prompt.
3. Fill in the server address, the certificate SHA-256 (if the console shows
   one) and the enrollment code from the console's "Connection info...", then
   click Install.

From a script, in an administrator prompt:
    guardian-setup.exe install --server wss://SERVER:8443 --code CODE [--cert-sha256 SHA256]

The agent goes to C:\Program Files\Guardian (service GuardianAgent), its data to
C:\ProgramData\Guardian Agent. Guardian 2.x (ProcSentinelAgent) is found and,
after you confirm, removed. Port 11500 (TCP and UDP) is opened for the local
network, for the console's fallback mode.

Other commands: guardian-setup.exe --help
(status, update, uninstall, reenroll, whitelist, service start/stop, logs)

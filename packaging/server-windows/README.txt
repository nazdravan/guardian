Guardian 3.0 server for Windows
===============================

guardian-server.exe is the Guardian server as a Windows service (64-bit).
Use it when the server should run all the time on a Windows machine. To run
the server only while the teacher's window is open, use guardian-console.exe
("Run the server on this computer") instead.

Install (in an administrator prompt, from this folder)
------------------------------------------------------
    copy guardian-server.exe "C:\Program Files\Guardian Server\"
    "C:\Program Files\Guardian Server\guardian-server.exe" service install

The service is GuardianServer (automatic start). Its data, the database and a
self-signed certificate, is in C:\ProgramData\Guardian Server\ (SYSTEM and
Administrators only). Install the exe from a folder only administrators can
write, such as Program Files: the service runs it as LocalSystem.

Open TCP port 8443 for the school network:
    netsh advfirewall firewall add rule name="Guardian server" dir=in action=allow protocol=TCP localport=8443 remoteip=localsubnet

First administrator
-------------------
The service log shows the certificate SHA-256 and, until the first
administrator exists, a bootstrap code. Create the administrator with
    guardian-cli.exe init --server https://<THIS-COMPUTER>:8443 --cert-sha256 <sha256>
or from guardian-console.exe ("Connect to a server").

Remove: guardian-server.exe service uninstall

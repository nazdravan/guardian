Guardian Server for Windows
===========================

Run the Guardian server on the teacher's own computer and manage the class
from one window. The server runs only while the window is open; when you close
it, the student computers keep the last rules they received.

First run
---------
1. Double-click GuardianServer.exe and accept the administrator prompt.
   It opens port 8080 in Windows Firewall (local network only) and creates
   C:\ProgramData\Guardian Server\ with the database and .env (tokens).
2. Click "Connect..." and copy into each student computer's agent.env:
       SERVER_ADDRESS=http://<THIS-COMPUTER-NAME>:8080
       TOKEN=<the agent TOKEN shown>
3. In the Android app (optional), use the phone address and ADMIN_TOKEN shown.
   Android usually cannot find a Windows computer by name, so use the IP
   address; it may change from day to day.

Notes
-----
- Do NOT install a Guardian agent on this computer: it would appear as a PC
  and "Block all PCs" would apply to it.
- Windows agents find this computer by name. Linux agents usually do too
  (through systemd-resolved); if not, use the IP address or ask for a fixed
  address (DHCP reservation) on the router.
- If no agent has connected after 2 minutes, the window shows a warning.
- Tokens are generated once and kept in C:\ProgramData\Guardian Server\.env.
  Replacing GuardianServer.exe keeps them.

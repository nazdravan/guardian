// Guardian Server is the Windows app a teacher opens at the start of a
// lesson: it runs the Guardian server in-process while its window is open and
// manages the class's applications and computers from one screen.
// See specs/2026-11-13-windows-server-gui-design.md.
package main

import (
	"os"
	"path/filepath"

	"github.com/lxn/walk"

	"server-gui/core"
	"server/guardian"
)

const appTitle = "Guardian Server"

func main() {
	if !acquireSingleInstance() {
		focusExistingWindow()
		return
	}

	a := &app{}
	a.dir = filepath.Join(os.Getenv("ProgramData"), "Guardian Server")
	a.hostname, _ = os.Hostname()
	a.ip = guardian.LocalIP()

	if s, err := core.LoadOrCreate(a.dir); err != nil {
		a.startErr = err.Error()
	} else {
		a.settings = s
		if err := ensureFirewallRule(s.Port); err != nil {
			a.firewallErr = err.Error()
		}
		if r, err := core.Start(a.dir, s); err != nil {
			a.startErr = core.StartError(err, s.Port)
		} else {
			a.runner = r
			a.client = core.NewClient(r.Port(), s.AdminToken)
		}
	}

	if err := a.build(); err != nil {
		walk.MsgBox(nil, appTitle, "Could not create the window: "+err.Error(), walk.MsgBoxIconError)
		a.stop()
		os.Exit(1)
	}
	a.startRefresh()
	a.mw.Run()
	a.stop()
}

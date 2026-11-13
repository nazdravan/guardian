package main

import (
	"fmt"
	"os/exec"
	"strings"
	"syscall"

	"github.com/lxn/win"
	"golang.org/x/sys/windows"

	"server-gui/core"
)

// instanceMutex is held for the life of the process; a second launch sees it
// and hands focus to the first window instead of failing on the port.
var instanceMutex windows.Handle

func acquireSingleInstance() bool {
	name, _ := windows.UTF16PtrFromString(`Local\GuardianServer`)
	h, err := windows.CreateMutex(nil, false, name)
	if err == windows.ERROR_ALREADY_EXISTS {
		windows.CloseHandle(h)
		return false
	}
	instanceMutex = h
	return true
}

func focusExistingWindow() {
	title, _ := windows.UTF16PtrFromString(appTitle)
	hwnd := win.FindWindow(nil, title)
	if hwnd == 0 {
		return
	}
	if win.IsIconic(hwnd) {
		win.ShowWindow(hwnd, win.SW_RESTORE)
	}
	win.SetForegroundWindow(hwnd)
}

// netsh runs netsh without flashing a console window.
func netsh(args ...string) (string, error) {
	cmd := exec.Command("netsh", args...)
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: windows.CREATE_NO_WINDOW}
	out, err := cmd.CombinedOutput()
	return strings.TrimSpace(string(out)), err
}

// ensureFirewallRule adds the inbound rule for port unless it already exists.
func ensureFirewallRule(port int) error {
	if _, err := netsh("advfirewall", "firewall", "show", "rule", "name="+core.FirewallRuleName(port)); err == nil {
		return nil
	}
	if out, err := netsh(core.FirewallArgs(port)...); err != nil {
		return fmt.Errorf("could not open port %d in Windows Firewall: %v %s", port, err, out)
	}
	return nil
}

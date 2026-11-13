package main

import (
	"fmt"
	"time"

	"github.com/lxn/walk"
	. "github.com/lxn/walk/declarative"

	"server-gui/core"
	"server/guardian"
)

const refreshEvery = 3 * time.Second

var modes = []string{"blacklist", "whitelist"}
var modeLabels = []string{"Blacklist", "Whitelist"}

type app struct {
	mw *walk.MainWindow

	dir         string
	hostname    string
	ip          string
	settings    core.Settings
	runner      *core.Runner
	client      *core.Client
	startErr    string
	firewallErr string

	stateLabel  *walk.Label
	syncLabel   *walk.Label
	statusLine  *walk.Label
	pcHeader    *walk.Label
	protection  *walk.CheckBox
	modeCombo   *walk.ComboBox
	appsTable   *walk.TableView
	pcsTable    *walk.TableView
	apps        *appModel
	pcs         *pcModel
	updating    bool // true while apply() sets widgets, so their events are ignored
	currentMode string
	stopped     bool
}

// snapshot is one refresh's worth of server state, fetched off the UI thread.
type snapshot struct {
	status guardian.StatusResponse
	apps   []guardian.Application
	pcs    []guardian.Computer
	now    time.Time
	err    error
}

func (a *app) running() bool { return a.client != nil }

func (a *app) build() error {
	a.apps = &appModel{a: a}
	a.pcs = &pcModel{}
	a.currentMode = modes[0]

	err := MainWindow{
		AssignTo: &a.mw,
		Title:    appTitle,
		MinSize:  Size{Width: 820, Height: 520},
		Size:     Size{Width: 960, Height: 620},
		Layout:   VBox{},
		Children: []Widget{
			Composite{
				Layout: HBox{MarginsZero: true},
				Children: []Widget{
					Label{AssignTo: &a.stateLabel, Font: Font{Bold: true}},
					HSpacer{Size: 12},
					CheckBox{AssignTo: &a.protection, Text: "Protection on", OnCheckedChanged: a.onStatusChanged},
					Label{Text: "Mode:"},
					ComboBox{AssignTo: &a.modeCombo, Model: modeLabels, CurrentIndex: 0, OnCurrentIndexChanged: a.onStatusChanged},
					HSpacer{},
					PushButton{Text: "Block all PCs", OnClicked: func() { a.act(a.client.BlockAll) }},
					PushButton{Text: "Unblock all", OnClicked: func() { a.act(a.client.UnblockAll) }},
					PushButton{Text: "Connect…", OnClicked: a.showConnect},
				},
			},
			Label{AssignTo: &a.syncLabel},
			HSplitter{
				Children: []Widget{
					Composite{
						Layout: VBox{MarginsZero: true},
						Children: []Widget{
							Composite{
								Layout: HBox{MarginsZero: true},
								Children: []Widget{
									Label{Text: "Applications", Font: Font{Bold: true}},
									HSpacer{},
									PushButton{Text: "+ Add", OnClicked: a.addApp},
									PushButton{Text: "Remove", OnClicked: a.removeApp},
								},
							},
							TableView{
								AssignTo:   &a.appsTable,
								Model:      a.apps,
								CheckBoxes: true,
								Columns: []TableViewColumn{
									{Title: "Application", Width: 260},
									{Title: "Mode", Width: 100},
								},
							},
						},
					},
					Composite{
						Layout: VBox{MarginsZero: true},
						Children: []Widget{
							Composite{
								Layout: HBox{MarginsZero: true},
								Children: []Widget{
									Label{Text: "Computers", Font: Font{Bold: true}},
									Label{AssignTo: &a.pcHeader},
									HSpacer{},
									PushButton{Text: "Block", OnClicked: func() { a.setSelectedBlocked(true) }},
									PushButton{Text: "Unblock", OnClicked: func() { a.setSelectedBlocked(false) }},
								},
							},
							TableView{
								AssignTo:        &a.pcsTable,
								Model:           a.pcs,
								OnItemActivated: a.togglePC,
								MultiSelection:  false,
								Columns: []TableViewColumn{
									{Title: "Computer", Width: 140},
									{Title: "State", Width: 140},
								},
							},
						},
					},
				},
			},
			Label{AssignTo: &a.statusLine},
		},
	}.Create()
	if err != nil {
		return err
	}
	if icon := appIcon(); icon != nil {
		a.mw.SetIcon(icon)
	}
	a.mw.Closing().Attach(func(canceled *bool, reason walk.CloseReason) { a.stop() })

	if !a.running() {
		a.stateLabel.SetText("● Server stopped: " + a.startErr)
		a.stateLabel.SetTextColor(walk.RGB(0xa4, 0x26, 0x2c))
		for _, w := range []walk.Widget{a.protection, a.modeCombo, a.appsTable, a.pcsTable} {
			w.SetEnabled(false)
		}
	} else {
		a.stateLabel.SetText(fmt.Sprintf("● Server running on port %d", a.runner.Port()))
		a.stateLabel.SetTextColor(walk.RGB(0x10, 0x7c, 0x10))
	}
	if a.firewallErr != "" {
		a.statusLine.SetText("⚠ " + a.firewallErr)
	}
	return nil
}

// stop shuts the embedded server down; safe to call more than once.
func (a *app) stop() {
	if a.runner != nil && !a.stopped {
		a.stopped = true
		a.runner.Stop()
	}
}

func (a *app) startRefresh() {
	if !a.running() {
		return
	}
	go func() {
		for {
			s := a.fetch()
			a.mw.Synchronize(func() { a.apply(s) })
			time.Sleep(refreshEvery)
			if a.stopped {
				return
			}
		}
	}()
}

func (a *app) fetch() snapshot {
	var s snapshot
	if s.status, s.err = a.client.Status(); s.err != nil {
		return s
	}
	if s.apps, s.err = a.client.Apps(); s.err != nil {
		return s
	}
	s.pcs, s.now, s.err = a.client.Computers()
	return s
}

// apply shows a snapshot; it runs on the UI thread.
func (a *app) apply(s snapshot) {
	if s.err != nil {
		a.statusLine.SetText("Could not read from the server: " + s.err.Error())
		return
	}
	if a.firewallErr == "" {
		a.statusLine.SetText("")
	}

	a.updating = true
	a.protection.SetChecked(s.status.Enabled)
	for i, m := range modes {
		if m == s.status.Mode {
			a.modeCombo.SetCurrentIndex(i)
			a.currentMode = m
		}
	}
	core.SortApps(s.apps)
	a.apps.set(s.apps)
	a.updating = false

	a.pcs.set(s.pcs, s.now)
	a.pcHeader.SetText(core.OnlineSummary(s.pcs, s.now))

	switch last, ok := core.LastSync(s.pcs); {
	case core.NoAgentWarning(a.runner.StartedAt(), s.pcs, s.now):
		a.syncLabel.SetText(fmt.Sprintf("⚠ No agent has connected for 2 minutes. Agents may not find %s — try the IP address under Connect…", a.hostname))
	case ok:
		a.syncLabel.SetText("Last agent sync: " + agoText(s.now.Sub(last)))
	default:
		a.syncLabel.SetText("Waiting for the first agent…")
	}
}

func agoText(d time.Duration) string {
	if d < 0 {
		d = 0
	}
	switch {
	case d < time.Minute:
		return fmt.Sprintf("%d s ago", int(d.Seconds()))
	case d < time.Hour:
		return fmt.Sprintf("%d min ago", int(d.Minutes()))
	default:
		return fmt.Sprintf("%d h ago", int(d.Hours()))
	}
}

// act runs one change against the server, then refreshes.
func (a *app) act(f func() error) {
	if !a.running() {
		return
	}
	if err := f(); err != nil {
		a.statusLine.SetText("⚠ " + err.Error())
	}
	a.apply(a.fetch())
}

func (a *app) onStatusChanged() {
	if a.updating || !a.running() {
		return
	}
	i := a.modeCombo.CurrentIndex()
	if i < 0 || i >= len(modes) {
		return
	}
	a.act(func() error { return a.client.SetStatus(a.protection.Checked(), modes[i]) })
}

func (a *app) addApp() {
	if !a.running() {
		return
	}
	text, ok := inputDialog(a.mw, "Add application", fmt.Sprintf("Process name to add to the %s:", a.currentMode))
	if !ok {
		return
	}
	name, err := core.ValidateAppName(text)
	if err != nil {
		walk.MsgBox(a.mw, appTitle, err.Error(), walk.MsgBoxIconWarning)
		return
	}
	mode := a.currentMode
	a.act(func() error { return a.client.AddApp(name, mode) })
}

func (a *app) removeApp() {
	i := a.appsTable.CurrentIndex()
	if !a.running() || i < 0 || i >= len(a.apps.rows) {
		return
	}
	app := a.apps.rows[i]
	if walk.MsgBox(a.mw, appTitle, fmt.Sprintf("Remove %s from the %s?", app.Name, app.Mode), walk.MsgBoxYesNo|walk.MsgBoxIconQuestion) != walk.DlgCmdYes {
		return
	}
	a.act(func() error { return a.client.RemoveApp(app.Name, app.Mode) })
}

func (a *app) setSelectedBlocked(blocked bool) {
	i := a.pcsTable.CurrentIndex()
	if !a.running() || i < 0 || i >= len(a.pcs.rows) {
		return
	}
	id := a.pcs.rows[i].Identity
	a.act(func() error { return a.client.SetBlocked(id, blocked) })
}

func (a *app) togglePC() {
	i := a.pcsTable.CurrentIndex()
	if i < 0 || i >= len(a.pcs.rows) {
		return
	}
	a.setSelectedBlocked(!a.pcs.rows[i].Blocked)
}

// appModel backs the Applications table; the check box is "enabled".
type appModel struct {
	walk.TableModelBase
	a    *app
	rows []guardian.Application
}

func (m *appModel) set(rows []guardian.Application) {
	m.rows = rows
	m.PublishRowsReset()
}

func (m *appModel) RowCount() int { return len(m.rows) }

func (m *appModel) Value(row, col int) interface{} {
	if row < 0 || row >= len(m.rows) {
		return ""
	}
	if col == 0 {
		return m.rows[row].Name
	}
	return m.rows[row].Mode
}

func (m *appModel) Checked(row int) bool {
	return row >= 0 && row < len(m.rows) && m.rows[row].Enabled
}

func (m *appModel) SetChecked(row int, checked bool) error {
	if m.a.updating || row < 0 || row >= len(m.rows) || !m.a.running() {
		return nil
	}
	app := m.rows[row]
	m.a.act(func() error { return m.a.client.SetAppEnabled(app.Name, app.Mode, checked) })
	return nil
}

// pcModel backs the Computers table.
type pcModel struct {
	walk.TableModelBase
	rows []guardian.Computer
	now  time.Time
}

func (m *pcModel) set(rows []guardian.Computer, now time.Time) {
	m.rows, m.now = rows, now
	m.PublishRowsReset()
}

func (m *pcModel) RowCount() int { return len(m.rows) }

func (m *pcModel) Value(row, col int) interface{} {
	if row < 0 || row >= len(m.rows) {
		return ""
	}
	c := m.rows[row]
	if col == 0 {
		return fmt.Sprintf("PC %d", c.Identity)
	}
	online := core.IsOnline(c, m.now)
	switch {
	case !online && c.Blocked:
		return "offline (blocked)"
	case !online:
		return "offline"
	case c.Blocked:
		return "● blocked"
	default:
		return "● online"
	}
}

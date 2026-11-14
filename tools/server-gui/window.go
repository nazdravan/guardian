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

// modeLabels say what each mode does instead of naming it.
var modeLabels = []string{"Block the listed apps", "Allow only the listed apps"}

// Palette: soft backgrounds with a dark text of the same hue, so the state
// card and the banner read at a glance but stay calm.
var (
	greenBg  = walk.RGB(0xe6, 0xf4, 0xea)
	greenFg  = walk.RGB(0x13, 0x73, 0x33)
	amberBg  = walk.RGB(0xfe, 0xf3, 0xd6)
	amberFg  = walk.RGB(0x7a, 0x4a, 0x00)
	redBg    = walk.RGB(0xfc, 0xe8, 0xe6)
	redFg    = walk.RGB(0xa5, 0x0e, 0x0e)
	grayBg   = walk.RGB(0xe8, 0xea, 0xed)
	windowBg = walk.RGB(0xf0, 0xf0, 0xf0)
	grayFg   = walk.RGB(0x70, 0x75, 0x7a)
	uiFont   = Font{Family: "Segoe UI", PointSize: 9}
	headFont = Font{Family: "Segoe UI", PointSize: 13, Bold: true}
)

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

	// State card
	card       *walk.Composite
	cardTitle  *walk.Label
	cardText   *walk.Label
	protectBtn *walk.PushButton
	cardBg     walk.Color
	cardBgSet  bool

	modeCombo *walk.ComboBox
	classBtns []*walk.PushButton

	// Warning banner
	banner        *walk.Composite
	bannerText    *walk.Label
	bannerConnect *walk.PushButton

	appsBox   *walk.GroupBox
	appsHint  *walk.Label
	appsTable *walk.TableView
	removeBtn *walk.PushButton
	addBtn    *walk.PushButton

	pcsBox   *walk.GroupBox
	pcsHint  *walk.Label
	pcsTable *walk.TableView
	pcBtn    *walk.PushButton

	sbServer *walk.StatusBarItem
	sbSync   *walk.StatusBarItem

	apps        *appModel
	pcs         *pcModel
	updating    bool // true while apply() sets widgets, so their events are ignored
	enabled     bool
	currentMode string
	readErr     string
	noAgent     bool
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

	// The builder applies CurrentIndex and other properties at the end of
	// Create, which fires OnCurrentIndexChanged; without this guard every
	// launch would send SetStatus(false, "blacklist") and switch protection off.
	a.updating = true
	defer func() { a.updating = false }()

	var blockAll, unblockAll *walk.PushButton

	err := MainWindow{
		AssignTo: &a.mw,
		Title:    appTitle,
		Font:     uiFont,
		// An explicit background, even the default grey, is what makes walk
		// honour a Label's TextColor: without one the system colour wins.
		Background: SolidColorBrush{Color: windowBg},
		MinSize:    Size{Width: 860, Height: 560},
		Size:       Size{Width: 1000, Height: 680},
		Layout:     VBox{Margins: Margins{Left: 12, Top: 12, Right: 12, Bottom: 8}, Spacing: 10},
		Children: []Widget{
			// 1. State card: is the class protected right now?
			Composite{
				AssignTo: &a.card,
				Layout:   HBox{Margins: Margins{Left: 16, Top: 12, Right: 16, Bottom: 12}, Spacing: 12},
				Children: []Widget{
					Composite{
						Layout: VBox{MarginsZero: true, Spacing: 2},
						Children: []Widget{
							Label{AssignTo: &a.cardTitle, Font: headFont},
							Label{AssignTo: &a.cardText},
						},
					},
					HSpacer{},
					PushButton{
						AssignTo:    &a.protectBtn,
						MinSize:     Size{Width: 170, Height: 34},
						OnClicked:   func() { a.setStatus(!a.enabled, a.currentMode) },
						ToolTipText: "Switches the rules on or off for every computer. The lists are kept.",
					},
				},
			},
			// 2. Class controls
			Composite{
				Layout: HBox{MarginsZero: true, Spacing: 8},
				Children: []Widget{
					Label{Text: "Rule:"},
					ComboBox{
						AssignTo:              &a.modeCombo,
						Model:                 modeLabels,
						CurrentIndex:          0,
						MinSize:               Size{Width: 190},
						OnCurrentIndexChanged: a.onModeChanged,
						ToolTipText:           "Block: the listed apps are closed.\nAllow only: everything except the listed apps (and Windows itself) is closed.",
					},
					HSpacer{},
					Label{Text: "All computers:", TextColor: grayFg},
					PushButton{AssignTo: &blockAll, Text: "Block all", OnClicked: func() { a.act(a.client.BlockAll) },
						ToolTipText: "Apply the rules to every computer"},
					PushButton{AssignTo: &unblockAll, Text: "Unblock all", OnClicked: func() { a.act(a.client.UnblockAll) },
						ToolTipText: "Let every computer run anything"},
				},
			},
			// 3. Warning banner, hidden unless something needs the teacher.
			Composite{
				AssignTo: &a.banner,
				Visible:  false,
				Layout:   HBox{Margins: Margins{Left: 12, Top: 8, Right: 8, Bottom: 8}, Spacing: 8},
				Children: []Widget{
					Label{AssignTo: &a.bannerText, TextColor: amberFg},
					HSpacer{},
					PushButton{AssignTo: &a.bannerConnect, Text: "Connection info…", OnClicked: a.showConnect},
				},
			},
			// 4. The two lists
			HSplitter{
				HandleWidth: 10,
				Children: []Widget{
					GroupBox{
						AssignTo: &a.appsBox,
						Title:    "Applications",
						Layout:   VBox{Margins: Margins{Left: 10, Top: 8, Right: 10, Bottom: 10}, Spacing: 8},
						Children: []Widget{
							Composite{
								Layout: HBox{MarginsZero: true, Spacing: 8},
								Children: []Widget{
									PushButton{AssignTo: &a.addBtn, Text: "+ Add application…", OnClicked: a.addApp},
									PushButton{AssignTo: &a.removeBtn, Text: "Remove", Enabled: false, OnClicked: a.removeApp,
										ToolTipText: "Remove the selected application (Delete)"},
									HSpacer{},
								},
							},
							Label{AssignTo: &a.appsHint, TextColor: grayFg, Visible: false},
							TableView{
								AssignTo:              &a.appsTable,
								Model:                 a.apps,
								CheckBoxes:            true,
								LastColumnStretched:   true,
								OnCurrentIndexChanged: a.updateButtons,
								OnKeyDown: func(key walk.Key) {
									if key == walk.KeyDelete {
										a.removeApp()
									}
								},
								StyleCell:   a.styleApp,
								ToolTipText: "Untick an application to pause its rule without removing it.",
								Columns: []TableViewColumn{
									{Title: "Application", Width: 240},
									{Title: "Rule"},
								},
							},
						},
					},
					GroupBox{
						AssignTo: &a.pcsBox,
						Title:    "Computers",
						Layout:   VBox{Margins: Margins{Left: 10, Top: 8, Right: 10, Bottom: 10}, Spacing: 8},
						Children: []Widget{
							Composite{
								Layout: HBox{MarginsZero: true, Spacing: 8},
								Children: []Widget{
									PushButton{AssignTo: &a.pcBtn, Text: "Block", Enabled: false, MinSize: Size{Width: 110},
										OnClicked: a.togglePC, ToolTipText: "Block or unblock the selected computer (or double-click it)"},
									HSpacer{},
									// Here rather than in the top row: it is how computers get into this list.
									PushButton{Text: "Connection info…", OnClicked: a.showConnect,
										ToolTipText: "What to put in each student computer's agent.env, and the phone's address"},
								},
							},
							Label{AssignTo: &a.pcsHint, TextColor: grayFg, Visible: false},
							TableView{
								AssignTo:              &a.pcsTable,
								Model:                 a.pcs,
								OnItemActivated:       a.togglePC,
								OnCurrentIndexChanged: a.updateButtons,
								MultiSelection:        false,
								LastColumnStretched:   true,
								StyleCell:             a.stylePC,
								Columns: []TableViewColumn{
									{Title: "Computer", Width: 90},
									{Title: "Connection", Width: 170},
									{Title: "Rules"},
								},
							},
						},
					},
				},
			},
		},
		StatusBarItems: []StatusBarItem{
			{AssignTo: &a.sbServer, Width: 420},
			{AssignTo: &a.sbSync, Width: 260},
		},
	}.Create()
	if err != nil {
		return err
	}
	a.classBtns = []*walk.PushButton{blockAll, unblockAll}
	a.pcs.table = a.pcsTable
	if icon := appIcon(); icon != nil {
		a.mw.SetIcon(icon)
	}
	a.mw.Closing().Attach(func(canceled *bool, reason walk.CloseReason) { a.stop() })
	if b, err := walk.NewSolidColorBrush(amberBg); err == nil {
		a.banner.SetBackground(b)
	}

	if !a.running() {
		a.setCard(redBg, redFg, "Server is not running", a.startErr)
		a.protectBtn.SetVisible(false)
		for _, w := range []walk.Widget{a.modeCombo, a.appsTable, a.pcsTable, a.addBtn, a.classBtns[0], a.classBtns[1]} {
			w.SetEnabled(false)
		}
		a.sbServer.SetText("Server stopped")
	} else {
		a.setCard(grayBg, grayFg, "Starting…", "")
		a.sbServer.SetText(fmt.Sprintf("Server running on port %d  ·  http://%s:%d", a.runner.Port(), a.hostname, a.runner.Port()))
	}
	a.showBanner()
	a.showLists(nil, nil, time.Time{})
	return nil
}

func (a *app) setCard(bg, fg walk.Color, title, text string) {
	// Only when the colour changes, and always with a new brush: walk disposes
	// a brush as soon as it is detached from its last window, so setting the
	// same brush again on every refresh would leave the card with a dead one.
	if !a.cardBgSet || bg != a.cardBg {
		if b, err := walk.NewSolidColorBrush(bg); err == nil {
			a.card.SetBackground(b)
			a.cardBg, a.cardBgSet = bg, true
		}
	}
	a.cardTitle.SetTextColor(fg)
	a.cardTitle.SetText(title)
	a.cardText.SetText(text)
	a.cardText.SetVisible(text != "")
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
		a.readErr = "Could not read from the server: " + s.err.Error()
		a.showBanner()
		return
	}
	a.readErr = ""

	a.updating = true
	a.enabled = s.status.Enabled
	for i, m := range modes {
		if m == s.status.Mode {
			a.modeCombo.SetCurrentIndex(i)
			a.currentMode = m
		}
	}
	apps := core.AppsForMode(s.apps, a.currentMode)
	core.SortApps(apps)
	a.apps.set(apps)
	a.updating = false

	a.pcs.set(s.pcs, s.now)
	a.showCard(apps, s.pcs)
	a.showLists(apps, s.pcs, s.now)

	switch last, ok := core.LastSync(s.pcs); {
	case ok:
		a.sbSync.SetText("Last agent sync: " + agoText(s.now.Sub(last)))
	default:
		a.sbSync.SetText("Waiting for the first agent…")
	}
	a.noAgent = core.NoAgentWarning(a.runner.StartedAt(), s.pcs, s.now)
	a.showBanner()
	a.updateButtons()
}

// showCard explains in one sentence what the agents are doing right now.
func (a *app) showCard(apps []guardian.Application, pcs []guardian.Computer) {
	n := core.CountEnabled(apps)
	blocked := 0
	for _, c := range pcs {
		if c.Blocked {
			blocked++
		}
	}
	on := fmt.Sprintf("%d of %d computers follow the rules.", blocked, len(pcs))
	if len(pcs) == 0 {
		on = "No student computer has connected yet."
	}
	if !a.enabled {
		a.setCard(amberBg, amberFg, "Protection is off",
			"Students can run any program. The lists and blocks are kept for when you turn it back on.")
		a.protectBtn.SetText("Turn protection on")
		return
	}
	var what string
	switch {
	case a.currentMode == "whitelist":
		what = fmt.Sprintf("Only %s may run on blocked computers.", plural(n, "application"))
	case n == 0:
		what = "No application is blocked yet: add one below."
	default:
		what = fmt.Sprintf("%s closed on blocked computers.", capitalize(plural(n, "application")+pluralVerb(n)))
	}
	a.setCard(greenBg, greenFg, "Protection is on", what+"  "+on)
	a.protectBtn.SetText("Turn protection off")
}

// showLists sets the group titles and swaps an empty table for a hint.
func (a *app) showLists(apps []guardian.Application, pcs []guardian.Computer, now time.Time) {
	title, hint := "Blocked applications", "No blocked applications yet.\nClick “+ Add application…” and type a process name, for example chrome.exe."
	if a.currentMode == "whitelist" {
		title, hint = "Allowed applications", "No allowed applications yet.\nWith protection on, blocked computers can run only Windows itself.\nClick “+ Add application…” to allow one, for example winword.exe."
	}
	if !a.running() {
		hint = "The server is not running."
	}
	a.appsBox.SetTitle(fmt.Sprintf("%s  (%d)", title, len(apps)))
	a.appsHint.SetText(hint)
	// The tables stay visible even when empty: they are what takes the
	// window's spare height, and their headers show what will appear there.
	a.appsHint.SetVisible(len(apps) == 0)

	a.pcsBox.SetTitle("Computers  (" + core.OnlineSummary(pcs, now) + ")")
	a.pcsHint.SetText("No student computer has connected yet.\nClick “Connection info…” to see the two lines to put in each computer's agent.env.")
	if !a.running() {
		a.pcsHint.SetText("The server is not running.")
	}
	a.pcsHint.SetVisible(len(pcs) == 0)
}

// showBanner shows the most important warning, if any.
func (a *app) showBanner() {
	text, connect := "", false
	switch {
	case a.firewallErr != "":
		text = "⚠  " + a.firewallErr
	case a.readErr != "":
		text = "⚠  " + a.readErr
	case a.noAgent:
		text = fmt.Sprintf("⚠  No student computer has connected for 2 minutes. They may not find “%s” by name — try the IP address under Connection info.", a.hostname)
		connect = true
	}
	a.bannerText.SetText(text)
	a.bannerConnect.SetVisible(connect)
	a.banner.SetVisible(text != "")
}

// updateButtons enables the list buttons only when they have something to act on.
func (a *app) updateButtons() {
	i := a.appsTable.CurrentIndex()
	a.removeBtn.SetEnabled(a.running() && i >= 0 && i < len(a.apps.rows))

	j := a.pcsTable.CurrentIndex()
	if !a.running() || j < 0 || j >= len(a.pcs.rows) {
		a.pcBtn.SetEnabled(false)
		a.pcBtn.SetText("Block")
		return
	}
	c := a.pcs.rows[j]
	a.pcBtn.SetEnabled(true)
	if c.Blocked {
		a.pcBtn.SetText(fmt.Sprintf("Unblock PC %d", c.Identity))
	} else {
		a.pcBtn.SetText(fmt.Sprintf("Block PC %d", c.Identity))
	}
}

func plural(n int, word string) string {
	if n == 1 {
		return "1 " + word
	}
	return fmt.Sprintf("%d %ss", n, word)
}

func pluralVerb(n int) string {
	if n == 1 {
		return " is"
	}
	return " are"
}

func capitalize(s string) string {
	if s == "" || s[0] < 'a' || s[0] > 'z' {
		return s
	}
	return string(s[0]-32) + s[1:]
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
	case d < 48*time.Hour:
		return fmt.Sprintf("%d h ago", int(d.Hours()))
	default:
		return fmt.Sprintf("%d days ago", int(d.Hours()/24))
	}
}

// act runs one change against the server, then refreshes.
func (a *app) act(f func() error) {
	if !a.running() {
		return
	}
	if err := f(); err != nil {
		walk.MsgBox(a.mw, appTitle, err.Error(), walk.MsgBoxIconWarning)
	}
	a.apply(a.fetch())
}

func (a *app) setStatus(enabled bool, mode string) {
	a.act(func() error { return a.client.SetStatus(enabled, mode) })
}

func (a *app) onModeChanged() {
	if a.updating || !a.running() {
		return
	}
	i := a.modeCombo.CurrentIndex()
	if i < 0 || i >= len(modes) {
		return
	}
	a.setStatus(a.enabled, modes[i])
}

func (a *app) addApp() {
	if !a.running() {
		return
	}
	prompt := "Process name to block:"
	if a.currentMode == "whitelist" {
		prompt = "Process name to allow:"
	}
	text, ok := inputDialog(a.mw, "Add application", prompt)
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
	if walk.MsgBox(a.mw, appTitle, fmt.Sprintf("Remove %s from the list?", app.Name), walk.MsgBoxYesNo|walk.MsgBoxIconQuestion) != walk.DlgCmdYes {
		return
	}
	a.act(func() error { return a.client.RemoveApp(app.Name, app.Mode) })
}

func (a *app) togglePC() {
	i := a.pcsTable.CurrentIndex()
	if !a.running() || i < 0 || i >= len(a.pcs.rows) {
		return
	}
	c := a.pcs.rows[i]
	a.act(func() error { return a.client.SetBlocked(c.Identity, !c.Blocked) })
}

func (a *app) styleApp(style *walk.CellStyle) {
	row := style.Row()
	if row < 0 || row >= len(a.apps.rows) {
		return
	}
	if !a.apps.rows[row].Enabled {
		style.TextColor = grayFg
	}
}

func (a *app) stylePC(style *walk.CellStyle) {
	row := style.Row()
	if row < 0 || row >= len(a.pcs.rows) {
		return
	}
	c := a.pcs.rows[row]
	switch style.Col() {
	case 1:
		if core.IsOnline(c, a.pcs.now) {
			style.TextColor = greenFg
		} else {
			style.TextColor = grayFg
		}
	case 2:
		if c.Blocked {
			style.TextColor = redFg
		}
	}
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
	// A reset with an unchanged row count does not repaint by itself.
	if m.a.appsTable != nil {
		m.a.appsTable.Invalidate()
	}
}

// ID keeps the selection across the 3 s refresh (walk.IDProvider).
func (m *appModel) ID(row int) interface{} {
	if row < 0 || row >= len(m.rows) {
		return nil
	}
	return m.rows[row].Mode + "\x00" + m.rows[row].Name
}

func (m *appModel) RowCount() int { return len(m.rows) }

func (m *appModel) Value(row, col int) interface{} {
	if row < 0 || row >= len(m.rows) {
		return ""
	}
	r := m.rows[row]
	if col == 0 {
		return r.Name
	}
	switch {
	case !r.Enabled:
		return "Paused"
	case r.Mode == "whitelist":
		return "Allowed"
	default:
		return "Blocked"
	}
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
	table *walk.TableView
	rows  []guardian.Computer
	now   time.Time
}

func (m *pcModel) set(rows []guardian.Computer, now time.Time) {
	m.rows, m.now = rows, now
	m.PublishRowsReset()
	// A reset with an unchanged row count does not repaint by itself.
	if m.table != nil {
		m.table.Invalidate()
	}
}

// ID keeps the selection across the 3 s refresh (walk.IDProvider).
func (m *pcModel) ID(row int) interface{} {
	if row < 0 || row >= len(m.rows) {
		return nil
	}
	return m.rows[row].Identity
}

func (m *pcModel) RowCount() int { return len(m.rows) }

func (m *pcModel) Value(row, col int) interface{} {
	if row < 0 || row >= len(m.rows) {
		return ""
	}
	c := m.rows[row]
	switch col {
	case 0:
		return fmt.Sprintf("PC %d", c.Identity)
	case 1:
		if core.IsOnline(c, m.now) {
			return "● Online"
		}
		if d, ok := core.LastSeen(c, m.now); ok {
			return "Offline · " + agoText(d)
		}
		return "Offline"
	default:
		if c.Blocked {
			return "Blocked"
		}
		return "Free"
	}
}

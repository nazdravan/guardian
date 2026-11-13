package main

import (
	"github.com/lxn/walk"
	. "github.com/lxn/walk/declarative"

	"server-gui/core"
)

// copyRow is one "label: value [Copy]" line of the Connect window.
func copyRow(label string, edit **walk.LineEdit, value string, password bool) []Widget {
	return []Widget{
		Label{Text: label},
		LineEdit{AssignTo: edit, Text: value, ReadOnly: true, PasswordMode: password},
		PushButton{Text: "Copy", OnClicked: func() { walk.Clipboard().SetText(value) }},
	}
}

func (a *app) showConnect() {
	ci := core.NewConnectInfo(a.hostname, a.ip, a.settings)
	var dlg *walk.Dialog
	var agentLine, tokenLine, byName, byIP, admin *walk.LineEdit
	var closeBtn *walk.PushButton

	rows := []Widget{}
	rows = append(rows, copyRow("agent.env line:", &agentLine, ci.AgentLine, false)...)
	rows = append(rows, copyRow("agent.env line:", &tokenLine, ci.TokenLine, false)...)
	rows = append(rows, copyRow("Phone address:", &byName, ci.PhoneByName, false)...)
	rows = append(rows, copyRow("Phone address (IP):", &byIP, ci.PhoneByIP, false)...)
	rows = append(rows, copyRow("Phone ADMIN_TOKEN:", &admin, ci.AdminToken, true)...)

	Dialog{
		AssignTo:     &dlg,
		Title:        "Connect agents and the phone",
		MinSize:      Size{Width: 640, Height: 300},
		Layout:       VBox{},
		CancelButton: &closeBtn,
		Children: []Widget{
			Label{Text: "On each student computer, put these two lines into the agent's agent.env:"},
			Composite{Layout: Grid{Columns: 3, MarginsZero: true}, Children: rows},
			Label{Text: "The IP address can change from day to day; prefer the computer name for agents.\n" +
				"Android usually cannot find a Windows computer by name, so the phone may need the IP address.\n" +
				"Do not install an agent on this computer."},
			Composite{
				Layout: HBox{MarginsZero: true},
				Children: []Widget{
					PushButton{Text: "Show ADMIN_TOKEN", OnClicked: func() { admin.SetPasswordMode(false) }},
					HSpacer{},
					PushButton{AssignTo: &closeBtn, Text: "Close", OnClicked: func() { dlg.Cancel() }},
				},
			},
		},
	}.Run(a.mw)
}

// inputDialog asks for a single line of text; walk has no built-in input box.
func inputDialog(owner walk.Form, title, prompt string) (string, bool) {
	var dlg *walk.Dialog
	var edit *walk.LineEdit
	var okBtn, cancelBtn *walk.PushButton
	var result string

	cmd, err := Dialog{
		AssignTo:      &dlg,
		Title:         title,
		MinSize:       Size{Width: 420, Height: 140},
		Layout:        VBox{},
		DefaultButton: &okBtn,
		CancelButton:  &cancelBtn,
		Children: []Widget{
			Label{Text: prompt},
			LineEdit{AssignTo: &edit, CueBanner: "for example: chrome.exe"},
			Composite{
				Layout: HBox{MarginsZero: true},
				Children: []Widget{
					HSpacer{},
					PushButton{AssignTo: &okBtn, Text: "Add", OnClicked: func() {
						result = edit.Text()
						dlg.Accept()
					}},
					PushButton{AssignTo: &cancelBtn, Text: "Cancel", OnClicked: func() { dlg.Cancel() }},
				},
			},
		},
	}.Run(owner)
	if err != nil || cmd != walk.DlgCmdOK {
		return "", false
	}
	return result, true
}

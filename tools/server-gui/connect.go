package main

import (
	"github.com/lxn/walk"
	. "github.com/lxn/walk/declarative"

	"server-gui/core"
)

// copyRow is one "label: value [Copy]" line of the Connection info window.
func copyRow(label string, edit **walk.LineEdit, value string, password bool) []Widget {
	return []Widget{
		Label{Text: label},
		LineEdit{AssignTo: edit, Text: value, ReadOnly: true, PasswordMode: password},
		PushButton{Text: "Copy", OnClicked: func() {
			walk.Clipboard().SetText(value)
			(*edit).SetFocus()
			(*edit).SetTextSelection(0, -1)
		}},
	}
}

func (a *app) showConnect() {
	ci := core.NewConnectInfo(a.hostname, a.ip, a.settings)
	var dlg *walk.Dialog
	var agentLine, tokenLine, byName, byIP, admin *walk.LineEdit
	var closeBtn, showBtn *walk.PushButton

	agents := []Widget{}
	agents = append(agents, copyRow("Server address:", &agentLine, ci.AgentLine, false)...)
	agents = append(agents, copyRow("Token:", &tokenLine, ci.TokenLine, false)...)

	phone := []Widget{}
	phone = append(phone, copyRow("Address:", &byIP, ci.PhoneByIP, false)...)
	phone = append(phone, copyRow("Address (by name):", &byName, ci.PhoneByName, false)...)
	phone = append(phone, copyRow("ADMIN_TOKEN:", &admin, ci.AdminToken, true)...)

	Dialog{
		AssignTo:     &dlg,
		Title:        "Connection info",
		Font:         uiFont,
		Background:   SolidColorBrush{Color: windowBg},
		MinSize:      Size{Width: 680, Height: 380},
		Layout:       VBox{Margins: Margins{Left: 14, Top: 14, Right: 14, Bottom: 12}, Spacing: 10},
		CancelButton: &closeBtn,
		Children: []Widget{
			GroupBox{
				Title:  "1. Student computers",
				Layout: VBox{Margins: Margins{Left: 10, Top: 8, Right: 10, Bottom: 10}, Spacing: 6},
				Children: []Widget{
					Label{Text: "Put these two lines into agent.env on every student computer, then restart its agent:"},
					Composite{Layout: Grid{Columns: 3, MarginsZero: true, Spacing: 6}, Children: agents},
					Label{TextColor: grayFg, Text: "The computer name stays the same even when the router gives this computer a new IP address.\n" +
						"Do not install an agent on this computer: it would show up as a student PC."},
				},
			},
			GroupBox{
				Title:  "2. Android app (optional)",
				Layout: VBox{Margins: Margins{Left: 10, Top: 8, Right: 10, Bottom: 10}, Spacing: 6},
				Children: []Widget{
					Label{Text: "In the Guardian app's Settings, enter the address and ADMIN_TOKEN:"},
					Composite{Layout: Grid{Columns: 3, MarginsZero: true, Spacing: 6}, Children: phone},
					Label{TextColor: grayFg, Text: "Android usually cannot find a Windows computer by name, so use the IP address. It may change from day to day."},
				},
			},
			Composite{
				Layout: HBox{MarginsZero: true},
				Children: []Widget{
					PushButton{AssignTo: &showBtn, Text: "Show ADMIN_TOKEN", OnClicked: func() {
						hidden := admin.PasswordMode()
						admin.SetPasswordMode(!hidden)
						if hidden {
							showBtn.SetText("Hide ADMIN_TOKEN")
						} else {
							showBtn.SetText("Show ADMIN_TOKEN")
						}
					}},
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
		Font:          uiFont,
		MinSize:       Size{Width: 440, Height: 150},
		Layout:        VBox{Margins: Margins{Left: 14, Top: 14, Right: 14, Bottom: 12}, Spacing: 8},
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

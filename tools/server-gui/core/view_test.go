package core

import (
	"testing"
	"time"

	"server/guardian"
)

var t0 = time.Date(2026, 11, 13, 10, 0, 0, 0, time.UTC)

func pc(id int, ago time.Duration) guardian.Computer {
	return guardian.Computer{Identity: id, DateTime: t0.Add(-ago).Format(time.RFC3339)}
}

func TestValidateAppName(t *testing.T) {
	if got, err := ValidateAppName(" chrome.exe "); err != nil || got != "chrome.exe" {
		t.Errorf(`ValidateAppName(" chrome.exe ") = %q, %v`, got, err)
	}
	for _, bad := range []string{"a", "  ", "ab"} {
		if _, err := ValidateAppName(bad); err == nil {
			t.Errorf("ValidateAppName(%q) accepted", bad)
		}
	}
}

func TestSortApps(t *testing.T) {
	apps := []guardian.Application{
		{Name: "zoom.exe", Enabled: true},
		{Name: "Roblox.exe", Enabled: false},
		{Name: "chrome.exe", Enabled: true},
		{Name: "anydesk.exe", Enabled: false},
	}
	SortApps(apps)
	want := []string{"anydesk.exe", "Roblox.exe", "chrome.exe", "zoom.exe"}
	for i, w := range want {
		if apps[i].Name != w {
			t.Fatalf("order = %v, want %v", apps, want)
		}
	}
}

func TestIsOnline(t *testing.T) {
	if !IsOnline(pc(1, 59*time.Second), t0) {
		t.Error("59 s ago should be online")
	}
	if IsOnline(pc(1, 61*time.Second), t0) {
		t.Error("61 s ago should be offline")
	}
	if IsOnline(guardian.Computer{DateTime: "garbage"}, t0) {
		t.Error("unparseable time should be offline")
	}
}

func TestOnlineSummary(t *testing.T) {
	cs := []guardian.Computer{pc(1, time.Second), pc(2, 10*time.Second), pc(3, time.Hour)}
	if got := OnlineSummary(cs, t0); got != "2/3 online" {
		t.Errorf("OnlineSummary = %q", got)
	}
}

func TestLastSync(t *testing.T) {
	if _, ok := LastSync(nil); ok {
		t.Error("LastSync(nil) ok")
	}
	got, ok := LastSync([]guardian.Computer{pc(1, time.Hour), pc(2, 5*time.Second)})
	if !ok || !got.Equal(t0.Add(-5*time.Second)) {
		t.Errorf("LastSync = %v %v", got, ok)
	}
}

func TestNoAgentWarning(t *testing.T) {
	start := t0.Add(-2 * time.Minute)
	if NoAgentWarning(t0.Add(-time.Minute), nil, t0) {
		t.Error("1 min after start: no warning yet")
	}
	if !NoAgentWarning(start, nil, t0) {
		t.Error("2 min after start, no computers: warning")
	}
	if NoAgentWarning(start, []guardian.Computer{pc(1, 30*time.Second)}, t0) {
		t.Error("a computer synced after start: no warning")
	}
	if !NoAgentWarning(start, []guardian.Computer{pc(1, time.Hour)}, t0) {
		t.Error("only a sync from before start: warning")
	}
}

func TestNewConnectInfo(t *testing.T) {
	ci := NewConnectInfo("PROFESOR-PC", "192.168.1.37", Settings{Port: 8080, Token: "tok", AdminToken: "adm"})
	want := ConnectInfo{
		AgentLine:   "SERVER_ADDRESS=http://PROFESOR-PC:8080",
		TokenLine:   "TOKEN=tok",
		PhoneByName: "http://PROFESOR-PC:8080",
		PhoneByIP:   "http://192.168.1.37:8080",
		AdminToken:  "adm",
	}
	if ci != want {
		t.Errorf("NewConnectInfo = %+v, want %+v", ci, want)
	}
}

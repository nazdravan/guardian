package main

import (
	"fmt"
	"net/http"
	"os"
	"testing"

	"server-gui/core"
)

// TestBuildWindow is a smoke test for the declarative layout: a typo in a
// widget property only shows up when the window is actually created. It needs
// an interactive desktop session, so it is opt-in.
//
//	set PS_GUI_TEST=1 && go test -run TestBuildWindow
func TestBuildWindow(t *testing.T) {
	if os.Getenv("PS_GUI_TEST") == "" {
		t.Skip("set PS_GUI_TEST=1 to run the window smoke test")
	}

	s := core.Settings{Port: 0, Token: "client", AdminToken: "admin"}
	r, err := core.Start(t.TempDir(), s)
	if err != nil {
		t.Fatal(err)
	}
	a := &app{settings: s, runner: r, client: core.NewClient(r.Port(), "admin"), hostname: "TEST-PC", ip: "127.0.0.1"}
	defer a.stop()

	// Opening the window must not change the class's state.
	if err := a.client.SetStatus(true, "whitelist"); err != nil {
		t.Fatal(err)
	}
	if err := a.build(); err != nil {
		t.Fatalf("build: %v", err)
	}
	defer a.mw.Dispose()
	if st, err := a.client.Status(); err != nil || !st.Enabled || st.Mode != "whitelist" {
		t.Fatalf("status after build = %+v, %v; want enabled whitelist", st, err)
	}

	if err := a.client.AddApp("chrome.exe", "blacklist"); err != nil {
		t.Fatal(err)
	}
	req, _ := http.NewRequest("GET", fmt.Sprintf("http://127.0.0.1:%d/client/sync?identity=7", r.Port()), nil)
	req.Header.Set("Authorization", "Bearer client")
	if resp, err := http.DefaultClient.Do(req); err == nil {
		resp.Body.Close()
	}

	a.apply(a.fetch())
	if n := a.apps.RowCount(); n != 1 {
		t.Errorf("application rows = %d, want 1", n)
	}
	if n := a.pcs.RowCount(); n != 1 {
		t.Errorf("computer rows = %d, want 1", n)
	}
	if got := a.pcs.Value(0, 0); got != "PC 7" {
		t.Errorf("computer label = %v, want PC 7", got)
	}

	// The selection survives a refresh.
	a.pcsTable.SetCurrentIndex(0)
	a.apply(a.fetch())
	if a.pcsTable.CurrentIndex() != 0 {
		t.Errorf("selection after refresh = %d, want 0", a.pcsTable.CurrentIndex())
	}
}

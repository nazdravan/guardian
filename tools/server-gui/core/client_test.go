package core

import (
	"fmt"
	"net/http"
	"testing"

	"server/guardian"
)

func newTestClient(t *testing.T) (*Client, *Runner) {
	r := startTest(t)
	return NewClient(r.Port(), "admin"), r
}

func findApp(apps []guardian.Application, name string) (guardian.Application, bool) {
	for _, a := range apps {
		if a.Name == name {
			return a, true
		}
	}
	return guardian.Application{}, false
}

func TestClientAppsRoundTrip(t *testing.T) {
	c, _ := newTestClient(t)
	if err := c.AddApp("chrome.exe", "blacklist"); err != nil {
		t.Fatal(err)
	}
	apps, err := c.Apps()
	if err != nil {
		t.Fatal(err)
	}
	if a, ok := findApp(apps, "chrome.exe"); !ok || !a.Enabled || a.Mode != "blacklist" {
		t.Fatalf("after add: %+v ok=%v", a, ok)
	}
	if err := c.SetAppEnabled("chrome.exe", "blacklist", false); err != nil {
		t.Fatal(err)
	}
	apps, _ = c.Apps()
	if a, _ := findApp(apps, "chrome.exe"); a.Enabled {
		t.Error("chrome.exe still enabled")
	}
	if err := c.RemoveApp("chrome.exe", "blacklist"); err != nil {
		t.Fatal(err)
	}
	apps, _ = c.Apps()
	if _, ok := findApp(apps, "chrome.exe"); ok {
		t.Error("chrome.exe not removed")
	}
}

func TestClientStatus(t *testing.T) {
	c, _ := newTestClient(t)
	if err := c.SetStatus(true, "whitelist"); err != nil {
		t.Fatal(err)
	}
	st, err := c.Status()
	if err != nil {
		t.Fatal(err)
	}
	if !st.Enabled || st.Mode != "whitelist" {
		t.Errorf("Status = %+v", st)
	}
}

func blockedOf(t *testing.T, c *Client, identity int) bool {
	t.Helper()
	cs, now, err := c.Computers()
	if err != nil {
		t.Fatal(err)
	}
	if now.IsZero() {
		t.Error("Computers returned no server time")
	}
	for _, pc := range cs {
		if pc.Identity == identity {
			return pc.Blocked
		}
	}
	t.Fatalf("computer %d not listed", identity)
	return false
}

func TestClientComputers(t *testing.T) {
	c, r := newTestClient(t)
	req, _ := http.NewRequest("GET", fmt.Sprintf("http://127.0.0.1:%d/client/sync?identity=7", r.Port()), nil)
	req.Header.Set("Authorization", "Bearer client")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()

	if err := c.SetBlocked(7, false); err != nil {
		t.Fatal(err)
	}
	if blockedOf(t, c, 7) {
		t.Error("PC 7 should be unblocked")
	}
	if err := c.BlockAll(); err != nil {
		t.Fatal(err)
	}
	if !blockedOf(t, c, 7) {
		t.Error("PC 7 should be blocked after BlockAll")
	}
	if err := c.UnblockAll(); err != nil {
		t.Fatal(err)
	}
	if blockedOf(t, c, 7) {
		t.Error("PC 7 should be unblocked after UnblockAll")
	}
}

func TestClientWrongTokenErrors(t *testing.T) {
	_, r := newTestClient(t)
	if _, err := NewClient(r.Port(), "wrong").Apps(); err == nil {
		t.Error("Apps with a wrong admin token succeeded")
	}
}

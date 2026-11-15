package guardian

import (
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
)

func newTestServer(t *testing.T) (*Server, *httptest.Server) {
	t.Helper()
	s, err := New(Config{
		DBPath:      filepath.Join(t.TempDir(), "guardian.db"),
		ClientToken: "client",
		AdminToken:  "admin",
	})
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	ts := httptest.NewServer(s.Handler())
	t.Cleanup(func() {
		ts.Close()
		s.Close()
	})
	return s, ts
}

func do(t *testing.T, method, url, token, body string) (int, string) {
	t.Helper()
	req, err := http.NewRequest(method, url, strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	if body != "" {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, string(b)
}

func TestHealthNeedsNoToken(t *testing.T) {
	_, ts := newTestServer(t)
	if code, _ := do(t, "GET", ts.URL+"/health", "", ""); code != http.StatusOK {
		t.Fatalf("GET /health = %d, want 200", code)
	}
}

func TestAdminRoutesUseAdminToken(t *testing.T) {
	_, ts := newTestServer(t)
	if code, _ := do(t, "GET", ts.URL+"/status", "admin", ""); code != http.StatusOK {
		t.Errorf("GET /status with admin token = %d, want 200", code)
	}
	if code, _ := do(t, "GET", ts.URL+"/status", "client", ""); code != http.StatusUnauthorized {
		t.Errorf("GET /status with client token = %d, want 401", code)
	}
}

func TestSyncUsesClientToken(t *testing.T) {
	_, ts := newTestServer(t)
	code, body := do(t, "GET", ts.URL+"/client/sync", "client", "")
	if code != http.StatusOK {
		t.Fatalf("GET /client/sync = %d, want 200", code)
	}
	if !strings.Contains(body, `"mode":"free"`) {
		t.Errorf("fresh server should sync mode free, got %s", body)
	}
}

func TestUpdateComputerKeepsLastSync(t *testing.T) {
	s, ts := newTestServer(t)
	if _, err := s.db.Exec("INSERT INTO computers (identity, blocked, datetime) VALUES (7, 0, datetime('now', '-10 minutes'))"); err != nil {
		t.Fatal(err)
	}
	var before string
	s.db.QueryRow("SELECT datetime FROM computers WHERE identity = 7").Scan(&before)

	if code, body := do(t, "PUT", ts.URL+"/manage/computers", "admin", `{"identity":7,"blocked":true}`); code != http.StatusOK {
		t.Fatalf("PUT /manage/computers = %d: %s", code, body)
	}
	var after string
	var blocked bool
	s.db.QueryRow("SELECT datetime, blocked FROM computers WHERE identity = 7").Scan(&after, &blocked)
	if !blocked {
		t.Error("computer 7 should be blocked")
	}
	if after != before {
		t.Errorf("last sync changed from %s to %s by blocking", before, after)
	}

	// An unknown computer is still created with the requested state.
	do(t, "PUT", ts.URL+"/manage/computers", "admin", `{"identity":9,"blocked":true}`)
	if err := s.db.QueryRow("SELECT blocked FROM computers WHERE identity = 9").Scan(&blocked); err != nil || !blocked {
		t.Errorf("computer 9: blocked=%v err=%v, want created and blocked", blocked, err)
	}
}

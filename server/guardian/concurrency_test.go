package guardian

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// TestWritesSurviveConcurrentReads reproduces "Failed to update database":
// the Windows window reads /manage/computers every 3 s and every agent writes
// its last sync on /client/sync, so a teacher's click often lands while
// another connection holds the database. Without a busy timeout SQLite
// answers that write with SQLITE_BUSY at once.
func TestWritesSurviveConcurrentReads(t *testing.T) {
	_, ts := newTestServer(t)
	if code, body := do(t, "POST", ts.URL+"/manage/applications", "admin", `{"name":"chrome.exe","mode":"blacklist"}`); code != http.StatusOK && code != http.StatusCreated {
		t.Fatalf("add app = %d %s", code, body)
	}

	call := func(method, path, token, body string) int {
		req, _ := http.NewRequest(method, ts.URL+path, strings.NewReader(body))
		req.Header.Set("Authorization", "Bearer "+token)
		if body != "" {
			req.Header.Set("Content-Type", "application/json")
		}
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return -1
		}
		resp.Body.Close()
		return resp.StatusCode
	}

	stop := make(chan struct{})
	var wg sync.WaitGroup
	var failures atomic.Int64
	loop := func(f func() int) {
		defer wg.Done()
		for {
			select {
			case <-stop:
				return
			default:
			}
			if f() != http.StatusOK {
				failures.Add(1)
			}
		}
	}
	for i := 1; i <= 8; i++ { // agents writing their sync time
		id := i
		wg.Add(1)
		go loop(func() int { return call("GET", fmt.Sprintf("/client/sync?identity=%d", id), "client", "") })
	}
	for i := 0; i < 2; i++ { // the window and the phone reading
		wg.Add(1)
		go loop(func() int { return call("GET", "/manage/computers", "admin", "") })
	}

	// The teacher ticking and unticking the application.
	deadline := time.Now().Add(2 * time.Second)
	n := 0
	for time.Now().Before(deadline) {
		body := fmt.Sprintf(`{"name":"chrome.exe","mode":"blacklist","enabled":%v}`, n%2 == 0)
		if code := call("PUT", "/manage/applications", "admin", body); code != http.StatusOK {
			failures.Add(1)
			t.Errorf("PUT #%d = %d", n, code)
			if n > 5 {
				break
			}
		}
		n++
	}
	close(stop)
	wg.Wait()
	if f := failures.Load(); f > 0 {
		t.Fatalf("%d requests failed under concurrent load (%d PUTs sent)", f, n)
	}
}

// TestDatabaseOpensWithPragmas checks the pragmas reach SQLite, with a path
// that has a space in it like %ProgramData%\Guardian Server\guardian.db.
func TestDatabaseOpensWithPragmas(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "Guardian Server")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	s, err := New(Config{DBPath: filepath.Join(dir, "guardian.db"), ClientToken: "c", AdminToken: "a"})
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	defer s.Close()
	var mode string
	var timeout int
	if err := s.db.QueryRow("PRAGMA journal_mode").Scan(&mode); err != nil || mode != "wal" {
		t.Errorf("journal_mode = %q, %v; want wal", mode, err)
	}
	if err := s.db.QueryRow("PRAGMA busy_timeout").Scan(&timeout); err != nil || timeout != 5000 {
		t.Errorf("busy_timeout = %d, %v; want 5000", timeout, err)
	}
}

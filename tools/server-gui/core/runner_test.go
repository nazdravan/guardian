package core

import (
	"fmt"
	"net/http"
	"testing"
	"time"
)

func testSettings() Settings {
	return Settings{Port: 0, Token: "client", AdminToken: "admin"}
}

func startTest(t *testing.T) *Runner {
	t.Helper()
	r, err := Start(t.TempDir(), testSettings())
	if err != nil {
		t.Fatalf("Start: %v", err)
	}
	t.Cleanup(func() { r.Stop() })
	return r
}

func TestStartServesHealth(t *testing.T) {
	r := startTest(t)
	resp, err := http.Get(fmt.Sprintf("http://127.0.0.1:%d/health", r.Port()))
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Errorf("GET /health = %d", resp.StatusCode)
	}
	if time.Since(r.StartedAt()) > time.Minute {
		t.Errorf("StartedAt = %v", r.StartedAt())
	}
}

func TestStartReportsPortInUse(t *testing.T) {
	first := startTest(t)
	s := testSettings()
	s.Port = first.Port()
	second, err := Start(t.TempDir(), s)
	if err == nil {
		second.Stop()
		t.Fatal("second Start on the same port succeeded")
	}
	if !IsAddrInUse(err) {
		t.Errorf("IsAddrInUse(%v) = false", err)
	}
	if got, want := StartError(err, s.Port), fmt.Sprintf("port %d is in use", s.Port); got != want {
		t.Errorf("StartError = %q, want %q", got, want)
	}
}

func TestStopIsIdempotent(t *testing.T) {
	r, err := Start(t.TempDir(), testSettings())
	if err != nil {
		t.Fatal(err)
	}
	if err := r.Stop(); err != nil {
		t.Errorf("first Stop: %v", err)
	}
	if err := r.Stop(); err != nil {
		t.Errorf("second Stop: %v", err)
	}
	if _, err := http.Get(fmt.Sprintf("http://127.0.0.1:%d/health", r.Port())); err == nil {
		t.Error("server still answers after Stop")
	}
}

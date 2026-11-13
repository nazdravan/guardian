package core

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestLoadOrCreateGeneratesOnFirstRun(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "Guardian Server")
	s, err := LoadOrCreate(dir)
	if err != nil {
		t.Fatal(err)
	}
	if s.Port != 8080 {
		t.Errorf("Port = %d, want 8080", s.Port)
	}
	if len(s.Token) != 43 || len(s.AdminToken) != 43 {
		t.Errorf("token lengths = %d / %d, want 43", len(s.Token), len(s.AdminToken))
	}
	if s.Token == s.AdminToken {
		t.Error("TOKEN and ADMIN_TOKEN must differ")
	}
	again, err := LoadOrCreate(dir)
	if err != nil {
		t.Fatal(err)
	}
	if again != s {
		t.Errorf("second load = %+v, want %+v", again, s)
	}
	if got := s.DBPath(dir); got != filepath.Join(dir, "guardian.db") {
		t.Errorf("DBPath = %q", got)
	}
}

func TestLoadOrCreateFillsOnlyMissing(t *testing.T) {
	dir := t.TempDir()
	env := filepath.Join(dir, ".env")
	os.WriteFile(env, []byte("# teacher's note\nTOKEN=keep-me\nADMIN_TOKEN=\nEXTRA=1\n"), 0o600)

	s, err := LoadOrCreate(dir)
	if err != nil {
		t.Fatal(err)
	}
	if s.Token != "keep-me" {
		t.Errorf("Token = %q, want keep-me", s.Token)
	}
	if len(s.AdminToken) != 43 {
		t.Errorf("AdminToken = %q, want a generated token", s.AdminToken)
	}
	b, _ := os.ReadFile(env)
	for _, want := range []string{"TOKEN=keep-me", "ADMIN_TOKEN=" + s.AdminToken, "EXTRA=1", "# teacher's note", "PORT=8080"} {
		if !strings.Contains(string(b), want) {
			t.Errorf(".env lacks %q:\n%s", want, b)
		}
	}
}

func TestLoadOrCreateRejectsBadPort(t *testing.T) {
	for _, port := range []string{"abc", "70000", "0"} {
		dir := t.TempDir()
		os.WriteFile(filepath.Join(dir, ".env"), []byte("PORT="+port+"\n"), 0o600)
		if _, err := LoadOrCreate(dir); err == nil || !strings.Contains(err.Error(), "PORT") {
			t.Errorf("PORT=%s: err = %v, want an error mentioning PORT", port, err)
		}
	}
}

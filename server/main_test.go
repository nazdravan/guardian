package main

import "testing"

func TestConfigFromEnvKeepsFallbackTokens(t *testing.T) {
	t.Setenv("TOKEN", "")
	t.Setenv("ADMIN_TOKEN", "")
	cfg := configFromEnv()
	if cfg.ClientToken != fallbackToken || cfg.AdminToken != fallbackToken {
		t.Errorf("fallback tokens = %q / %q", cfg.ClientToken, cfg.AdminToken)
	}
	if cfg.DBPath != "./guardian.db" {
		t.Errorf("DBPath = %q", cfg.DBPath)
	}

	t.Setenv("TOKEN", "x")
	if got := configFromEnv().ClientToken; got != "x" {
		t.Errorf("ClientToken = %q, want x", got)
	}
}

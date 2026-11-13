// Package core is the OS-independent part of Guardian Server for Windows:
// settings, running the embedded server, talking to it, and the logic behind
// what the window shows. It has no GUI dependency, so it is tested anywhere.
package core

import (
	"crypto/rand"
	"encoding/base64"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// DefaultPort matches the agents' and the phone app's default.
const DefaultPort = 8080

// Settings is what Guardian Server keeps in <data dir>\.env.
type Settings struct {
	Port       int
	Token      string // agents' TOKEN
	AdminToken string // phone and window's ADMIN_TOKEN
}

// DBPath is the server database inside the data dir.
func (s Settings) DBPath(dir string) string {
	return filepath.Join(dir, "guardian.db")
}

// NewToken returns 32 random bytes as URL-safe base64 (43 characters).
func NewToken() string {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		panic("crypto/rand failed: " + err.Error())
	}
	return base64.RawURLEncoding.EncodeToString(b)
}

// LoadOrCreate reads dir/.env, creating dir and the file on first run. A
// missing or blank PORT, TOKEN or ADMIN_TOKEN is filled in and written back;
// a value that is already there is never replaced, because agents and the
// phone already use it. Other lines and comments are kept as they are.
func LoadOrCreate(dir string) (Settings, error) {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return Settings{}, err
	}
	path := filepath.Join(dir, ".env")

	var lines []string
	if b, err := os.ReadFile(path); err == nil {
		// Notepad may save "UTF-8 with BOM"; the BOM would hide the first key.
		text := strings.TrimPrefix(string(b), "\ufeff")
		lines = strings.Split(strings.ReplaceAll(text, "\r\n", "\n"), "\n")
		if len(lines) > 0 && lines[len(lines)-1] == "" {
			lines = lines[:len(lines)-1]
		}
	} else if !os.IsNotExist(err) {
		return Settings{}, err
	}

	values := map[string]string{}
	index := map[string]int{}
	for i, line := range lines {
		t := strings.TrimSpace(line)
		if t == "" || strings.HasPrefix(t, "#") {
			continue
		}
		if k, v, ok := strings.Cut(t, "="); ok {
			k = strings.TrimSpace(k)
			values[k] = unquote(strings.TrimSpace(v))
			index[k] = i
		}
	}

	changed := false
	fill := func(key, value string) {
		if values[key] != "" {
			return
		}
		values[key] = value
		changed = true
		if i, ok := index[key]; ok {
			lines[i] = key + "=" + value
		} else {
			lines = append(lines, key+"="+value)
		}
	}
	fill("PORT", strconv.Itoa(DefaultPort))
	fill("TOKEN", NewToken())
	fill("ADMIN_TOKEN", NewToken())

	port, err := strconv.Atoi(values["PORT"])
	if err != nil || port < 1 || port > 65535 {
		return Settings{}, fmt.Errorf("PORT in %s must be a number from 1 to 65535, not %q", path, values["PORT"])
	}

	if changed {
		if err := os.WriteFile(path, []byte(strings.Join(lines, "\r\n")+"\r\n"), 0o600); err != nil {
			return Settings{}, err
		}
	}
	return Settings{Port: port, Token: values["TOKEN"], AdminToken: values["ADMIN_TOKEN"]}, nil
}

// unquote strips one pair of matching quotes, like the Linux server's .env loader.
func unquote(v string) string {
	if len(v) >= 2 && (v[0] == '"' || v[0] == '\'') && v[len(v)-1] == v[0] {
		return v[1 : len(v)-1]
	}
	return v
}

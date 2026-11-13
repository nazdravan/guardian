package core

import (
	"fmt"
	"sort"
	"strings"
	"time"

	"server/guardian"
)

// OnlineWindow matches the Android app: a computer that synced less than a
// minute ago is online.
const OnlineWindow = 60 * time.Second

// NoAgentAfter is how long the server runs before warning that no agent has
// reached it.
const NoAgentAfter = 2 * time.Minute

// ValidateAppName trims a typed process name and refuses very short ones:
// the agent matches blacklist names as substrings of the process list, so a
// name like "a" would close almost every program.
func ValidateAppName(name string) (string, error) {
	name = strings.TrimSpace(name)
	if len([]rune(name)) < 3 {
		return "", fmt.Errorf("enter at least 3 characters, e.g. chrome.exe")
	}
	return name, nil
}

// SortApps orders applications like the phone does: disabled first, then by
// name, ignoring case.
func SortApps(apps []guardian.Application) {
	sort.SliceStable(apps, func(i, j int) bool {
		if apps[i].Enabled != apps[j].Enabled {
			return !apps[i].Enabled
		}
		return strings.ToLower(apps[i].Name) < strings.ToLower(apps[j].Name)
	})
}

func syncTime(c guardian.Computer) (time.Time, bool) {
	t, err := time.Parse(time.RFC3339, c.DateTime)
	return t, err == nil
}

// IsOnline reports whether c synced within OnlineWindow of now.
func IsOnline(c guardian.Computer, now time.Time) bool {
	t, ok := syncTime(c)
	if !ok {
		return false
	}
	d := now.Sub(t)
	if d < 0 {
		d = -d
	}
	return d < OnlineWindow
}

// OnlineSummary is the Computers header, e.g. "9/12 online".
func OnlineSummary(cs []guardian.Computer, now time.Time) string {
	n := 0
	for _, c := range cs {
		if IsOnline(c, now) {
			n++
		}
	}
	return fmt.Sprintf("%d/%d online", n, len(cs))
}

// LastSync is the most recent sync of any computer.
func LastSync(cs []guardian.Computer) (time.Time, bool) {
	var last time.Time
	found := false
	for _, c := range cs {
		if t, ok := syncTime(c); ok && (!found || t.After(last)) {
			last, found = t, true
		}
	}
	return last, found
}

// NoAgentWarning is true once the server has run for NoAgentAfter without a
// single computer syncing since it started: agents probably cannot find it.
func NoAgentWarning(startedAt time.Time, cs []guardian.Computer, now time.Time) bool {
	if now.Sub(startedAt) < NoAgentAfter {
		return false
	}
	last, ok := LastSync(cs)
	return !ok || !last.After(startedAt)
}

// ConnectInfo is what the Connect window offers for copying.
type ConnectInfo struct {
	AgentLine   string // SERVER_ADDRESS line for agent.env
	TokenLine   string // TOKEN line for agent.env
	PhoneByName string
	PhoneByIP   string // fallback: Android usually cannot resolve Windows names
	AdminToken  string
}

// NewConnectInfo builds the Connect window's values.
func NewConnectInfo(hostname, ip string, s Settings) ConnectInfo {
	byName := fmt.Sprintf("http://%s:%d", hostname, s.Port)
	return ConnectInfo{
		AgentLine:   "SERVER_ADDRESS=" + byName,
		TokenLine:   "TOKEN=" + s.Token,
		PhoneByName: byName,
		PhoneByIP:   fmt.Sprintf("http://%s:%d", ip, s.Port),
		AdminToken:  s.AdminToken,
	}
}

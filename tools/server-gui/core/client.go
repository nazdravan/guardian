package core

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"

	"server/guardian"
)

// Client is how the window talks to the embedded server: over HTTP on
// 127.0.0.1 with ADMIN_TOKEN, through the same handlers the phone uses.
type Client struct {
	base  string
	token string
	http  *http.Client
}

// NewClient returns a client for the server on 127.0.0.1:port.
func NewClient(port int, adminToken string) *Client {
	return &Client{
		base:  fmt.Sprintf("http://127.0.0.1:%d", port),
		token: adminToken,
		http:  &http.Client{Timeout: 5 * time.Second},
	}
}

// call sends body (if not nil) as JSON and decodes the reply into out (if
// not nil). A non-2xx reply becomes an error with the server's message.
func (c *Client) call(method, path string, body, out any) error {
	var rd io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return err
		}
		rd = bytes.NewReader(b)
	}
	req, err := http.NewRequest(method, c.base+path, rd)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+c.token)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		var e guardian.ErrorResponse
		if json.Unmarshal(data, &e) == nil && e.Error != "" {
			return fmt.Errorf("%s %s: %s", method, path, e.Error)
		}
		return fmt.Errorf("%s %s: %s", method, path, resp.Status)
	}
	if out != nil {
		return json.Unmarshal(data, out)
	}
	return nil
}

// Status is protection on/off and the class mode.
func (c *Client) Status() (guardian.StatusResponse, error) {
	var st guardian.StatusResponse
	err := c.call("GET", "/status", nil, &st)
	return st, err
}

// SetStatus switches protection and the class mode.
func (c *Client) SetStatus(enabled bool, mode string) error {
	return c.call("PUT", "/status", guardian.StatusResponse{Enabled: enabled, Mode: mode}, nil)
}

// Apps lists every application in both modes.
func (c *Client) Apps() ([]guardian.Application, error) {
	var r guardian.ApplicationsResponse
	err := c.call("GET", "/manage/applications", nil, &r)
	return r.Applications, err
}

// AddApp adds a process name to mode's list.
func (c *Client) AddApp(name, mode string) error {
	return c.call("POST", "/manage/applications", map[string]string{"name": name, "mode": mode}, nil)
}

// SetAppEnabled ticks or unticks an application.
func (c *Client) SetAppEnabled(name, mode string, enabled bool) error {
	return c.call("PUT", "/manage/applications", map[string]any{"name": name, "mode": mode, "enabled": enabled}, nil)
}

// RemoveApp deletes an application from mode's list.
func (c *Client) RemoveApp(name, mode string) error {
	return c.call("DELETE", "/manage/applications", map[string]string{"name": name, "mode": mode}, nil)
}

// Computers lists every computer that has synced, and the server's clock.
func (c *Client) Computers() ([]guardian.Computer, time.Time, error) {
	var r guardian.ComputersResponse
	if err := c.call("GET", "/manage/computers", nil, &r); err != nil {
		return nil, time.Time{}, err
	}
	now, err := time.Parse(time.RFC3339, r.CurrentTime)
	if err != nil {
		return nil, time.Time{}, fmt.Errorf("bad current_time %q: %v", r.CurrentTime, err)
	}
	return r.Computers, now, nil
}

// SetBlocked blocks or unblocks one computer.
func (c *Client) SetBlocked(identity int, blocked bool) error {
	return c.call("PUT", "/manage/computers", map[string]any{"identity": identity, "blocked": blocked}, nil)
}

// BlockAll blocks every known computer.
func (c *Client) BlockAll() error {
	return c.call("PUT", "/manage/computers/block_all", nil, nil)
}

// UnblockAll unblocks every known computer.
func (c *Client) UnblockAll() error {
	return c.call("DELETE", "/manage/computers/reset", nil, nil)
}

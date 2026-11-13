package core

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"sync"
	"time"

	"server/guardian"
)

// Runner is the embedded server while the window is open.
type Runner struct {
	srv       *guardian.Server
	http      *http.Server
	port      int
	startedAt time.Time
	stopOnce  sync.Once
}

// Start opens the database in dir and listens on s.Port on every interface.
// The listen happens before Start returns, so "port in use" is reported here
// and the window never shows a server as running that is not. Port 0 picks a
// free port (tests only).
func Start(dir string, s Settings) (*Runner, error) {
	srv, err := guardian.New(guardian.Config{
		DBPath:      s.DBPath(dir),
		ClientToken: s.Token,
		AdminToken:  s.AdminToken,
	})
	if err != nil {
		return nil, err
	}
	ln, err := net.Listen("tcp", fmt.Sprintf(":%d", s.Port))
	if err != nil {
		srv.Close()
		return nil, err
	}
	r := &Runner{
		srv:       srv,
		port:      ln.Addr().(*net.TCPAddr).Port,
		startedAt: time.Now(),
		// Same timeouts as server/main.go.
		http: &http.Server{
			Handler:      srv.Handler(),
			ReadTimeout:  15 * time.Second,
			WriteTimeout: 15 * time.Second,
			IdleTimeout:  60 * time.Second,
		},
	}
	go r.http.Serve(ln)
	return r, nil
}

// Port is the port actually listened on.
func (r *Runner) Port() int { return r.port }

// StartedAt is when the server started listening.
func (r *Runner) StartedAt() time.Time { return r.startedAt }

// Stop shuts the server down gracefully (up to 5 s) and closes the database.
// Calling it again does nothing and returns nil.
func (r *Runner) Stop() error {
	var err error
	r.stopOnce.Do(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		err = r.http.Shutdown(ctx)
		if cerr := r.srv.Close(); err == nil {
			err = cerr
		}
	})
	return err
}

// StartError turns a Start failure into the text the window shows.
func StartError(err error, port int) string {
	if IsAddrInUse(err) {
		return fmt.Sprintf("port %d is in use", port)
	}
	return err.Error()
}

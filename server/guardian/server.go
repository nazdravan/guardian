// Package guardian is the Guardian server: the HTTP API, its SQLite database
// and the in-memory caches. server/main.go runs it on Linux; the Windows
// Guardian Server app (tools/server-gui) embeds it.
package guardian

import (
	"database/sql"
	"fmt"
	"net/http"
	"sync"

	_ "modernc.org/sqlite"
)

// Config is everything the server needs from its host program.
type Config struct {
	DBPath      string // SQLite file, created if missing
	ClientToken string // Bearer token for agent endpoints
	AdminToken  string // Bearer token for management endpoints
}

// Server holds the state of the guardian server.
type Server struct {
	mu           sync.RWMutex
	db           *sql.DB
	appsCache    map[string]Application
	enabledCache bool
	modeCache    string
	clientCache  map[string]bool

	clientToken string
	adminToken  string
}

// New opens (and migrates) the database at cfg.DBPath and loads the caches.
func New(cfg Config) (*Server, error) {
	db, err := openDatabase(cfg.DBPath)
	if err != nil {
		return nil, err
	}

	s := &Server{
		db:          db,
		appsCache:   make(map[string]Application),
		modeCache:   "blacklist",
		clientCache: make(map[string]bool),
		clientToken: cfg.ClientToken,
		adminToken:  cfg.AdminToken,
	}

	if err := s.initDatabase(); err != nil {
		db.Close()
		return nil, fmt.Errorf("failed to initialize database: %v", err)
	}
	if err := s.loadFromDatabase(); err != nil {
		db.Close()
		return nil, fmt.Errorf("failed to load data: %v", err)
	}
	return s, nil
}

// Handler returns the server's HTTP routes.
func (s *Server) Handler() http.Handler {
	return s.setupRoutes()
}

// Close closes the database.
func (s *Server) Close() error {
	if s.db != nil {
		return s.db.Close()
	}
	return nil
}

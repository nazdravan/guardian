package main

import (
	"bufio"
	"context"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"server/guardian"
)

// fallbackToken is used when TOKEN or ADMIN_TOKEN is not set, as before.
const fallbackToken = "mILp9n6shk3G9SGSaS2nmP6YlLHwsP1Z"

func getenvDefault(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

// configFromEnv builds the server config the Linux binary has always used:
// ./guardian.db in the working directory and tokens from the environment.
func configFromEnv() guardian.Config {
	return guardian.Config{
		DBPath:      "./guardian.db",
		ClientToken: getenvDefault("TOKEN", fallbackToken),
		AdminToken:  getenvDefault("ADMIN_TOKEN", fallbackToken),
	}
}

func main() {
	slog.SetDefault(slog.New(slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{Level: slog.LevelInfo})))

	if err := loadEnvFile(".env"); err != nil && !os.IsNotExist(err) {
		slog.Warn("could not load .env file", "error", err)
	}

	server, err := guardian.New(configFromEnv())
	if err != nil {
		slog.Error("failed to create server", "error", err)
		os.Exit(1)
	}
	defer server.Close()

	addr := "0.0.0.0:8080"
	displayAddr := "http://localhost:8080/"
	if envAddr := os.Getenv("SERVER_ADDRESS"); envAddr != "" {
		displayAddr = envAddr
		parts := strings.Split(envAddr, "//")
		addr = parts[len(parts)-1]
	}

	httpServer := &http.Server{
		Addr:         addr,
		Handler:      server.Handler(),
		ReadTimeout:  15 * time.Second,
		WriteTimeout: 15 * time.Second,
		IdleTimeout:  60 * time.Second,
	}

	// Graceful shutdown
	done := make(chan os.Signal, 1)
	signal.Notify(done, os.Interrupt, syscall.SIGTERM)

	go func() {
		fmt.Printf("Guardian Server starting on %s\n", addr)
		fmt.Printf("API: %s\n", displayAddr)
		if err := httpServer.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			slog.Error("server failed", "error", err)
			os.Exit(1)
		}
	}()

	<-done
	slog.Info("shutting down server")

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	if err := httpServer.Shutdown(ctx); err != nil {
		slog.Error("shutdown error", "error", err)
	}
	slog.Info("server stopped")
}

// loadEnvFile loads KEY=VALUE lines into the environment without overriding
// variables that are already set.
func loadEnvFile(filename string) error {
	file, err := os.Open(filename)
	if err != nil {
		return err
	}
	defer file.Close()

	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) == 2 {
			key := strings.TrimSpace(parts[0])
			value := strings.TrimSpace(parts[1])
			if len(value) >= 2 {
				if (strings.HasPrefix(value, "\"") && strings.HasSuffix(value, "\"")) ||
					(strings.HasPrefix(value, "'") && strings.HasSuffix(value, "'")) {
					value = value[1 : len(value)-1]
				}
			}
			if os.Getenv(key) == "" {
				os.Setenv(key, value)
			}
		}
	}
	return scanner.Err()
}

package guardian

import (
	"crypto/subtle"
	"log/slog"
	"net/http"
)

// authMiddleware accepts only requests carrying "Authorization: Bearer <token>".
func authMiddleware(token string) func(http.Handler) http.Handler {
	expected := []byte("Bearer " + token)
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			actual := []byte(r.Header.Get("Authorization"))
			if subtle.ConstantTimeCompare(actual, expected) != 1 {
				slog.Warn("unauthorized access attempt", "remote", r.RemoteAddr, "path", r.URL.Path)
				http.Error(w, "Unauthorized", http.StatusUnauthorized)
				return
			}
			next.ServeHTTP(w, r)
		})
	}
}

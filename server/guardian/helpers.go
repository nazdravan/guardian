package guardian

import (
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"time"
)

func appCacheKey(name, mode string) string {
	return name + ":" + mode
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(v)
}

func parseIntParam(param string) (int, error) {
	var value int
	_, err := fmt.Sscanf(param, "%d", &value)
	return value, err
}

func getCurrentTime() string {
	return time.Now().Format("2006-01-02T15:04:05Z07:00")
}

// LocalIP returns this machine's outbound LAN IPv4 address, or 127.0.0.1.
func LocalIP() string {
	conn, err := net.Dial("udp", "8.8.8.8:80")
	if err != nil {
		addrs, err := net.InterfaceAddrs()
		if err != nil {
			return "127.0.0.1"
		}
		for _, addr := range addrs {
			if ipNet, ok := addr.(*net.IPNet); ok && !ipNet.IP.IsLoopback() {
				if ipNet.IP.To4() != nil {
					return ipNet.IP.String()
				}
			}
		}
		return "127.0.0.1"
	}
	defer conn.Close()
	return conn.LocalAddr().(*net.UDPAddr).IP.String()
}

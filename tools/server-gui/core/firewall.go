package core

import "fmt"

// FirewallRuleName is the inbound rule Guardian Server adds for its port.
func FirewallRuleName(port int) string {
	return fmt.Sprintf("Guardian Server (TCP %d)", port)
}

// FirewallArgs are the netsh arguments that add that rule: every network
// profile, because Windows often classifies school networks as Public, but
// only for computers on the local subnet.
func FirewallArgs(port int) []string {
	return []string{"advfirewall", "firewall", "add", "rule",
		"name=" + FirewallRuleName(port), "dir=in", "action=allow", "protocol=TCP",
		fmt.Sprintf("localport=%d", port), "remoteip=LocalSubnet", "profile=any"}
}

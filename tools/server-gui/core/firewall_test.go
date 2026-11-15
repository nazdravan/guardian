package core

import (
	"reflect"
	"testing"
)

func TestFirewallArgs(t *testing.T) {
	want := []string{"advfirewall", "firewall", "add", "rule",
		"name=Guardian Server (TCP 8080)", "dir=in", "action=allow", "protocol=TCP",
		"localport=8080", "remoteip=LocalSubnet", "profile=any"}
	if got := FirewallArgs(8080); !reflect.DeepEqual(got, want) {
		t.Errorf("FirewallArgs(8080) =\n%q\nwant\n%q", got, want)
	}
	if got := FirewallRuleName(8080); got != "Guardian Server (TCP 8080)" {
		t.Errorf("FirewallRuleName = %q", got)
	}
}

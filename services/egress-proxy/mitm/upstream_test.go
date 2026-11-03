package main

import (
	"context"
	"crypto/x509"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// A request for a name the server's certificate covers goes through, and the
// server sees the Host the client asked for.
func TestUpstreamVerifiesTheRequestedName(t *testing.T) {
	var seenHost string
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seenHost = r.Host
		_, _ = io.WriteString(w, "hello")
	}))
	defer server.Close()
	roots := x509.NewCertPool()
	roots.AddCert(server.Certificate())
	dial := func(ctx context.Context, network, addr string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, network, server.Listener.Addr().String())
	}
	front := httptest.NewServer(upstreamHandler(roots, dial))
	defer front.Close()

	// httptest's certificate is valid for example.com.
	req, _ := http.NewRequest("GET", front.URL+"/x", nil)
	req.Host = "example.com"
	res, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(res.Body)
	if res.StatusCode != 200 || string(body) != "hello" || seenHost != "example.com" {
		t.Fatalf("got %d %q host=%q", res.StatusCode, body, seenHost)
	}

	// A name the certificate does not cover is refused, not served.
	req, _ = http.NewRequest("GET", front.URL+"/x", nil)
	req.Host = "other.invalid"
	res, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	if res.StatusCode != http.StatusBadGateway {
		t.Fatalf("a wrong name answered %d, want 502", res.StatusCode)
	}
}

func TestHostChecks(t *testing.T) {
	for _, bad := range []string{"", "a/b", "user@host", "a b"} {
		if checkHost(bad) == nil {
			t.Errorf("%q should be refused", bad)
		}
	}
	if checkHost("api.anthropic.com") != nil || checkHost("example.com:8443") != nil {
		t.Error("plain hosts must pass")
	}
	if got := withPort("example.com"); got != "example.com:443" {
		t.Errorf("withPort = %q", got)
	}
	if got := withPort("example.com:8443"); got != "example.com:8443" {
		t.Errorf("withPort = %q", got)
	}
	_ = strings.TrimSpace
}

package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"log"
	"net"
	"net/http"
	"net/http/httputil"
	"os"
	"strings"
	"time"
)

// The hop that leaves for the internet.
//
// Envoy used to encrypt the decrypted request again with its own upstream TLS
// stack. That failed in two ways that are about the stack and not about the
// servers: two names on one address (platform.claude.com and claude.ai) shared a
// connection set and the second one was checked against the first one's name,
// and a wildcard certificate (registry.npmjs.org under *.npmjs.org) was refused.
// Both showed up as a bare 503 in the sandbox.
//
// Here the name that is checked is the name the client asked for, taken from the
// request's Host header, and Go's verifier builds the chain (it follows cross
// signed roots, which BoringSSL does not). Policy is not decided here: Envoy has
// already asked the control API about this exact request before it reaches this
// port, and the port is on loopback.

// defaultUpstream is where Envoy hands the request over.
const defaultUpstream = "127.0.0.1:10002"

// upstreamHandler re-encrypts a plain HTTP/1.1 request towards the host named in
// its Host header, verified against roots.
func upstreamHandler(roots *x509.CertPool, dial func(ctx context.Context, network, addr string) (net.Conn, error)) http.Handler {
	return upstreamHandlerWith(roots, dial, newDlpClientFromEnv())
}

// upstreamHandlerWith is upstreamHandler with the content check passed in, so a
// test can decide whether the check is part of the hop.
func upstreamHandlerWith(roots *x509.CertPool, dial func(ctx context.Context, network, addr string) (net.Conn, error), dlp *dlpClient) http.Handler {
	transport := &http.Transport{
		TLSClientConfig:       &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12},
		DialContext:           dial,
		ForceAttemptHTTP2:     false,
		TLSNextProto:          map[string]func(string, *tls.Conn) http.RoundTripper{},
		DisableCompression:    true,
		MaxIdleConnsPerHost:   8,
		IdleConnTimeout:       60 * time.Second,
		TLSHandshakeTimeout:   10 * time.Second,
		ResponseHeaderTimeout: 300 * time.Second,
	}
	proxy := &httputil.ReverseProxy{
		Director: func(r *http.Request) {
			host := r.Host
			r.URL.Scheme = "https"
			r.URL.Host = withPort(host)
			// The name stays what the client asked for; the transport derives
			// the TLS server name from URL.Host.
			r.Host = host
			// A nil entry keeps the proxy from adding the loopback address.
			r.Header["X-Forwarded-For"] = nil
			// The proxy's own bookkeeping never reaches the destination. The
			// sandbox id and the egress mode are how this hop learns what to
			// check; they are not facts a model provider has any use for.
			r.Header.Del("x-nestbox-sandbox")
			r.Header.Del("x-nestbox-source-ip")
			r.Header.Del("x-nestbox-egress-mode")
		},
		Transport:     transport,
		FlushInterval: -1, // streamed answers (SSE) must not wait for a full buffer
		// A pseudonymised request comes back with placeholders in the answer.
		// They are turned back here, in the hop that holds the plaintext.
		ModifyResponse: func(resp *http.Response) error {
			annotateResponse(resp)
			pairs, _ := resp.Request.Context().Value(restoreKey{}).([]restorePair)
			if err := restoreResponse(resp, pairs); err != nil {
				return err
			}
			tapLive(resp)
			return nil
		},
		ErrorHandler: func(w http.ResponseWriter, r *http.Request, err error) {
			log.Printf("egress-upstream: %s: %v", r.Host, err)
			http.Error(w, "upstream: "+err.Error(), http.StatusBadGateway)
		},
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if err := checkHost(r.Host); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		r = r.WithContext(context.WithValue(r.Context(), liveInfoKey{}, liveInfo{
			SandboxID:  r.Header.Get("x-nestbox-sandbox"),
			RequestRef: r.Header.Get("x-request-id"),
		}))
		if dlp != nil && dlp.apply(w, r) {
			return
		}
		proxy.ServeHTTP(w, traceUpstream(r))
	})
}

// checkHost refuses a Host header that is not a plain host name or address.
func checkHost(host string) error {
	if host == "" || strings.ContainsAny(host, "/\\@ \t\r\n") {
		return errors.New("the request names no usable host")
	}
	return nil
}

// withPort adds 443 to a host that names no port.
func withPort(host string) string {
	if _, _, err := net.SplitHostPort(host); err == nil {
		return host
	}
	return net.JoinHostPort(strings.Trim(host, "[]"), "443")
}

// loadRoots is the system list plus any extra PEM bundle.
func loadRoots(extraPath string) *x509.CertPool {
	pool, err := x509.SystemCertPool()
	if err != nil || pool == nil {
		pool = x509.NewCertPool()
	}
	if data, err := os.ReadFile(extraPath); err == nil {
		pool.AppendCertsFromPEM(data)
	} else {
		log.Printf("egress-upstream: no extra roots at %s: %v", extraPath, err)
	}
	return pool
}

// serveUpstream blocks, serving the re-encryption hop on addr.
func serveUpstream(addr string, roots *x509.CertPool) {
	dialer := &net.Dialer{Timeout: 10 * time.Second, KeepAlive: 30 * time.Second}
	server := &http.Server{
		Addr:              addr,
		Handler:           upstreamHandler(roots, dialer.DialContext),
		ReadHeaderTimeout: 15 * time.Second,
	}
	log.Printf("egress-upstream: re-encrypting on %s", addr)
	if err := server.ListenAndServe(); err != nil {
		log.Fatalf("egress-upstream: %v", err)
	}
}

// Command egress-mitm terminates TLS for the egress proxy.
//
// Envoy can decrypt a CONNECT tunnel, but it cannot mint a certificate: a TLS
// server must present a certificate when the handshake happens, before anyone
// could ask an API which certificate belongs to the requested name. So the one
// thing Envoy cannot do lives here, in a small loopback process:
//
//	client --TLS(SNI)--> egress-mitm --TLS(SNI)--> Envoy :10000 --> the internet
//
// The client's TLS bytes arrive from Envoy's CONNECT termination
// (`connect_config` in envoy.yaml). We answer with a leaf certificate for the
// requested SNI, signed by this node's CA, and forward the decrypted bytes to
// Envoy's second listener — re-encrypted with the same SNI so Envoy's access
// log can report the real server name from the handshake.
//
// What this program deliberately does not do: it has no policy, no allowlist,
// no HTTP parsing, and no log of its own. Every request is decided by the
// control API through Envoy's ext_authz filter, on both hops. A certificate
// broker that also decided would be a second policy.
//
// Trust: the CA key comes from a Docker secret (`/run/secrets/egress_ca.key`),
// never from the image and never from a bind mount, so it cannot be read from
// the host filesystem by anything that can read the proxy container's
// configuration. If the key is missing the process exits: an egress proxy that
// silently falls back to "no inspection" would be worse than one that does not
// start its terminator. Envoy keeps running and fails CONNECT closed.
package main

import (
	"bytes"
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"log"
	"math/big"
	"net"
	"os"
	"strings"
	"sync"
	"time"
)

const (
	// defaultListen is where Envoy's `tls_terminator` cluster points.
	defaultListen = "127.0.0.1:10001"
	// defaultInner is Envoy's `egress_terminated` listener.
	defaultInner = "127.0.0.1:10000"
	// defaultCACert and defaultCAKey are the Docker secret paths in the
	// compose service; see AGENTS.md for how they are mounted.
	defaultCACert = "/run/secrets/egress_ca.crt"
	defaultCAKey  = "/run/secrets/egress_ca.key"

	// leafLifetime is short because a leaf is cheap to mint and a stolen one
	// is worthless quickly. The cache renews an hour before expiry.
	leafLifetime = 24 * time.Hour
	renewBefore  = time.Hour
	// cacheLimit bounds memory if a sandbox walks a large number of hosts.
	cacheLimit = 1024
)

// authority mints leaf certificates from the node CA.
type authority struct {
	caCert  *x509.Certificate
	caKey   crypto.Signer
	leafKey *ecdsa.PrivateKey

	mu    sync.Mutex
	cache map[string]*tls.Certificate
}

// crypto is imported for the Signer interface the CA key is stored as.

func main() {
	listenAddr := envOr("NESTBOX_MITM_LISTEN", defaultListen)
	innerAddr := envOr("NESTBOX_EGRESS_INNER", defaultInner)
	caCertPath := envOr("NESTBOX_EGRESS_CA_CERT", defaultCACert)
	caKeyPath := envOr("NESTBOX_EGRESS_CA_KEY", defaultCAKey)

	auth, err := loadAuthority(caCertPath, caKeyPath)
	if err != nil {
		// Fail closed and loud. The container keeps running (Envoy is the
		// entrypoint's exec target), but no HTTPS request is terminated.
		log.Fatalf("egress-mitm: cannot load the node CA: %v", err)
	}

	listener, err := net.Listen("tcp", listenAddr)
	if err != nil {
		log.Fatalf("egress-mitm: cannot listen on %s: %v", listenAddr, err)
	}
	log.Printf("egress-mitm: terminating TLS on %s, forwarding to %s", listenAddr, innerAddr)

	server := &tls.Config{
		MinVersion: tls.VersionTLS12,
		// No ALPN: the broker speaks HTTP/1.1 on both sides. Offering h2 here
		// would promise a protocol this process cannot re-originate.
		GetCertificate: func(hello *tls.ClientHelloInfo) (*tls.Certificate, error) {
			name := strings.TrimSuffix(strings.ToLower(strings.TrimSpace(hello.ServerName)), ".")
			if name == "" {
				// Without SNI there is no name to sign for. A CONNECT to an IP
				// literal lands here; refusing is honest, and the client's
				// error says TLS failed rather than that it was inspected.
				return nil, errors.New("no SNI in ClientHello")
			}
			return auth.certificateFor(name)
		},
	}

	// The hop that leaves for the internet, verified here (see upstream.go).
	go serveUpstream(envOr("NESTBOX_EGRESS_UPSTREAM_LISTEN", defaultUpstream),
		loadRoots(envOr("NESTBOX_UPSTREAM_ROOTS", "/etc/envoy/mozilla-ca.pem")))

	for {
		conn, err := listener.Accept()
		if err != nil {
			log.Printf("egress-mitm: accept: %v", err)
			continue
		}
		go handle(conn, server, innerAddr)
	}
}

// handle terminates one client TLS connection and pipes it to Envoy.
func handle(conn net.Conn, cfg *tls.Config, innerAddr string) {
	defer conn.Close()

	// Envoy sends the sandbox's address ahead of the TLS bytes (PROXY protocol
	// v1). Without it the inner hop would see 127.0.0.1 and the control API could
	// not tell which sandbox is asking. A connection that does not start with the
	// header is not from Envoy and is dropped: this port is loopback only, and
	// guessing an identity would be worse than refusing.
	origin, err := readProxyHeader(conn)
	if err != nil {
		log.Printf("egress-mitm: no PROXY header from %s: %v", conn.RemoteAddr(), err)
		return
	}

	// The handshake is where the certificate is chosen. A failure here is a
	// client that cannot use this tunnel (no SNI, a name we cannot sign); it
	// is not a reason to keep the connection open.
	client := tls.Server(conn, cfg)
	if err := client.Handshake(); err != nil {
		log.Printf("egress-mitm: handshake with %s: %v", conn.RemoteAddr(), err)
		return
	}
	sni := client.ConnectionState().ServerName

	raw, err := net.DialTimeout("tcp", innerAddr, 3*time.Second)
	if err != nil {
		log.Printf("egress-mitm: connect to the inner listener for %q: %v", sni, err)
		return
	}
	defer raw.Close()
	// Hand the sandbox's address on, before the first TLS byte.
	if _, err := fmt.Fprintf(raw, "PROXY TCP4 %s 127.0.0.1 %d 10000\r\n", origin.ip, origin.port); err != nil {
		log.Printf("egress-mitm: write PROXY header: %v", err)
		return
	}
	inner := tls.Client(raw, &tls.Config{
		MinVersion: tls.VersionTLS12,
		// The inner hop is loopback and its certificate is generated at image
		// build time; the peer is authenticated by the address, not by PKI.
		// `ServerName` still matters: it is what Envoy logs as the SNI.
		ServerName:         sni,
		InsecureSkipVerify: true,
	})
	if err := inner.Handshake(); err != nil {
		log.Printf("egress-mitm: inner handshake for %q: %v", sni, err)
		return
	}
	defer inner.Close()

	done := make(chan struct{}, 2)
	go func() { _, _ = io.Copy(inner, client); done <- struct{}{} }()
	go func() { _, _ = io.Copy(client, inner); done <- struct{}{} }()
	<-done
}

// loadAuthority reads the node CA and prepares the shared leaf key.
func loadAuthority(certPath, keyPath string) (*authority, error) {
	certPEM, err := os.ReadFile(certPath)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", certPath, err)
	}
	keyPEM, err := os.ReadFile(keyPath)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", keyPath, err)
	}

	certBlock, _ := pem.Decode(certPEM)
	if certBlock == nil {
		return nil, fmt.Errorf("%s: no PEM certificate", certPath)
	}
	caCert, err := x509.ParseCertificate(certBlock.Bytes)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", certPath, err)
	}
	if !caCert.IsCA {
		return nil, fmt.Errorf("%s: not a CA certificate", certPath)
	}

	keyBlock, _ := pem.Decode(keyPEM)
	if keyBlock == nil {
		return nil, fmt.Errorf("%s: no PEM private key", keyPath)
	}
	caKey, err := parseECKey(keyBlock.Bytes)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", keyPath, err)
	}

	leafKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, fmt.Errorf("generate leaf key: %w", err)
	}

	return &authority{
		caCert:  caCert,
		caKey:   caKey,
		leafKey: leafKey,
		cache:   make(map[string]*tls.Certificate),
	}, nil
}

// certificateFor returns a cached or freshly minted certificate for one host.
func (a *authority) certificateFor(name string) (*tls.Certificate, error) {
	a.mu.Lock()
	defer a.mu.Unlock()

	if cached, ok := a.cache[name]; ok {
		if leaf, err := x509.ParseCertificate(cached.Certificate[0]); err == nil {
			if time.Until(leaf.NotAfter) > renewBefore {
				return cached, nil
			}
		}
	}

	leaf, err := a.mint(name)
	if err != nil {
		return nil, err
	}
	if len(a.cache) >= cacheLimit {
		// A sandbox that walks more hosts than this is not the case this
		// cache exists for; dropping it costs one signature per host.
		a.cache = make(map[string]*tls.Certificate)
	}
	a.cache[name] = leaf
	return leaf, nil
}

// mint signs one leaf certificate for `name`.
func (a *authority) mint(name string) (*tls.Certificate, error) {
	serial, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 128))
	if err != nil {
		return nil, fmt.Errorf("serial for %q: %w", name, err)
	}

	now := time.Now()
	template := &x509.Certificate{
		SerialNumber:          serial,
		Subject:               pkix.Name{CommonName: name, Organization: []string{"Nestbox egress inspection"}},
		NotBefore:             now.Add(-5 * time.Minute),
		NotAfter:              now.Add(leafLifetime),
		KeyUsage:              x509.KeyUsageDigitalSignature,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
		IsCA:                  false,
	}
	if ip := net.ParseIP(name); ip != nil {
		template.IPAddresses = []net.IP{ip}
	} else {
		template.DNSNames = []string{name}
	}

	der, err := x509.CreateCertificate(rand.Reader, template, a.caCert, a.leafKey.Public(), a.caKey)
	if err != nil {
		return nil, fmt.Errorf("sign %q: %w", name, err)
	}

	return &tls.Certificate{
		Certificate: [][]byte{der, a.caCert.Raw},
		PrivateKey:  a.leafKey,
		Leaf:        template,
	}, nil
}

// parseECKey accepts the SEC1 form `openssl ecparam -genkey` writes and the
// PKCS#8 form other tools produce.
func parseECKey(der []byte) (*ecdsa.PrivateKey, error) {
	if key, err := x509.ParseECPrivateKey(der); err == nil {
		return key, nil
	}
	parsed, err := x509.ParsePKCS8PrivateKey(der)
	if err != nil {
		return nil, errors.New("not an EC private key in SEC1 or PKCS#8 form")
	}
	key, ok := parsed.(*ecdsa.PrivateKey)
	if !ok {
		return nil, errors.New("not an EC private key")
	}
	return key, nil
}

func envOr(key, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(key)); value != "" {
		return value
	}
	return fallback
}

// origin is the sandbox's address as Envoy saw it.
type origin struct {
	ip   string
	port int
}

// readProxyHeader reads one PROXY protocol v1 line and nothing after it.
//
// One byte at a time on purpose: a buffered reader would swallow the start of
// the TLS ClientHello that follows, and the handshake would then hang.
func readProxyHeader(conn net.Conn) (origin, error) {
	_ = conn.SetReadDeadline(time.Now().Add(3 * time.Second))
	defer conn.SetReadDeadline(time.Time{})
	var line bytes.Buffer
	one := make([]byte, 1)
	for line.Len() < 108 {
		if _, err := io.ReadFull(conn, one); err != nil {
			return origin{}, err
		}
		line.WriteByte(one[0])
		if bytes.HasSuffix(line.Bytes(), []byte("\r\n")) {
			break
		}
	}
	fields := strings.Fields(strings.TrimSpace(line.String()))
	if len(fields) != 6 || fields[0] != "PROXY" || fields[1] != "TCP4" {
		return origin{}, fmt.Errorf("unexpected header %q", strings.TrimSpace(line.String()))
	}
	ip := net.ParseIP(fields[2])
	if ip == nil || ip.To4() == nil {
		return origin{}, fmt.Errorf("bad source address %q", fields[2])
	}
	port := 0
	if _, err := fmt.Sscanf(fields[4], "%d", &port); err != nil || port < 1 || port > 65535 {
		return origin{}, fmt.Errorf("bad source port %q", fields[4])
	}
	return origin{ip: ip.String(), port: port}, nil
}

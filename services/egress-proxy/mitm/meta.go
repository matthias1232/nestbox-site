package main

import (
	"context"
	"net"
	"net/http"
	"net/http/httptrace"
	"os"
	"strings"
	"sync"
	"time"
)

// What the far end of a request was, said to the console.
//
// The console lists a request with the address it reached, and — for a request
// this hop refused — the address it would have reached. Both are facts the
// request itself does not carry: the name in the Host header is what the model
// provider is called, the address is where the connection went, and a routing
// proxy in between makes the two differ. So the address is taken from the
// connection, not from the name, and the name is never written in its place.

// The two response headers this file adds. The refusal-side one is set by
// `dlp.go` before it writes the refusal status.
const (
	upstreamIPHeader     = "x-nestbox-upstream-ip"
	upstreamWouldHeader  = "x-nestbox-upstream-ip-would"
	routerProviderHeader = "x-nestbox-router-provider"
	routerModelHeader    = "x-nestbox-router-model"
)

// routerHeadersEnv names the upstream headers a routing proxy sets, provider
// first, model second, comma separated.
const routerHeadersEnv = "NESTBOX_ROUTER_HEADERS"

// wouldTTL is how long a "would have reached" address is reused. An address
// changes rarely and the answer is only shown to an operator; a minute-scale
// lookup per refusal would be a DNS query per refused request.
const wouldTTL = 5 * time.Minute

// upstreamAddr is the address the request's connection reached.
//
// The trace callback runs on the transport's goroutine and the response is
// annotated on another, so the field is guarded.
type upstreamAddr struct {
	mu sync.Mutex
	ip string
}

// set records the address of the connection the request went out on.
func (a *upstreamAddr) set(ip string) {
	if a == nil || ip == "" {
		return
	}
	a.mu.Lock()
	a.ip = ip
	a.mu.Unlock()
}

// get returns the recorded address, or "" when no connection was made.
func (a *upstreamAddr) get() string {
	if a == nil {
		return ""
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.ip
}

// upstreamAddrKey carries the *upstreamAddr of one request through its context.
type upstreamAddrKey struct{}

// traceUpstream attaches a connection trace that records the address the
// request reached.
//
// The trace travels inside the request context, which Go's proxy clones into
// the outbound request, so the transport reports the connection it opened for
// exactly this request — not the name the client asked for and not the first
// address a resolver happened to return.
func traceUpstream(r *http.Request) *http.Request {
	if r == nil {
		return r
	}
	addr := &upstreamAddr{}
	trace := &httptrace.ClientTrace{
		GotConn: func(info httptrace.GotConnInfo) {
			if info.Conn == nil {
				return
			}
			remote := info.Conn.RemoteAddr().String()
			host, _, err := net.SplitHostPort(remote)
			if err != nil {
				// A transport on a unix socket or a test double has no port.
				host = remote
			}
			addr.set(host)
		},
	}
	ctx := httptrace.WithClientTrace(context.WithValue(r.Context(), upstreamAddrKey{}, addr), trace)
	return r.WithContext(ctx)
}

// annotateResponse adds the upstream address and the router's own headers to
// an answer.
//
// The address comes from the trace `traceUpstream` attached; a response whose
// request was never traced gets no address header rather than a wrong one. The
// router headers are copied only when the routing proxy actually sent one: an
// empty `x-nestbox-router-provider` would read as "the router was asked and did
// not answer", which is a different fact from "there is no router".
func annotateResponse(resp *http.Response) {
	if resp == nil || resp.Header == nil {
		return
	}
	if resp.Request != nil {
		if addr, ok := resp.Request.Context().Value(upstreamAddrKey{}).(*upstreamAddr); ok {
			if ip := addr.get(); ip != "" {
				resp.Header.Set(upstreamIPHeader, ip)
			}
		}
	}
	names := routerHeaderNames()
	if len(names) > 0 {
		if value := resp.Header.Get(names[0]); value != "" {
			resp.Header.Set(routerProviderHeader, value)
		}
	}
	if len(names) > 1 {
		if value := resp.Header.Get(names[1]); value != "" {
			resp.Header.Set(routerModelHeader, value)
		}
	}
}

// routerHeaderNames splits NESTBOX_ROUTER_HEADERS into its positions.
//
// Read per response and not cached: the variable is set once by the start
// script, and a cache here would be a second place to look when a deployment
// changes it.
func routerHeaderNames() []string {
	raw := strings.TrimSpace(os.Getenv(routerHeadersEnv))
	if raw == "" {
		return nil
	}
	parts := strings.Split(raw, ",")
	names := make([]string, 0, len(parts))
	for _, part := range parts {
		names = append(names, strings.ToLower(strings.TrimSpace(part)))
	}
	return names
}

// wouldEntry is one cached answer of the resolver.
type wouldEntry struct {
	ip string
	at time.Time
}

// wouldCache holds the last address seen per host name.
var wouldCache sync.Map

// lookupWould returns the address a refused request would have reached.
//
// A refusal is answered before any connection is made, so there is no
// connection to ask; the name is resolved instead, with a short deadline so a
// slow resolver cannot hold the refusal open. Only successful answers are
// cached: caching "lookup failed" would keep a refusal without an address for
// five minutes after the resolver recovers.
func lookupWould(host string) string {
	host = hostOnly(host)
	if host == "" {
		return ""
	}
	if cached, ok := wouldCache.Load(host); ok {
		entry, _ := cached.(wouldEntry)
		if time.Since(entry.at) < wouldTTL {
			return entry.ip
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	addrs, err := net.DefaultResolver.LookupHost(ctx, host)
	if err != nil || len(addrs) == 0 {
		return ""
	}
	wouldCache.Store(host, wouldEntry{ip: addrs[0], at: time.Now()})
	return addrs[0]
}

// hostOnly returns a host without its port, and without the brackets IPv6
// literals are written in.
//
// The policy check is per name, not per port: a destination is one destination
// whether the client named 443 or 8443.
func hostOnly(host string) string {
	host = strings.TrimSpace(host)
	if host == "" {
		return ""
	}
	if name, _, err := net.SplitHostPort(host); err == nil {
		return strings.Trim(name, "[]")
	}
	return strings.Trim(host, "[]")
}

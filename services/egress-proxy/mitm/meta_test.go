package main

import (
	"context"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"testing"
)

// The console shows the address a request reached. It cannot learn it from the
// name the client asked for: a routing proxy makes the two differ, and a refusal
// has no connection at all. These tests pin both facts.

// tracedGet issues one request through the trace and returns its response.
func tracedGet(t *testing.T, upstream *httptest.Server) *http.Response {
	t.Helper()
	request, err := http.NewRequestWithContext(context.Background(), http.MethodGet, upstream.URL, nil)
	if err != nil {
		t.Fatalf("building the request: %v", err)
	}
	response, err := upstream.Client().Do(traceUpstream(request))
	if err != nil {
		t.Fatalf("requesting the upstream: %v", err)
	}
	t.Cleanup(func() { _ = response.Body.Close() })
	_, _ = io.Copy(io.Discard, response.Body)
	return response
}

func TestAnnotateResponseNamesTheAddressReached(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{}`)
	}))
	t.Cleanup(upstream.Close)

	response := tracedGet(t, upstream)
	annotateResponse(response)
	if got := response.Header.Get(upstreamIPHeader); got != "127.0.0.1" {
		t.Fatalf("%s is %q, want the loopback address the test server listens on", upstreamIPHeader, got)
	}
}

func TestAnnotateResponseCopiesTheRouterHeaders(t *testing.T) {
	t.Setenv(routerHeadersEnv, "x-omniroute-provider, x-omniroute-model")
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("x-omniroute-provider", "openrouter")
		w.Header().Set("x-omniroute-model", "deepseek-v4.1-flash-combo")
		_, _ = io.WriteString(w, `{}`)
	}))
	t.Cleanup(upstream.Close)

	response := tracedGet(t, upstream)
	annotateResponse(response)
	if got := response.Header.Get(routerProviderHeader); got != "openrouter" {
		t.Fatalf("%s is %q", routerProviderHeader, got)
	}
	if got := response.Header.Get(routerModelHeader); got != "deepseek-v4.1-flash-combo" {
		t.Fatalf("%s is %q", routerModelHeader, got)
	}
}

func TestAnnotateResponseWithoutARouterWritesNoRouterHeaders(t *testing.T) {
	t.Setenv(routerHeadersEnv, "")
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{}`)
	}))
	t.Cleanup(upstream.Close)

	response := tracedGet(t, upstream)
	annotateResponse(response)
	if got := response.Header.Get(routerProviderHeader); got != "" {
		t.Fatalf("no router configured, but %s is %q", routerProviderHeader, got)
	}
	if got := response.Header.Get(routerModelHeader); got != "" {
		t.Fatalf("no router configured, but %s is %q", routerModelHeader, got)
	}
}

func TestAnnotateResponseLeavesAnEmptyRouterHeaderAbsent(t *testing.T) {
	t.Setenv(routerHeadersEnv, "x-omniroute-provider,x-omniroute-model")
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{}`)
	}))
	t.Cleanup(upstream.Close)

	response := tracedGet(t, upstream)
	annotateResponse(response)
	// An empty value would say "the router was asked and did not answer"; the
	// sentence the console shows for that is not true here.
	if _, present := response.Header[http.CanonicalHeaderKey(routerProviderHeader)]; present {
		t.Fatalf("%s was written without a value: %v", routerProviderHeader, response.Header)
	}
}

func TestAnnotateResponseWithoutATraceWritesNoAddress(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{}`)
	}))
	t.Cleanup(upstream.Close)
	// No traceUpstream: this request carries no *upstreamAddr, and the answer
	// must say nothing rather than guess the name it was asked for.
	request, err := http.NewRequestWithContext(context.Background(), http.MethodGet, upstream.URL, nil)
	if err != nil {
		t.Fatalf("building the request: %v", err)
	}
	response, err := upstream.Client().Do(request)
	if err != nil {
		t.Fatalf("requesting the upstream: %v", err)
	}
	t.Cleanup(func() { _ = response.Body.Close() })
	_, _ = io.Copy(io.Discard, response.Body)

	annotateResponse(response)
	if got := response.Header.Get(upstreamIPHeader); got != "" {
		t.Fatalf("an untraced request got %s %q", upstreamIPHeader, got)
	}
	annotateResponse(nil) // must not panic
	annotateResponse(&http.Response{Header: http.Header{}})
}

func TestLookupWouldResolvesAndCachesTheName(t *testing.T) {
	first := lookupWould("localhost:8443")
	if first == "" {
		t.Fatal("localhost did not resolve; every refusal would be shown without an address")
	}
	if net.ParseIP(first) == nil {
		t.Fatalf("%q is not an address", first)
	}
	if second := lookupWould("localhost"); second != first {
		t.Fatalf("the cached answer changed from %q to %q", first, second)
	}
}

func TestLookupWouldLeavesAnUnknownNameEmpty(t *testing.T) {
	if got := lookupWould("this-name-does-not-exist.invalid"); got != "" {
		t.Fatalf("an unresolvable name produced %q", got)
	}
}

func TestHostOnlyDropsThePort(t *testing.T) {
	cases := map[string]string{
		"api.anthropic.com":       "api.anthropic.com",
		"api.anthropic.com:443":   "api.anthropic.com",
		"api.anthropic.com:8443":  "api.anthropic.com",
		"[::1]:443":               "::1",
		"[::1]":                   "::1",
		"127.0.0.1:10002":         "127.0.0.1",
		"  api.anthropic.com:443": "api.anthropic.com",
		"":                        "",
	}
	for host, want := range cases {
		if got := hostOnly(host); got != want {
			t.Errorf("hostOnly(%q) = %q, want %q", host, got, want)
		}
	}
}

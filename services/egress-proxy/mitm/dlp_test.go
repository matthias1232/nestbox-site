package main

import (
	"crypto/tls"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

// The hop must do exactly three things with an answer from the control API:
// refuse a blocked request, forward the rewritten text when values were replaced,
// and forward the original bytes when nothing matched. The fourth case — no
// answer at all — depends on the mode, and is the one that decides whether an
// outage of the control API takes the egress down.

func dlpTestClient(t *testing.T, verdict string) (*dlpClient, *[]string) {
	t.Helper()
	seen := &[]string{}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		*seen = append(*seen, string(body))
		if r.Header.Get("x-nestbox-node") == "" || !strings.HasPrefix(r.Header.Get("authorization"), "Bearer ") {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		w.Header().Set("content-type", "application/json")
		_, _ = io.WriteString(w, verdict)
	}))
	t.Cleanup(server.Close)
	return &dlpClient{url: server.URL, nodeID: "11111111-1111-1111-1111-111111111111", token: "t", client: server.Client()}, seen
}

func inspectRequest(body string, contentType string) *http.Request {
	request := httptest.NewRequest(http.MethodPost, "https://api.anthropic.com/v1/messages", strings.NewReader(body))
	request.Host = "api.anthropic.com"
	request.ContentLength = int64(len(body))
	request.Header.Set("content-type", contentType)
	request.Header.Set("x-nestbox-sandbox", "22222222-2222-2222-2222-222222222222")
	return request
}

func TestDlpRefusesABlockedBody(t *testing.T) {
	client, seen := dlpTestClient(t, `{"permitted":false,"action":"blocked","transformed_content":"x","findings":[]}`)
	recorder := httptest.NewRecorder()
	handled := client.apply(recorder, inspectRequest(`{"messages":[]}`, "application/json"))
	if !handled {
		t.Fatal("a blocked body must be answered here, not forwarded")
	}
	if recorder.Code != http.StatusForbidden {
		t.Fatalf("status %d, want 403", recorder.Code)
	}
	if !strings.Contains(recorder.Body.String(), "privacy.blocked") {
		t.Fatalf("body %q does not name the reason", recorder.Body.String())
	}
	if len(*seen) != 1 || !strings.Contains((*seen)[0], "22222222-2222-2222-2222-222222222222") {
		t.Fatalf("the check named the wrong sandbox: %v", *seen)
	}
	if !strings.Contains((*seen)[0], `"source":"egress"`) {
		t.Fatalf("the check did not say which hop saw it: %v", *seen)
	}
}

func TestDlpForwardsTheRewrittenBody(t *testing.T) {
	client, _ := dlpTestClient(t, `{"permitted":true,"action":"replaced","transformed_content":"{\"messages\":\"<IBAN_faf7e1c0>\"}","findings":[]}`)
	request := inspectRequest(`{"messages":"DE89370400440532013000"}`, "application/json")
	recorder := httptest.NewRecorder()
	if client.apply(recorder, request) {
		t.Fatal("a rewritten body is forwarded, not answered")
	}
	got, _ := io.ReadAll(request.Body)
	if string(got) != `{"messages":"<IBAN_faf7e1c0>"}` {
		t.Fatalf("body %q was not rewritten", got)
	}
	if request.ContentLength != int64(len(got)) {
		t.Fatalf("content length %d does not match %d bytes", request.ContentLength, len(got))
	}
}

func TestDlpLeavesAnUnmatchedBodyAlone(t *testing.T) {
	client, _ := dlpTestClient(t, `{"permitted":true,"action":"allowed","transformed_content":"","findings":[]}`)
	original := `{"messages":"nothing to see"}`
	request := inspectRequest(original, "application/json")
	if client.apply(httptest.NewRecorder(), request) {
		t.Fatal("an allowed body is forwarded")
	}
	got, _ := io.ReadAll(request.Body)
	if string(got) != original {
		t.Fatalf("body changed to %q", got)
	}
}

func TestDlpFailsClosedUntilItHasSeenThePolicy(t *testing.T) {
	// The control API is not there at all: the check cannot be answered.
	dead := &dlpClient{url: "http://127.0.0.1:1", nodeID: "n", token: "t", client: &http.Client{Transport: &http.Transport{TLSClientConfig: &tls.Config{}}}}
	recorder := httptest.NewRecorder()
	if !dead.apply(recorder, inspectRequest(`{"a":1}`, "application/json")) {
		t.Fatal("a body it could not check must not be forwarded when no policy is known")
	}
	if recorder.Code != http.StatusServiceUnavailable {
		t.Fatalf("status %d, want 503", recorder.Code)
	}
}

func TestDlpFailsOpenOnlyForObserve(t *testing.T) {
	dead := &dlpClient{url: "http://127.0.0.1:1", nodeID: "n", token: "t", client: &http.Client{Transport: &http.Transport{TLSClientConfig: &tls.Config{}}}}
	sandbox := "22222222-2222-2222-2222-222222222222"

	// The policy the sandbox runs under was reported as observe by an earlier
	// answer, so an outage may not turn it into a block.
	dead.modes.Store(sandbox, "observe")
	if dead.apply(httptest.NewRecorder(), inspectRequest(`{"a":1}`, "application/json")) {
		t.Fatal("observe forwards a body it could not check")
	}

	// The same outage under strict refuses.
	dead.modes.Store(sandbox, "strict")
	recorder := httptest.NewRecorder()
	if !dead.apply(recorder, inspectRequest(`{"a":1}`, "application/json")) {
		t.Fatal("strict must refuse a body it could not check")
	}
	if recorder.Code != http.StatusServiceUnavailable {
		t.Fatalf("status %d, want 503", recorder.Code)
	}
}

func TestDlpRefusesABodyItCannotRead(t *testing.T) {
	client, _ := dlpTestClient(t, `{"permitted":true,"action":"allowed","transformed_content":"","findings":[]}`)
	huge := strings.Repeat("x", maxInspectedBody+1)
	request := inspectRequest(huge, "application/json")
	recorder := httptest.NewRecorder()
	if !client.apply(recorder, request) {
		t.Fatal("a body over the limit must not be forwarded unchecked")
	}
	if recorder.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("status %d, want 413", recorder.Code)
	}
	if !strings.Contains(recorder.Body.String(), "privacy.too_large") {
		t.Fatalf("body %q does not name the reason", recorder.Body.String())
	}
}

func TestDlpIgnoresWhatItMustNotRead(t *testing.T) {
	client, seen := dlpTestClient(t, `{"permitted":true,"action":"allowed","transformed_content":"","findings":[]}`)
	cases := map[string]*http.Request{
		"no sandbox":   inspectRequest(`{"a":1}`, "application/json"),
		"not json":     inspectRequest(`{"a":1}`, "application/octet-stream"),
		"a get":        inspectRequest(`{}`, "application/json"),
		"an empty one": inspectRequest(``, "application/json"),
	}
	cases["no sandbox"].Header.Del("x-nestbox-sandbox")
	cases["a get"].Method = http.MethodGet
	if client.apply(httptest.NewRecorder(), cases["no sandbox"]) {
		t.Error("a request without a sandbox has nothing to check")
	}
	if client.apply(httptest.NewRecorder(), cases["not json"]) {
		t.Error("a binary body is not read")
	}
	if client.apply(httptest.NewRecorder(), cases["a get"]) {
		t.Error("a bodyless method is not read")
	}
	if client.apply(httptest.NewRecorder(), cases["an empty one"]) {
		t.Error("an empty body is not read")
	}
	if len(*seen) != 0 {
		t.Fatalf("the control API was asked about %d bodies it should not see", len(*seen))
	}
}

func TestDlpSendsTheRequestRef(t *testing.T) {
	client, seen := dlpTestClient(t, `{"permitted":true,"action":"allowed"}`)
	request := inspectRequest(`{"a":1}`, "application/json")
	request.Header.Set("x-request-id", "req_abc123")
	client.apply(httptest.NewRecorder(), request)
	if len(*seen) != 1 {
		t.Fatalf("the control API was asked %d times, want 1", len(*seen))
	}
	// Without the ref the console cannot tie the body check to the turn the
	// client asked for, and the check looks like an orphan row.
	if !strings.Contains((*seen)[0], `"request_ref":"req_abc123"`) {
		t.Fatalf("the check did not carry the request ref: %s", (*seen)[0])
	}
}

func TestDlpForwardsARecordedDestinationWhenTheCheckGoesDown(t *testing.T) {
	const sandbox = "22222222-2222-2222-2222-222222222222"
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("content-type", "application/json")
		// The policy enforces here in general, but this destination is only
		// recorded: the hop never had the right to stop it.
		_, _ = io.WriteString(w, `{"permitted":true,"action":"allowed","mode":"strict","enforcement":"record_only"}`)
	}))
	t.Cleanup(server.Close)
	client := &dlpClient{url: server.URL, nodeID: "n", token: "t", client: server.Client()}

	if client.apply(httptest.NewRecorder(), inspectRequest(`{"a":1}`, "application/json")) {
		t.Fatal("an allowed body is forwarded")
	}

	// The control API goes away.
	client.url = "http://127.0.0.1:1"
	client.client = &http.Client{}

	if client.apply(httptest.NewRecorder(), inspectRequest(`{"a":2}`, "application/json")) {
		t.Fatalf("a destination that was only recorded must not be refused by an outage (sandbox %s)", sandbox)
	}

	// A destination that was never recorded has no such standing, even though
	// the mode is the same: the outage may not become a way around it.
	other := inspectRequest(`{"a":3}`, "application/json")
	other.Host = "api.example.com"
	recorder := httptest.NewRecorder()
	if !client.apply(recorder, other) {
		t.Fatal("a destination that was never recorded must be refused when the check cannot answer")
	}
	if recorder.Code != http.StatusServiceUnavailable {
		t.Fatalf("status %d, want 503", recorder.Code)
	}
}

func TestDlpDropsAcceptEncodingWhenTheAnswerCarriesPlaceholders(t *testing.T) {
	client, _ := dlpTestClient(t, `{"permitted":true,"action":"allowed","findings":[],"restore":[{"placeholder":"<IBAN_1>","value":"DE89370400440532013000"}]}`)
	request := inspectRequest(`{"messages":"<IBAN_1>"}`, "application/json")
	request.Header.Set("Accept-Encoding", "gzip, br")
	if client.apply(httptest.NewRecorder(), request) {
		t.Fatal("an allowed body is forwarded")
	}
	// restore.go walks the answer; a compressed answer is opaque there and the
	// placeholder would reach the sandbox instead of the value it stands for.
	if got := request.Header.Get("Accept-Encoding"); got != "" {
		t.Fatalf("the request still asks for %q", got)
	}
}

func TestDlpKeepsCompressionWhenThereIsNothingToRestore(t *testing.T) {
	client, _ := dlpTestClient(t, `{"permitted":true,"action":"allowed","findings":[]}`)
	request := inspectRequest(`{"a":1}`, "application/json")
	request.Header.Set("Accept-Encoding", "gzip")
	if client.apply(httptest.NewRecorder(), request) {
		t.Fatal("an allowed body is forwarded")
	}
	if got := request.Header.Get("Accept-Encoding"); got != "gzip" {
		t.Fatalf("compression was turned off for a body with nothing to restore: %q", got)
	}
}

func TestDlpReportsARefusalWithoutAFinding(t *testing.T) {
	reports := make(chan string, 4)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		if r.URL.Path == "/api/v1/internal/privacy/refusal" {
			// Same credential as the body check: one identity, one rotation.
			if r.Header.Get("x-nestbox-node") == "" || !strings.HasPrefix(r.Header.Get("authorization"), "Bearer ") {
				w.WriteHeader(http.StatusUnauthorized)
				return
			}
			reports <- string(body)
			w.WriteHeader(http.StatusNoContent)
			return
		}
		w.Header().Set("content-type", "application/json")
		_, _ = io.WriteString(w, `{"permitted":true,"action":"allowed"}`)
	}))
	t.Cleanup(server.Close)
	client := &dlpClient{url: server.URL, nodeID: "n", token: "t", client: server.Client()}

	request := inspectRequest(`{"a":1}`, "application/json")
	// "localhost" keeps the "would have reached" lookup off the internet.
	request.Host = "localhost"
	request.ContentLength = maxInspectedBody + 1
	request.Header.Set("x-request-id", "req_413")
	recorder := httptest.NewRecorder()
	if !client.apply(recorder, request) {
		t.Fatal("a body over the limit must not be forwarded unchecked")
	}
	if recorder.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("status %d, want 413", recorder.Code)
	}
	if got := recorder.Header().Get(upstreamWouldHeader); got == "" {
		t.Fatal("the refusal does not say which address the request would have reached")
	}

	select {
	case body := <-reports:
		for _, want := range []string{
			`"kind":"too_large"`,
			`"sandbox_id":"22222222-2222-2222-2222-222222222222"`,
			`"destination":"localhost"`,
			`"method":"POST"`,
			`"path":"/v1/messages"`,
			`"request_ref":"req_413"`,
		} {
			if !strings.Contains(body, want) {
				t.Fatalf("the refusal report does not carry %s: %s", want, body)
			}
		}
	case <-time.After(3 * time.Second):
		t.Fatal("the refusal was not reported; the console would show a gap where a refusal was")
	}
}

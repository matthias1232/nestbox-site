package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

// A tap may never change what the sandbox reads, and it must report the whole
// answer even when the provider sends it in many pieces.

func liveRequest() *http.Request {
	request := httptest.NewRequest(http.MethodPost, "https://api.anthropic.com/v1/messages", nil)
	return request.WithContext(context.WithValue(request.Context(), liveInfoKey{}, liveInfo{
		SandboxID:  "22222222-2222-2222-2222-222222222222",
		RequestRef: "req-1",
	}))
}

func liveResponse(request *http.Request, body string) *http.Response {
	return &http.Response{
		StatusCode:    200,
		Header:        http.Header{"Content-Type": []string{"text/event-stream"}},
		Body:          io.NopCloser(strings.NewReader(body)),
		ContentLength: int64(len(body)),
		Request:       request,
	}
}

func TestTapPassesEveryByteThroughAndReportsTheAnswer(t *testing.T) {
	var mu sync.Mutex
	var reports []map[string]any
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		raw, _ := io.ReadAll(r.Body)
		var body map[string]any
		_ = json.Unmarshal(raw, &body)
		mu.Lock()
		reports = append(reports, body)
		mu.Unlock()
		w.WriteHeader(http.StatusNoContent)
	}))
	defer server.Close()
	liveOverride = &dlpClient{url: server.URL, nodeID: "n", token: "t", client: server.Client()}
	defer func() { liveOverride = nil }()

	stream := ""
	var want strings.Builder
	var events []string
	for _, text := range []string{"Hal", "lo ", "Welt"} {
		events = append(events, `data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"`+text+`"}}`)
		want.WriteString(text)
	}
	events = append(events, `data: {"type":"message_delta","usage":{"output_tokens":3}}`)
	stream = strings.Join(events, "\n") + "\n"

	resp := liveResponse(liveRequest(), stream)
	tapLive(resp)
	got, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != stream {
		t.Fatalf("the stream was changed:\n%q\n%q", got, stream)
	}
	_ = resp.Body.Close()

	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		mu.Lock()
		found := false
		for _, report := range reports {
			if report["text_so_far"] == want.String() && report["done"] == true {
				found = true
			}
		}
		mu.Unlock()
		if found {
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("no complete report arrived: %v", reports)
}

func TestTapIgnoresAnAnswerThatIsNotAStream(t *testing.T) {
	request := liveRequest()
	resp := &http.Response{
		Header:  http.Header{"Content-Type": []string{"application/json"}},
		Body:    io.NopCloser(strings.NewReader(`{}`)),
		Request: request,
	}
	original := resp.Body
	tapLive(resp)
	if resp.Body != original {
		t.Fatal("a JSON answer must not be wrapped")
	}
}

func TestTapIgnoresARequestWithoutAnIdentity(t *testing.T) {
	request := httptest.NewRequest(http.MethodPost, "https://api.anthropic.com/v1/messages", nil)
	resp := liveResponse(request, "data: {}\n")
	original := resp.Body
	tapLive(resp)
	if resp.Body != original {
		t.Fatal("an unidentified request must not be tapped")
	}
}

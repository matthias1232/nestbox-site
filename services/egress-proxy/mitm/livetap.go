package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"log"
	"net/http"
	"strings"
	"sync"
	"time"
)

// tapLive watches a streamed answer and reports it live.
//
// The sandbox gets exactly the bytes the provider sent: this reader copies them
// through unchanged and reads a second time for the report. The control API masks
// what arrives, so a live viewer sees the same masked text the stored turn will
// hold, and nothing is written down twice.
//
// A report is sent at most every 400 ms, and one when the stream ends. A failed
// report is logged and forgotten: a live view may miss a second, a stream may not
// stall.

// liveReportGap is the shortest gap between two reports of the same answer.
const liveReportGap = 400 * time.Millisecond

// liveTextMax bounds what one report carries; the response side never needs more.
const liveTextMax = 16 * 1024

// liveInfoKey carries the sandbox and the request id to the response side.
type liveInfoKey struct{}

// liveInfo is what a live report needs to know about the request.
type liveInfo struct {
	SandboxID  string
	RequestRef string
}

// tapLive wraps a streamed answer, if this response is one and the request was
// identified.
func tapLive(resp *http.Response) {
	info, _ := resp.Request.Context().Value(liveInfoKey{}).(liveInfo)
	if info.SandboxID == "" || info.RequestRef == "" {
		return
	}
	if !strings.Contains(strings.ToLower(resp.Header.Get("content-type")), "text/event-stream") {
		return
	}
	client := liveClient()
	if client == nil {
		return
	}
	resp.Body = &liveReader{source: resp.Body, info: info, client: client}
}

var (
	liveOnce sync.Once
	// liveOverride lets a test point the reports at its own server.
	liveOverride *dlpClient
	liveShared   *dlpClient
)

// liveClient is the same identity the content check uses: one credential, one
// place to rotate it.
func liveClient() *dlpClient {
	if liveOverride != nil {
		return liveOverride
	}
	liveOnce.Do(func() { liveShared = newDlpClientFromEnv() })
	return liveShared
}

// liveReader passes bytes through and reads them a second time for the report.
type liveReader struct {
	source io.ReadCloser
	info   liveInfo
	client *dlpClient

	pending []byte
	text    strings.Builder
	inTok   int64
	outTok  int64
	last    time.Time
	busy    bool
	done    bool
	mu      sync.Mutex
}

func (l *liveReader) Read(p []byte) (int, error) {
	n, err := l.source.Read(p)
	if n > 0 {
		l.feed(p[:n])
		l.report(false)
	}
	if err != nil {
		l.report(true)
	}
	return n, err
}

func (l *liveReader) Close() error {
	l.report(true)
	return l.source.Close()
}

// feed reads complete `data:` lines and takes the answer text out of them.
func (l *liveReader) feed(chunk []byte) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.pending = append(l.pending, chunk...)
	for {
		index := bytes.IndexByte(l.pending, '\n')
		if index < 0 {
			if len(l.pending) > 4<<20 {
				l.pending = nil
			}
			return
		}
		line := strings.TrimSpace(string(l.pending[:index]))
		l.pending = l.pending[index+1:]
		if !strings.HasPrefix(line, "data:") {
			continue
		}
		payload := strings.TrimSpace(strings.TrimPrefix(line, "data:"))
		if payload == "" || payload == "[DONE]" {
			continue
		}
		var event map[string]any
		if err := json.Unmarshal([]byte(payload), &event); err != nil {
			continue
		}
		l.consume(event)
	}
}

// consume takes the text and the counters out of one streamed event.
func (l *liveReader) consume(event map[string]any) {
	if l.text.Len() < liveTextMax {
		if delta, ok := event["delta"].(map[string]any); ok {
			if text, ok := delta["text"].(string); ok {
				l.text.WriteString(text)
			}
		}
		if text, ok := event["delta"].(string); ok {
			l.text.WriteString(text)
		}
		if choices, ok := event["choices"].([]any); ok && len(choices) > 0 {
			if choice, ok := choices[0].(map[string]any); ok {
				if delta, ok := choice["delta"].(map[string]any); ok {
					if text, ok := delta["content"].(string); ok {
						l.text.WriteString(text)
					}
				}
			}
		}
	}
	number := func(value any) (int64, bool) {
		if number, ok := value.(float64); ok {
			return int64(number), true
		}
		return 0, false
	}
	if message, ok := event["message"].(map[string]any); ok {
		if usage, ok := message["usage"].(map[string]any); ok {
			if value, ok := number(usage["input_tokens"]); ok {
				l.inTok = value
			}
		}
	}
	if usage, ok := event["usage"].(map[string]any); ok {
		if value, ok := number(usage["output_tokens"]); ok {
			l.outTok = value
		}
		if value, ok := number(usage["completion_tokens"]); ok {
			l.outTok = value
		}
	}
}

// report sends what is known so far, if enough time has passed or the stream is
// over. It never blocks the read: a busy report is skipped.
func (l *liveReader) report(done bool) {
	l.mu.Lock()
	if l.done || (l.busy && !done) {
		l.mu.Unlock()
		return
	}
	now := time.Now()
	if !done && !l.last.IsZero() && now.Sub(l.last) < liveReportGap {
		l.mu.Unlock()
		return
	}
	l.last = now
	l.busy = true
	if done {
		l.done = true
	}
	body := map[string]any{
		"request_ref":   l.info.RequestRef,
		"sandbox_id":    l.info.SandboxID,
		"text_so_far":   l.text.String(),
		"input_tokens":  l.inTok,
		"output_tokens": l.outTok,
		"done":          done,
	}
	l.mu.Unlock()

	go func() {
		defer func() {
			l.mu.Lock()
			l.busy = false
			l.mu.Unlock()
		}()
		payload, err := json.Marshal(body)
		if err != nil {
			return
		}
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		request, err := http.NewRequestWithContext(
			ctx,
			http.MethodPost,
			l.client.url+"/api/v1/internal/chats/live",
			bytes.NewReader(payload),
		)
		if err != nil {
			return
		}
		request.Header.Set("content-type", "application/json")
		request.Header.Set("x-nestbox-node", l.client.nodeID)
		request.Header.Set("authorization", "Bearer "+l.client.token)
		response, err := l.client.client.Do(request)
		if err != nil {
			log.Printf("egress-live: %s: %v", l.info.RequestRef, err)
			return
		}
		_ = response.Body.Close()
	}()
}

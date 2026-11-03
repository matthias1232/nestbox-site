package main

import (
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"
)

// The answer goes back to an agent that has to keep working, so a placeholder in
// it must become the value again — in a whole JSON answer, in a streamed text
// delta, and in a streamed tool input that is itself JSON.

func response(contentType, body string) *http.Response {
	return &http.Response{
		StatusCode:    200,
		Header:        http.Header{"Content-Type": []string{contentType}},
		Body:          io.NopCloser(strings.NewReader(body)),
		ContentLength: int64(len(body)),
		Request:       &http.Request{},
	}
}

func TestRestoreInAWholeJsonAnswer(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<PERSON_NAMES_1a2b3c4d>", Value: `Max "M" Mustermann`}}
	resp := response("application/json", `{"content":[{"type":"text","text":"Hallo <PERSON_NAMES_1a2b3c4d>!"}]}`)
	if err := restoreResponse(resp, pairs); err != nil {
		t.Fatal(err)
	}
	got, _ := io.ReadAll(resp.Body)
	want := `{"content":[{"type":"text","text":"Hallo Max \"M\" Mustermann!"}]}`
	if string(got) != want {
		t.Fatalf("body %q, want %q", got, want)
	}
	if resp.ContentLength != int64(len(want)) {
		t.Fatalf("content length %d does not match %d bytes", resp.ContentLength, len(want))
	}
}

func TestRestoreInAStreamedTextDelta(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<IBAN_faf7e1c0>", Value: "DE89370400440532013000"}}
	stream := strings.Join([]string{
		`data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Die IBAN ist <IBAN_"}}`,
		`data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"faf7e1c0>."}}`,
		`data: {"type":"content_block_stop","index":0}`,
		"",
	}, "\n")
	resp := response("text/event-stream", stream)
	if err := restoreResponse(resp, pairs); err != nil {
		t.Fatal(err)
	}
	got, _ := io.ReadAll(resp.Body)
	// The placeholder was split over two deltas; the client concatenates the
	// text, so the assertion does too.
	var text strings.Builder
	for _, line := range strings.Split(string(got), "\n") {
		payload := strings.TrimPrefix(line, "data: ")
		if payload == "" || payload == line {
			continue
		}
		var event map[string]any
		if err := json.Unmarshal([]byte(payload), &event); err != nil {
			continue
		}
		if delta, ok := event["delta"].(map[string]any); ok {
			if value, ok := delta["text"].(string); ok {
				text.WriteString(value)
			}
		}
	}
	if text.String() != "Die IBAN ist DE89370400440532013000." {
		t.Fatalf("reassembled text %q", text.String())
	}
}

func TestRestoreInAStreamedToolInput(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<HOSTNAMES_1234abcd>", Value: `intranet.firma.local`}}
	stream := `data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"url\":\"<HOSTNAMES_1234abcd>\"}"}}` + "\n"
	resp := response("text/event-stream", stream)
	if err := restoreResponse(resp, pairs); err != nil {
		t.Fatal(err)
	}
	got, _ := io.ReadAll(resp.Body)
	if !strings.Contains(string(got), "intranet.firma.local") {
		t.Fatalf("tool input was not restored: %q", got)
	}
}

func TestRestoreLeavesUnknownPlaceholders(t *testing.T) {
	resp := response("application/json", `{"text":"<OTHER_ffffffff>"}`)
	if err := restoreResponse(resp, []restorePair{{Placeholder: "<IBAN_1>", Value: "x"}}); err != nil {
		t.Fatal(err)
	}
	got, _ := io.ReadAll(resp.Body)
	if string(got) != `{"text":"<OTHER_ffffffff>"}` {
		t.Fatalf("body changed to %q", got)
	}
}

func TestRestoreLeavesOtherContentTypesAlone(t *testing.T) {
	original := `{"text":"<IBAN_1>"}`
	resp := response("text/plain", original)
	if err := restoreResponse(resp, []restorePair{{Placeholder: "<IBAN_1>", Value: "x"}}); err != nil {
		t.Fatal(err)
	}
	got, _ := io.ReadAll(resp.Body)
	if string(got) != original {
		t.Fatalf("body changed to %q", got)
	}
}

// A value has to be escaped the way JSON defines it, not the way Go quotes a
// string: a bell has no `\a` in JSON, and an agent would refuse the answer.
func TestRestoreEscapesAValueTheJsonWay(t *testing.T) {
	value := "Tab\tEmoji😀\x07 \"q\""
	pairs := []restorePair{{Placeholder: "<CUSTOM_abc12345>", Value: value}}
	resp := response("application/json", `{"content":[{"type":"text","text":"Wert: <CUSTOM_abc12345>"}]}`)
	if err := restoreResponse(resp, pairs); err != nil {
		t.Fatal(err)
	}
	got, _ := io.ReadAll(resp.Body)
	if !json.Valid(got) {
		t.Fatalf("restored body is not valid JSON: %q", got)
	}
	var answer struct {
		Content []struct {
			Text string `json:"text"`
		} `json:"content"`
	}
	if err := json.Unmarshal(got, &answer); err != nil {
		t.Fatal(err)
	}
	if len(answer.Content) != 1 || answer.Content[0].Text != "Wert: "+value {
		t.Fatalf("text came back as %q, want %q", answer.Content[0].Text, "Wert: "+value)
	}
}

// sseEvent renders one event the way an upstream server sends it.
func sseEvent(t *testing.T, event map[string]any) string {
	t.Helper()
	encoded, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	return "data: " + string(encoded)
}

// restoreStream runs a stream through the restorer and returns every line of
// the answer, so a test can look at the order as well as at the content.
func restoreStream(t *testing.T, pairs []restorePair, lines ...string) []string {
	t.Helper()
	resp := response("text/event-stream", strings.Join(lines, "\n")+"\n")
	if err := restoreResponse(resp, pairs); err != nil {
		t.Fatal(err)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	return strings.Split(strings.TrimSuffix(string(body), "\n"), "\n")
}

// dataEvents parses the data lines in the order they were written. Every one of
// them has to be JSON: a mangled event would break the client.
func dataEvents(t *testing.T, lines []string) []map[string]any {
	t.Helper()
	var events []map[string]any
	for _, line := range lines {
		if !strings.HasPrefix(line, "data: ") {
			continue
		}
		var event map[string]any
		if err := json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &event); err != nil {
			t.Fatalf("line %q is not JSON: %v", line, err)
		}
		events = append(events, event)
	}
	return events
}

// anthropicDelta returns the delta map of an Anthropic event, or nil.
func anthropicDelta(event map[string]any) map[string]any {
	delta, _ := event["delta"].(map[string]any)
	return delta
}

// An Anthropic tool input is JSON inside JSON, and the model splits it wherever
// it likes. Holding the tail and prefixing it to the next fragment is what makes
// the assembled document valid again.
func TestRestoreAcrossFragmentedToolInput(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<HOSTNAMES_1234abcd>", Value: "intranet.firma.local"}}
	fragments := []string{`{"url":"<HOSTNA`, `MES_1234ab`, `cd>","ok":true}`}
	var stream []string
	for _, fragment := range fragments {
		stream = append(stream, sseEvent(t, map[string]any{
			"type":  "content_block_delta",
			"index": 0,
			"delta": map[string]any{"type": "input_json_delta", "partial_json": fragment},
		}))
	}
	stream = append(stream, sseEvent(t, map[string]any{"type": "content_block_stop", "index": 0}))

	events := dataEvents(t, restoreStream(t, pairs, stream...))
	var assembled strings.Builder
	for _, event := range events {
		if fragment, ok := anthropicDelta(event)["partial_json"].(string); ok {
			assembled.WriteString(fragment)
		}
	}
	if !json.Valid([]byte(assembled.String())) {
		t.Fatalf("assembled tool input is not valid JSON: %q", assembled.String())
	}
	var input struct {
		URL string `json:"url"`
		OK  bool   `json:"ok"`
	}
	if err := json.Unmarshal([]byte(assembled.String()), &input); err != nil {
		t.Fatal(err)
	}
	if input.URL != "intranet.firma.local" || !input.OK {
		t.Fatalf("tool input came back as %+v", input)
	}
}

// A block can end in the middle of a placeholder. The rest is then text as it
// stands, but it must not be written into the stop event: that event has no
// text, and a client that renders deltas would drop it.
func TestRestoreWritesAHeldTextTailBeforeTheBlockStop(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<PERSON_NAMES_1a2b3c4d>", Value: "Max Mustermann"}}
	lines := restoreStream(t, pairs,
		sseEvent(t, map[string]any{
			"type":  "content_block_delta",
			"index": 0,
			"delta": map[string]any{"type": "text_delta", "text": "Hallo <PERSON_NAMES_1a2b3c4"},
		}),
		sseEvent(t, map[string]any{"type": "content_block_stop", "index": 0}),
	)
	events := dataEvents(t, lines)
	if len(events) != 3 {
		t.Fatalf("%d events, want delta, tail delta, stop: %v", len(events), lines)
	}
	var text strings.Builder
	for _, event := range events[:2] {
		if event["type"] != "content_block_delta" {
			t.Fatalf("event %v is not a delta", event)
		}
		value, _ := anthropicDelta(event)["text"].(string)
		text.WriteString(value)
	}
	if text.String() != "Hallo <PERSON_NAMES_1a2b3c4" {
		t.Fatalf("reassembled text %q", text.String())
	}
	if events[2]["type"] != "content_block_stop" {
		t.Fatalf("last event is %v, want the stop", events[2])
	}
	if _, present := events[2]["delta"]; present {
		t.Fatalf("the stop event carries a delta: %v", events[2])
	}
	// The inserted event has to be closed before the stop line starts.
	for position, line := range lines {
		if strings.Contains(line, "content_block_stop") {
			if position == 0 || lines[position-1] != "" {
				t.Fatalf("no blank line before the stop: %q", lines)
			}
		}
	}
}

// The same for a tool input: the held fragment comes first, as a delta.
func TestRestoreWritesAHeldToolTailBeforeTheBlockStop(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<HOSTNAMES_1234abcd>", Value: "intranet.firma.local"}}
	lines := restoreStream(t, pairs,
		sseEvent(t, map[string]any{
			"type":  "content_block_delta",
			"index": 1,
			"delta": map[string]any{"type": "input_json_delta", "partial_json": `{"url":"<HOSTNAMES_`},
		}),
		sseEvent(t, map[string]any{"type": "content_block_stop", "index": 1}),
	)
	events := dataEvents(t, lines)
	if len(events) != 3 {
		t.Fatalf("%d events, want delta, tail delta, stop: %v", len(events), lines)
	}
	if events[0]["type"] != "content_block_delta" || events[1]["type"] != "content_block_delta" {
		t.Fatalf("the tail did not become a delta: %v", events)
	}
	if got := anthropicDelta(events[1])["partial_json"]; got != "<HOSTNAMES_" {
		t.Fatalf("held fragment %q", got)
	}
	if events[2]["type"] != "content_block_stop" {
		t.Fatalf("last event is %v, want the stop", events[2])
	}
}

// openAIContent reassembles the answer of an OpenAI chat stream.
func openAIContent(t *testing.T, events []map[string]any) string {
	t.Helper()
	var text strings.Builder
	for _, event := range events {
		choices, _ := event["choices"].([]any)
		for _, raw := range choices {
			choice, _ := raw.(map[string]any)
			delta, _ := choice["delta"].(map[string]any)
			content, _ := delta["content"].(string)
			text.WriteString(content)
		}
	}
	return text.String()
}

func TestRestoreInAnOpenAiChatStream(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<PERSON_NAMES_1a2b3c4d>", Value: "Max Mustermann"}}
	chunk := func(delta map[string]any, finish any) string {
		return sseEvent(t, map[string]any{
			"id":      "chatcmpl-1",
			"object":  "chat.completion.chunk",
			"model":   "test",
			"choices": []any{map[string]any{"index": 0, "delta": delta, "finish_reason": finish}},
		})
	}
	events := dataEvents(t, restoreStream(t, pairs,
		chunk(map[string]any{"content": "Hallo <PERSON_NAMES_"}, nil),
		chunk(map[string]any{"content": "1a2b3c4d>"}, nil),
		chunk(map[string]any{}, "stop"),
	))
	if got := openAIContent(t, events); got != "Hallo Max Mustermann" {
		t.Fatalf("reassembled answer %q", got)
	}
}

func TestRestoreInAnOpenAiToolCallStream(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<HOSTNAMES_1234abcd>", Value: "intranet.firma.local"}}
	arguments := func(fragment string) string {
		return sseEvent(t, map[string]any{
			"id":     "chatcmpl-2",
			"object": "chat.completion.chunk",
			"choices": []any{map[string]any{
				"index": 0,
				"delta": map[string]any{"tool_calls": []any{
					map[string]any{"index": 0, "function": map[string]any{"arguments": fragment}},
				}},
				"finish_reason": nil,
			}},
		})
	}
	events := dataEvents(t, restoreStream(t, pairs,
		arguments(`{"url":"<HOSTNA`),
		arguments(`MES_1234abcd>"}`),
		sseEvent(t, map[string]any{
			"id":     "chatcmpl-2",
			"object": "chat.completion.chunk",
			"choices": []any{map[string]any{
				"index": 0, "delta": map[string]any{}, "finish_reason": "tool_calls",
			}},
		}),
	))
	var assembled strings.Builder
	for _, event := range events {
		choices, _ := event["choices"].([]any)
		for _, raw := range choices {
			choice, _ := raw.(map[string]any)
			delta, _ := choice["delta"].(map[string]any)
			calls, _ := delta["tool_calls"].([]any)
			for _, rawCall := range calls {
				call, _ := rawCall.(map[string]any)
				function, _ := call["function"].(map[string]any)
				fragment, _ := function["arguments"].(string)
				assembled.WriteString(fragment)
			}
		}
	}
	if !json.Valid([]byte(assembled.String())) {
		t.Fatalf("assembled arguments are not valid JSON: %q", assembled.String())
	}
	var call struct {
		URL string `json:"url"`
	}
	if err := json.Unmarshal([]byte(assembled.String()), &call); err != nil {
		t.Fatal(err)
	}
	if call.URL != "intranet.firma.local" {
		t.Fatalf("arguments came back as %+v", call)
	}
}

// A choice that reports a finish reason sends no delta afterwards, so the held
// rest goes out as a chunk of its own, before that finish chunk.
func TestRestoreWritesHeldTailsBeforeTheFinishChunk(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<PERSON_NAMES_1a2b3c4d>", Value: "Max Mustermann"}}
	chunk := func(delta map[string]any, finish any) string {
		return sseEvent(t, map[string]any{
			"id":      "chatcmpl-3",
			"object":  "chat.completion.chunk",
			"choices": []any{map[string]any{"index": 0, "delta": delta, "finish_reason": finish}},
		})
	}
	events := dataEvents(t, restoreStream(t, pairs,
		chunk(map[string]any{"content": "Hallo <PERSON_NAMES_1a2b"}, nil),
		chunk(map[string]any{}, "stop"),
	))
	if got := openAIContent(t, events); got != "Hallo <PERSON_NAMES_1a2b" {
		t.Fatalf("reassembled answer %q", got)
	}
	if len(events) != 3 {
		t.Fatalf("%d events, want delta, tail chunk, finish: %v", len(events), events)
	}
	choices, _ := events[2]["choices"].([]any)
	choice, _ := choices[0].(map[string]any)
	if choice["finish_reason"] != "stop" {
		t.Fatalf("last event is %v, want the finish chunk", events[2])
	}
	delta, _ := choice["delta"].(map[string]any)
	if content, present := delta["content"]; present && content != "" {
		t.Fatalf("the finish chunk carries the rest: %v", events[2])
	}
}

// The Responses API streams a plain `delta` string and closes with a `done`
// event that carries the whole text.
func TestRestoreInAnOpenAiResponsesStream(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<IBAN_faf7e1c0>", Value: "DE89370400440532013000"}}
	delta := func(text string) string {
		return sseEvent(t, map[string]any{
			"type":          "response.output_text.delta",
			"item_id":       "msg_1",
			"output_index":  0,
			"content_index": 0,
			"delta":         text,
		})
	}
	done := sseEvent(t, map[string]any{
		"type":          "response.output_text.done",
		"item_id":       "msg_1",
		"output_index":  0,
		"content_index": 0,
		"text":          "Die IBAN ist <IBAN_faf7e1c0>.",
	})
	events := dataEvents(t, restoreStream(t, pairs,
		delta("Die IBAN ist <IBAN_"),
		delta("faf7e1c0>."),
		done,
	))
	var text strings.Builder
	for _, event := range events {
		if value, ok := event["delta"].(string); ok {
			text.WriteString(value)
		}
	}
	if text.String() != "Die IBAN ist DE89370400440532013000." {
		t.Fatalf("reassembled answer %q", text.String())
	}
}

// A response can stop mid-placeholder: the rest is emitted before the done event.
func TestRestoreWritesAHeldResponsesTailBeforeDone(t *testing.T) {
	pairs := []restorePair{{Placeholder: "<IBAN_faf7e1c0>", Value: "DE89370400440532013000"}}
	lines := restoreStream(t, pairs,
		sseEvent(t, map[string]any{
			"type":          "response.output_text.delta",
			"item_id":       "msg_2",
			"output_index":  0,
			"content_index": 0,
			"delta":         "Die IBAN ist <IBAN_faf7",
		}),
		sseEvent(t, map[string]any{
			"type":  "response.output_text.done",
			"text":  "Die IBAN ist <IBAN_faf7",
			"delta": nil,
		}),
	)
	events := dataEvents(t, lines)
	if len(events) != 3 {
		t.Fatalf("%d events, want delta, tail delta, done: %v", len(events), lines)
	}
	if events[1]["type"] != "response.output_text.delta" {
		t.Fatalf("the tail did not become a delta: %v", events[1])
	}
	if got := events[1]["delta"]; got != "<IBAN_faf7" {
		t.Fatalf("held tail %q", got)
	}
	if events[2]["type"] != "response.output_text.done" {
		t.Fatalf("last event is %v, want the done event", events[2])
	}
}

// An unknown placeholder is not ours to touch, in a stream either.
func TestRestoreLeavesUnknownPlaceholdersInAStream(t *testing.T) {
	lines := restoreStream(t, []restorePair{{Placeholder: "<IBAN_1abcdef>", Value: "x"}},
		sseEvent(t, map[string]any{
			"type":  "content_block_delta",
			"index": 0,
			"delta": map[string]any{"type": "text_delta", "text": "Hallo <OTHER_ffffffff>"},
		}),
	)
	if !strings.Contains(strings.Join(lines, "\n"), "<OTHER_ffffffff>") {
		t.Fatalf("unknown placeholder was dropped: %v", lines)
	}
}

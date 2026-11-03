package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"sort"
	"strconv"
	"strings"
)

// Turn placeholders back into the values they stood for.
//
// With `pseudonymize` the model never sees a real value: it sees
// `<PERSON_NAMES_1a2b3c4d>` and answers with it. The agent must still be able to
// work, so the answer is restored on the way back — here, in the hop that holds
// the plaintext. The list of pairs lives only in the memory of this request and
// is never written anywhere (privacy module, section 6).
//
// Three shapes have to be handled, and all of them split the placeholder across
// chunks:
//
//   - a whole JSON answer, where the placeholder sits inside a string literal, so
//     the value has to be escaped as JSON string content;
//   - Anthropic server-sent events, where `delta.text` is plain and
//     `delta.partial_json` carries a JSON fragment that is itself escaped, so the
//     value needs one more level of escaping;
//   - OpenAI streams, where the answer is `choices[i].delta.content`, the tool
//     arguments are `choices[i].delta.tool_calls[j].function.arguments`, and the
//     Responses API streams a plain `delta`.
//
// A delta can end in the middle of a placeholder, because the model emits one
// token at a time. The tail that could still become a placeholder is held back
// and prefixed to the next delta of the same block. Whatever is still held when
// the block, the choice or the stream ends is written as an event of its own
// before the event that ends it — an end event carries no text, so putting the
// rest there would lose it for a client that renders deltas.

// maxRestoredAnswer is the same ceiling the request side uses: a bigger answer is
// not buffered, it is streamed through unchanged.
const maxRestoredAnswer = 8 << 20

// placeholderTail reports whether `value` can still turn into a placeholder:
// a `<` followed only by what a placeholder is made of.
//
// A placeholder is a type in capitals, an underscore, and a lower-case id —
// `<PERSON_NAMES_1a2b3c4d>`. The model splits it wherever it likes, including
// inside the id, so a lower-case letter counts once the underscore was seen;
// without that, ordinary text such as `<div` would be held.
func placeholderTail(value string) bool {
	if !strings.HasPrefix(value, "<") || len(value) > 40 {
		return false
	}
	separator := false
	for _, character := range value[1:] {
		switch {
		case character == '_':
			separator = true
		case character >= 'A' && character <= 'Z', character >= '0' && character <= '9':
		case separator && character >= 'a' && character <= 'z':
		default:
			return false
		}
	}
	return true
}

// restorer replaces placeholders in one answer.
type restorer struct {
	pairs []restorePair
	// carry holds the tail of a streamed value per block and kind, keyed
	// "<index>:<text|json>".
	carry map[string]string
}

func newRestorer(pairs []restorePair) *restorer {
	ordered := make([]restorePair, len(pairs))
	copy(ordered, pairs)
	// Longest first: a placeholder is a prefix of no other, but a value could
	// contain one by accident.
	sort.Slice(ordered, func(left, right int) bool {
		return len(ordered[left].Placeholder) > len(ordered[right].Placeholder)
	})
	return &restorer{pairs: ordered, carry: map[string]string{}}
}

// value returns the replacement for one chunk, or "" and false.
//
// `fragment` says the text is JSON inside JSON, which needs one more escape.
func (r *restorer) value(chunk string, fragment bool) (string, bool) {
	replaced := chunk
	found := false
	for _, pair := range r.pairs {
		if !strings.Contains(replaced, pair.Placeholder) {
			continue
		}
		replacement := pair.Value
		if fragment {
			replacement = escapeInner(replacement)
		}
		replaced = strings.ReplaceAll(replaced, pair.Placeholder, replacement)
		found = true
	}
	return replaced, found
}

// restoreBody rewrites one whole JSON answer in place.
func (r *restorer) restoreBody(raw string) string {
	out, _ := r.value(raw, true)
	return out
}

// restoreChunk handles one streamed piece, holding a tail that could still
// become a placeholder.
//
// When the whole piece turns out to be such a tail, nothing of it is written:
// the piece is held and comes out in front of the next piece of the same block.
// Writing it out as well would leave the client with the tail twice.
func (r *restorer) restoreChunk(key, chunk string, fragment bool) (string, bool) {
	combined := r.carry[key] + chunk
	delete(r.carry, key)
	held := false
	cut := strings.LastIndex(combined, "<")
	if cut >= 0 && placeholderTail(combined[cut:]) {
		r.carry[key] = combined[cut:]
		combined = combined[:cut]
		held = true
	}
	out, found := r.value(combined, fragment)
	if out == "" && !found {
		return "", held
	}
	return out, true
}

// flush returns a held tail when a block ends.
func (r *restorer) flush(key string) (string, bool) {
	tail, held := r.carry[key]
	delete(r.carry, key)
	return tail, held
}

// escapeInner escapes a value for use inside a JSON string literal.
//
// The escaping has to be the one JSON defines, not the one Go's quoting uses:
// `strconv.Quote` turns a bell into `\a`, which is not a JSON escape, and it
// leaves an emoji as it is. Encoding a string with a JSON encoder and stripping
// the surrounding quotes gives exactly the bytes a JSON string literal may hold.
func escapeInner(value string) string {
	var buf bytes.Buffer
	encoder := json.NewEncoder(&buf)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(value); err != nil {
		return value
	}
	out := strings.TrimSuffix(buf.String(), "\n")
	if len(out) < 2 {
		return value
	}
	return out[1 : len(out)-1]
}

// encodeEvent renders one event as the payload of a `data:` line.
//
// HTML escaping is off on purpose: a placeholder the model left as it is should
// reach the agent byte for byte, not as `\u003cPERSON...`.
func encodeEvent(event map[string]any) (string, error) {
	var buf bytes.Buffer
	encoder := json.NewEncoder(&buf)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(event); err != nil {
		return "", err
	}
	return strings.TrimSuffix(buf.String(), "\n"), nil
}

// restoreResponse rewrites a response before it reaches the sandbox.
func restoreResponse(resp *http.Response, pairs []restorePair) error {
	if len(pairs) == 0 {
		return nil
	}
	restorer := newRestorer(pairs)
	contentType := strings.ToLower(resp.Header.Get("content-type"))

	if strings.Contains(contentType, "text/event-stream") {
		reader, writer := io.Pipe()
		original := resp.Body
		resp.Body = reader
		resp.ContentLength = -1
		resp.Header.Del("content-length")
		go func() {
			defer func() { _ = writer.Close() }()
			defer func() { _ = original.Close() }()
			_ = restorer.copyEvents(original, writer)
		}()
		return nil
	}

	if !strings.Contains(contentType, "json") {
		return nil
	}
	if resp.ContentLength > maxRestoredAnswer {
		return nil
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, maxRestoredAnswer+1))
	if err != nil {
		return err
	}
	_ = resp.Body.Close()
	if len(body) > maxRestoredAnswer {
		resp.Body = io.NopCloser(bytes.NewReader(body))
		return nil
	}
	restored := restorer.restoreBody(string(body))
	resp.Body = io.NopCloser(strings.NewReader(restored))
	resp.ContentLength = int64(len(restored))
	resp.Header.Set("content-length", strconv.Itoa(len(restored)))
	return nil
}

// copyEvents reads server-sent events one line at a time and writes them on
// immediately, so the first token still arrives first.
//
// One input line can become more than one output line: a block that ends with a
// held tail writes that tail as an event of its own first.
func (r *restorer) copyEvents(source io.Reader, sink io.Writer) error {
	scanner := bufio.NewScanner(source)
	scanner.Buffer(make([]byte, 0, 64*1024), 4<<20)
	for scanner.Scan() {
		for _, out := range r.restoreLine(scanner.Text()) {
			if _, err := io.WriteString(sink, out+"\n"); err != nil {
				return err
			}
		}
	}
	return scanner.Err()
}

// restoreLine rewrites one SSE line and returns the lines to write instead.
//
// Usually that is the line itself, recognised or not. A block that ends while a
// tail is held produces more: the tail as a delta of its own and the blank line
// that closes that event, in front of the unchanged line. A stop event carries
// no text, so the rest must not be written into it.
func (r *restorer) restoreLine(line string) []string {
	unchanged := []string{line}
	if !strings.HasPrefix(line, "data:") {
		return unchanged
	}
	payload := strings.TrimSpace(strings.TrimPrefix(line, "data:"))
	if payload == "" || payload == "[DONE]" {
		return unchanged
	}
	var event map[string]any
	if err := json.Unmarshal([]byte(payload), &event); err != nil {
		return unchanged
	}
	changed := false
	// `before` collects the events that have to precede this line.
	var before []string

	index := 0
	if raw, ok := event["index"].(float64); ok {
		index = int(raw)
	}

	if delta, ok := event["delta"].(map[string]any); ok {
		// `text` is the answer as a person reads it: a placeholder can be split
		// between deltas, so the tail is held and prefixed to the next one.
		if text, ok := delta["text"].(string); ok {
			if out, touched := r.restoreChunk(streamKey(index, "text"), text, false); touched {
				delta["text"] = out
				changed = true
			}
		}
		// `partial_json` is a JSON fragment that the client assembles, so holding
		// a tail over is safe there too: it is one more piece of the document.
		if fragment, ok := delta["partial_json"].(string); ok {
			if out, touched := r.restoreChunk(streamKey(index, "json"), fragment, true); touched {
				delta["partial_json"] = out
				changed = true
			}
		}
	}

	if kind, ok := event["type"].(string); ok {
		switch kind {
		case "content_block_stop":
			// The block ends before the placeholder was completed, so the held
			// tail is text as it stands and belongs to the client now.
			before = append(before, r.flushBlockDeltas(index)...)
		case "response.output_text.delta":
			// OpenAI Responses streams the answer as a plain `delta` string.
			if text, ok := event["delta"].(string); ok {
				if out, touched := r.restoreChunk("r:text", text, false); touched {
					event["delta"] = out
					changed = true
				}
			}
		case "response.output_text.done":
			if tail, held := r.flush("r:text"); held {
				restored, _ := r.value(tail, false)
				injected := make(map[string]any, len(event))
				for key, value := range event {
					injected[key] = value
				}
				injected["type"] = "response.output_text.delta"
				injected["delta"] = restored
				if encoded, err := encodeEvent(injected); err == nil {
					before = append(before, "data: "+encoded, "")
				}
			}
		}
	}

	// OpenAI Chat Completions streams the answer in `choices[i].delta.content`
	// and the tool arguments in `choices[i].delta.tool_calls[j]`.
	if choices, ok := event["choices"].([]any); ok {
		for position, raw := range choices {
			choice, ok := raw.(map[string]any)
			if !ok {
				continue
			}
			choiceIndex := position
			if rawIndex, ok := choice["index"].(float64); ok {
				choiceIndex = int(rawIndex)
			}
			if delta, ok := choice["delta"].(map[string]any); ok {
				if text, ok := delta["content"].(string); ok {
					if out, touched := r.restoreChunk(choiceKey(choiceIndex, "text"), text, false); touched {
						delta["content"] = out
						changed = true
					}
				}
				if calls, ok := delta["tool_calls"].([]any); ok {
					for position, raw := range calls {
						call, ok := raw.(map[string]any)
						if !ok {
							continue
						}
						toolIndex := position
						if rawIndex, ok := call["index"].(float64); ok {
							toolIndex = int(rawIndex)
						}
						function, ok := call["function"].(map[string]any)
						if !ok {
							continue
						}
						arguments, ok := function["arguments"].(string)
						if !ok {
							continue
						}
						if out, touched := r.restoreChunk(choiceKey(choiceIndex, "tool"+strconv.Itoa(toolIndex)), arguments, true); touched {
							function["arguments"] = out
							changed = true
						}
					}
				}
			}
			// A choice with a finish reason sends no further deltas, so its held
			// tails go out first, in chunks of their own.
			if finish, present := choice["finish_reason"]; present && finish != nil {
				before = append(before, r.flushChoice(event, choiceIndex)...)
			}
		}
	}

	lines := before
	if !changed {
		return append(lines, line)
	}
	encoded, err := encodeEvent(event)
	if err != nil {
		return append(lines, line)
	}
	return append(lines, "data: "+encoded)
}

// flushBlockDeltas turns the tails held for one content block into delta events
// of its own, each with the blank line that closes an SSE event.
func (r *restorer) flushBlockDeltas(index int) []string {
	kinds := []struct {
		kind      string
		eventType string
		field     string
		fragment  bool
	}{
		{kind: "text", eventType: "text_delta", field: "text"},
		{kind: "json", eventType: "input_json_delta", field: "partial_json", fragment: true},
	}
	var lines []string
	for _, spec := range kinds {
		tail, held := r.flush(streamKey(index, spec.kind))
		if !held {
			continue
		}
		restored, _ := r.value(tail, spec.fragment)
		injected := map[string]any{
			"type":  "content_block_delta",
			"index": index,
			"delta": map[string]any{"type": spec.eventType, spec.field: restored},
		}
		encoded, err := encodeEvent(injected)
		if err != nil {
			continue
		}
		lines = append(lines, "data: "+encoded, "")
	}
	return lines
}

// flushChoice turns the tails held for one OpenAI choice into chunks of its own.
func (r *restorer) flushChoice(event map[string]any, choice int) []string {
	var lines []string
	if tail, held := r.flush(choiceKey(choice, "text")); held {
		restored, _ := r.value(tail, false)
		lines = append(lines, openAIChunkLines(event, choice, map[string]any{"content": restored})...)
	}
	for _, tool := range r.heldTools(choice) {
		tail, held := r.flush(choiceKey(choice, "tool"+strconv.Itoa(tool)))
		if !held {
			continue
		}
		restored, _ := r.value(tail, true)
		delta := map[string]any{
			"tool_calls": []any{
				map[string]any{
					"index":    tool,
					"function": map[string]any{"arguments": restored},
				},
			},
		}
		lines = append(lines, openAIChunkLines(event, choice, delta)...)
	}
	return lines
}

// heldTools lists the tool indices of one choice that hold a tail, in order.
func (r *restorer) heldTools(choice int) []int {
	prefix := choiceKey(choice, "tool")
	var tools []int
	for key := range r.carry {
		if !strings.HasPrefix(key, prefix) {
			continue
		}
		if tool, err := strconv.Atoi(strings.TrimPrefix(key, prefix)); err == nil {
			tools = append(tools, tool)
		}
	}
	sort.Ints(tools)
	return tools
}

// openAIChunkLines builds one chat chunk with the envelope of `event` and only
// `delta` set in its choice, each with the blank line that closes an SSE event.
func openAIChunkLines(event map[string]any, choice int, delta map[string]any) []string {
	chunk := make(map[string]any, len(event))
	for key, value := range event {
		if key == "choices" {
			continue
		}
		chunk[key] = value
	}
	chunk["choices"] = []any{
		map[string]any{"index": choice, "delta": delta, "finish_reason": nil},
	}
	encoded, err := encodeEvent(chunk)
	if err != nil {
		return nil
	}
	return []string{"data: " + encoded, ""}
}

// choiceKey names the held tail of one OpenAI choice.
func choiceKey(choice int, kind string) string {
	return "c" + strconv.Itoa(choice) + ":" + kind
}

func streamKey(index int, kind string) string {
	return strconv.Itoa(index) + ":" + kind
}

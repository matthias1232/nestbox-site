package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/http"
	"os"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"
)

// The content check on the decrypted hop.
//
// The proxy decides *whether* a request may leave; this decides *what* may
// leave. It sits here and not in Envoy's Lua filter because rewriting a body is
// a first-class operation on an `io.Reader` and a guess in a filter without a
// JSON library: a filter that edits a payload it only pattern-matched would be a
// filter that corrupts requests and cannot say why.
//
// The rule is the control API's, not this file's: this hop sends the body and the
// address of the request and does what the answer says — refuse, forward the
// rewritten text, or forward unchanged. `observe` is the one mode that may carry
// on when the control API cannot answer; everywhere else an unanswered check is
// a refusal, because a filter that fails open is a filter an outage turns off.
//
// Only requests are inspected. An answer that streams back token by token
// cannot be rewritten without breaking the stream, so the outbound direction is
// recorded by the accounting proxy when the agent routes through it, and
// otherwise not at all. The console says that plainly instead of implying
// otherwise.

// maxInspectedBody is what the proxy reads and what the control API accepts.
// A larger body is not truncated: half a JSON document is not a smaller document,
// and a truncated one would be inspected as text and rewritten in the wrong
// places.
const maxInspectedBody = 8 << 20

// dlpTimeout bounds the check. A model request can be megabytes, and the scan is
// a walk over it; 1.5 s was measured against a 120-byte test body, not against
// what an agent actually sends.
const dlpTimeout = 5 * time.Second

// The three facts worth extracting from a body, compiled once. `MustCompile`
// per request was the only work in this file that grew with traffic.
var (
	modelPattern        = regexp.MustCompile(`"model"\s*:\s*"([^"]+)"`)
	conversationPattern = regexp.MustCompile(`session_id\\?"\s*:\s*\\?"([0-9a-fA-F-]{8,})`)
	toolPattern         = regexp.MustCompile(`"name"\s*:\s*"([^"]+)"`)
)

// dlpClient asks the control API about one body.
type dlpClient struct {
	url    string
	nodeID string
	token  string
	client *http.Client
	// The last mode the control API answered with, per sandbox. It is only used
	// when the check cannot answer: `observe` may let a body through, and a mode
	// nobody has seen may not.
	modes sync.Map
	// The last enforcement the control API reported, per sandbox and
	// destination. A destination that is only recorded cannot be refused by an
	// outage either: the policy never asked this hop to stop it.
	hosts sync.Map
}

// newDlpClientFromEnv builds the client from the proxy's own identity.
//
// The same node id and token the Lua filter uses for its tunnel check: the
// control API already trusts them, and a second credential would be a second
// thing to rotate.
func newDlpClientFromEnv() *dlpClient {
	nodeID := strings.TrimSpace(os.Getenv("NESTBOX_NODE_ID"))
	token := strings.TrimSpace(os.Getenv("NESTBOX_NODE_TOKEN"))
	if token == "" {
		if path := strings.TrimSpace(os.Getenv("NESTBOX_NODE_TOKEN_FILE")); path != "" {
			if raw, err := os.ReadFile(path); err == nil {
				token = strings.TrimSpace(string(raw))
			}
		}
	}
	url := strings.TrimSpace(os.Getenv("NESTBOX_CONTROL_URL"))
	if url == "" {
		url = "http://control-api:8000"
	}
	if nodeID == "" || token == "" {
		log.Printf("egress-dlp: no node identity; bodies are forwarded without a content check")
		return nil
	}
	return &dlpClient{
		url:    strings.TrimRight(url, "/"),
		nodeID: nodeID,
		token:  token,
		client: &http.Client{Timeout: dlpTimeout},
	}
}

// dlpRequest is one body and the request it belongs to.
type dlpRequest struct {
	SandboxID      string `json:"sandbox_id"`
	Content        string `json:"content"`
	Destination    string `json:"destination,omitempty"`
	Source         string `json:"source"`
	Method         string `json:"method,omitempty"`
	Path           string `json:"path,omitempty"`
	Model          string `json:"model,omitempty"`
	McpTool        string `json:"mcp_tool,omitempty"`
	ConversationID string `json:"conversation_id,omitempty"`
	// The id the caller gave this request, so the console can tie the body
	// check, the live stream and the refusal report to one row.
	RequestRef string `json:"request_ref,omitempty"`
}

// dlpFinding is what the console lists: kind, masked excerpt, what happened.
type dlpFinding struct {
	Detector    string `json:"detector"`
	MaskedValue string `json:"masked_value"`
	Action      string `json:"action"`
	Requested   string `json:"requested"`
}

// dlpVerdict is the control API's answer.
type dlpVerdict struct {
	Permitted          bool         `json:"permitted"`
	Action             string       `json:"action"`
	TransformedContent string       `json:"transformed_content"`
	Findings           []dlpFinding `json:"findings"`
	// The policy mode that answered, so this hop can fall back to the same
	// decision when it cannot ask.
	Mode string `json:"mode"`
	// What the policy did with this destination and this body:
	// `record_only` and `exempt` mean nothing was to be stopped here, so an
	// outage may not start stopping it.
	Enforcement string `json:"enforcement"`
	// Placeholders to turn back into their values on the way out, for
	// `pseudonymize`. Only the answer of this request carries them.
	Restore []restorePair `json:"restore"`
}

// restorePair is one placeholder and the value it stands for.
type restorePair struct {
	Placeholder string `json:"placeholder"`
	Value       string `json:"value"`
}

// inspect sends one body and returns the verdict.
func (c *dlpClient) inspect(ctx context.Context, body dlpRequest) (dlpVerdict, error) {
	payload, err := json.Marshal(body)
	if err != nil {
		return dlpVerdict{}, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.url+"/api/v1/internal/privacy/inspect", bytes.NewReader(payload))
	if err != nil {
		return dlpVerdict{}, err
	}
	req.Header.Set("content-type", "application/json")
	req.Header.Set("x-nestbox-node", c.nodeID)
	req.Header.Set("authorization", "Bearer "+c.token)
	resp, err := c.client.Do(req)
	if err != nil {
		return dlpVerdict{}, err
	}
	defer func() { _ = resp.Body.Close() }()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return dlpVerdict{}, err
	}
	if resp.StatusCode != http.StatusOK {
		return dlpVerdict{}, errors.New("inspect answered " + resp.Status)
	}
	var verdict dlpVerdict
	if err := json.Unmarshal(raw, &verdict); err != nil {
		return dlpVerdict{}, err
	}
	return verdict, nil
}

// apply checks one request and either lets it through (possibly rewritten) or
// writes the refusal itself and reports handled.
//
// A request this hop cannot check — no sandbox, a body it must not read, a
// non-JSON content type — is forwarded, because the policy decision that let it
// through has already been made by Envoy. What it must never do is forward a
// body it read and then failed to check while the policy forbids that; that case
// is the 503 below.
func (c *dlpClient) apply(w http.ResponseWriter, r *http.Request) (handled bool) {
	if r.Body == nil {
		return false
	}
	switch r.Method {
	case http.MethodPost, http.MethodPut, http.MethodPatch:
	default:
		return false
	}
	if !strings.Contains(strings.ToLower(r.Header.Get("content-type")), "json") {
		return false
	}
	sandboxID := strings.TrimSpace(r.Header.Get("x-nestbox-sandbox"))
	if sandboxID == "" {
		// Envoy copies the id out of the policy answer. An empty one means the
		// request was not identified, and a body nobody can be blamed for is not
		// a body this hop may refuse.
		log.Printf("egress-dlp: %s %s: no sandbox in the request; body not checked", r.Method, r.Host)
		return false
	}
	// Everything the check, the refusal report and the console need about this
	// request is gathered once, before the body is touched: a body that is
	// refused for its size never gets a Content field.
	body := dlpRequest{
		SandboxID:   sandboxID,
		Destination: r.Host,
		Source:      "egress",
		Method:      r.Method,
		Path:        r.URL.Path,
		RequestRef:  strings.TrimSpace(r.Header.Get("x-request-id")),
	}
	if r.ContentLength > maxInspectedBody {
		return c.refuseOversize(w, r, body)
	}
	original, err := io.ReadAll(io.LimitReader(r.Body, maxInspectedBody+1))
	if err != nil {
		return c.refuseUnavailable(w, r, body, err)
	}
	_ = r.Body.Close()
	if len(original) == 0 {
		r.Body = io.NopCloser(bytes.NewReader(original))
		return false
	}
	if len(original) > maxInspectedBody {
		return c.refuseOversize(w, r, body)
	}
	r.Body = io.NopCloser(bytes.NewReader(original))

	body.Content = string(original)
	text := string(original)
	body.Model = jsonField(text, modelPattern)
	body.ConversationID = jsonField(text, conversationPattern)
	if strings.Contains(text, `"jsonrpc"`) {
		body.Source = "mcp"
		body.McpTool = jsonField(text, toolPattern)
	}

	ctx, cancel := context.WithTimeout(r.Context(), dlpTimeout)
	defer cancel()
	verdict, err := c.inspect(ctx, body)
	if err == nil {
		c.modes.Store(sandboxID, verdict.Mode)
		c.hosts.Store(hostKey(sandboxID, r.Host), verdict.Enforcement)
		if len(verdict.Findings) > 0 || !verdict.Permitted {
			log.Printf("egress-dlp: %s %s: %s (%d findings)", r.Method, r.Host, verdict.Action, len(verdict.Findings))
		}
	}
	if err != nil {
		return c.refuseUnavailable(w, r, body, err)
	}
	if !verdict.Permitted {
		w.Header().Set(dlpHeader, "blocked")
		markWould(w, r)
		writeDlpRefusal(w, http.StatusForbidden, "privacy.blocked",
			"The content policy refused this request.")
		return true
	}
	if verdict.Action == "replaced" && verdict.TransformedContent != "" && verdict.TransformedContent != string(original) {
		replaced := []byte(verdict.TransformedContent)
		r.Body = io.NopCloser(bytes.NewReader(replaced))
		r.ContentLength = int64(len(replaced))
		r.Header.Set("content-length", strconv.Itoa(len(replaced)))
		w.Header().Set(dlpHeader, "replaced")
	}
	if len(verdict.Restore) > 0 {
		// The answer carries placeholders that restore.go turns back into their
		// values, and that walk needs a readable body. A compressed answer is
		// opaque bytes there, so the placeholder would reach the sandbox as
		// `<IBAN_...>`. Negotiation happens on the request, so this is the one
		// place the compression can be turned off.
		r.Header.Del("Accept-Encoding")
		*r = *r.WithContext(context.WithValue(r.Context(), restoreKey{}, verdict.Restore))
		w.Header().Set(dlpHeader, "pseudonymized")
	}
	return false
}

// mayFailOpen reports whether an unanswered check may be forwarded.
//
// It follows the *policy* the control API last reported, not the egress mode:
// comparing the egress mode made this branch dead code, because a transparent
// sandbox is never told "observe" (finding A3).
//
// Two answers count. `observe` and `off` say no decision was ever enforced, so
// an outage changes nothing. A destination the policy last reported as
// `record_only` or `exempt` is the narrower case of the same fact, and it is
// keyed by sandbox and destination: an outage of the control API must not start
// refusing traffic that was only ever being watched, but it must also not let a
// destination that *was* being enforced through (finding R8).
func (c *dlpClient) mayFailOpen(sandboxID, host string) bool {
	if mode, seen := c.modes.Load(sandboxID); seen {
		value, _ := mode.(string)
		if value == "observe" || value == "off" {
			return true
		}
	}
	if enforcement, seen := c.hosts.Load(hostKey(sandboxID, host)); seen {
		value, _ := enforcement.(string)
		if value == "record_only" || value == "exempt" {
			return true
		}
	}
	return false
}

// hostKey is how the last enforcement of one destination is kept.
func hostKey(sandboxID, host string) string {
	return sandboxID + "|" + hostOnly(host)
}

// markWould writes the address a refused request would have reached, before the
// refusal status is written.
//
// The header is what makes a refusal inspectable: the console can show that
// api.anthropic.com resolves to an address in the provider's range and not to
// something this node invented. An unresolved name writes nothing rather than
// a placeholder that looks like an address.
func markWould(w http.ResponseWriter, r *http.Request) {
	if ip := lookupWould(r.Host); ip != "" {
		w.Header().Set(upstreamWouldHeader, ip)
	}
}

// refuseUnavailable answers a request whose check could not be answered.
func (c *dlpClient) refuseUnavailable(w http.ResponseWriter, r *http.Request, body dlpRequest, cause error) bool {
	if c.mayFailOpen(body.SandboxID, r.Host) {
		log.Printf("egress-dlp: %s: %v; the policy records this destination, forwarding", r.Host, cause)
		return false
	}
	log.Printf("egress-dlp: %s: %v; refusing", r.Host, cause)
	w.Header().Set(dlpHeader, "unavailable")
	markWould(w, r)
	c.reportRefusal(body, refusalKindUnavailable)
	writeDlpRefusal(w, http.StatusServiceUnavailable, "privacy.unavailable",
		"The content check could not be answered, so the request was refused.")
	return true
}

// refuseOversize answers a body larger than the check reads.
//
// Padding a prompt past the limit would otherwise be a way around the policy,
// and a body that is never read cannot be touched up: half a JSON document is
// not a smaller document.
func (c *dlpClient) refuseOversize(w http.ResponseWriter, r *http.Request, body dlpRequest) bool {
	if c.mayFailOpen(body.SandboxID, r.Host) {
		log.Printf("egress-dlp: %s: body over %d bytes; the policy records this destination, forwarding", r.Host, maxInspectedBody)
		return false
	}
	log.Printf("egress-dlp: %s: body over %d bytes; refusing", r.Host, maxInspectedBody)
	w.Header().Set(dlpHeader, "too_large")
	markWould(w, r)
	c.reportRefusal(body, refusalKindTooLarge)
	writeDlpRefusal(w, http.StatusRequestEntityTooLarge, "privacy.too_large",
		"The request is larger than the content check reads (8 MiB) and the policy does not let unchecked content leave.")
	return true
}

// The two refusals this hop writes itself, as the control API names them.
const (
	refusalKindUnavailable = "unavailable"
	refusalKindTooLarge    = "too_large"
)

// reportRefusal tells the control API about a refusal that carries no finding.
//
// A blocked body is reported by the control API itself, because it is the one
// that matched something. A request refused for its size or for an unanswered
// check never reached it, so without this the console would show a gap where a
// refusal was. It runs in the background: the sandbox already has its answer,
// and a control API slow to record must not hold the refusal open. A failure is
// logged and dropped — the alternative, retrying, would turn one outage into a
// queue.
func (c *dlpClient) reportRefusal(body dlpRequest, kind string) {
	if c == nil || c.url == "" {
		return
	}
	report := struct {
		SandboxID   string `json:"sandbox_id"`
		Kind        string `json:"kind"`
		Destination string `json:"destination,omitempty"`
		Method      string `json:"method,omitempty"`
		Path        string `json:"path,omitempty"`
		RequestRef  string `json:"request_ref,omitempty"`
		Model       string `json:"model,omitempty"`
	}{
		SandboxID:   body.SandboxID,
		Kind:        kind,
		Destination: body.Destination,
		Method:      body.Method,
		Path:        body.Path,
		RequestRef:  body.RequestRef,
		Model:       body.Model,
	}
	payload, err := json.Marshal(report)
	if err != nil {
		log.Printf("egress-dlp: refusal report for %s could not be built: %v", body.Destination, err)
		return
	}
	url := c.url + "/api/v1/internal/privacy/refusal"
	nodeID, token, client := c.nodeID, c.token, c.client
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		defer cancel()
		req, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(payload))
		if err != nil {
			log.Printf("egress-dlp: refusal report for %s: %v", body.Destination, err)
			return
		}
		req.Header.Set("content-type", "application/json")
		req.Header.Set("x-nestbox-node", nodeID)
		req.Header.Set("authorization", "Bearer "+token)
		resp, err := client.Do(req)
		if err != nil {
			log.Printf("egress-dlp: refusal report for %s: %v", body.Destination, err)
			return
		}
		defer func() { _ = resp.Body.Close() }()
		_, _ = io.Copy(io.Discard, io.LimitReader(resp.Body, 4096))
		if resp.StatusCode >= 300 {
			log.Printf("egress-dlp: refusal report for %s: %s", body.Destination, resp.Status)
		}
	}()
}

// dlpHeader tells the sandbox — and, through Envoy's access log, the console —
// what happened to this request.
const dlpHeader = "x-nestbox-dlp"

// restoreKey carries the placeholders of one answer to the response side.
type restoreKey struct{}

// writeDlpRefusal answers the sandbox with a reason it can act on.
//
// No matched value is in the body; a detector name and a count are enough to
// say why, and enough is all that may be said.
func writeDlpRefusal(w http.ResponseWriter, status int, code, message string) {
	w.Header().Set("content-type", "application/json")
	w.WriteHeader(status)
	// The shape an agent's SDK already understands: `type` plus `error.type`
	// and `error.message`. A bare `{"error": {...}}` was shown as a parse error
	// rather than as the refusal it is.
	_ = json.NewEncoder(w).Encode(map[string]any{
		"type":  "error",
		"error": map[string]any{"type": "permission_error", "message": message + " (" + code + ")"},
	})
}

// jsonField returns the first match of a pattern, or "".
func jsonField(text string, pattern *regexp.Regexp) string {
	match := pattern.FindStringSubmatch(text)
	if len(match) < 2 {
		return ""
	}
	return match[1]
}

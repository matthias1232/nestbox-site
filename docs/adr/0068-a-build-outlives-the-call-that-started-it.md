# 0068 — A recipe build outlives the call that started it, and its Dockerfile carries the pack

## Context

A recipe image is not built by a person at a shell: the control API generates a
Dockerfile, hands it to the node's runner, and the runner builds it. Two bounds
grew wrong once the desktop pack became real (U09–U11):

* The runner refused a Dockerfile body over 64 KiB. The desktop pack embeds its
  start script, `nestbox-display` and `nestbox-desktop-mcp` in that Dockerfile as
  base64, because the build context is the string the API sends and nothing else
  — about 120 KB together. The build failed with `string_too_long` while the pack
  itself was fine.
* The control API's HTTP client gave every runner call 660 s. A desktop image on
  the appliance installs XFCE, Chrome, Node and the pack in about eleven minutes,
  so the API declared the build failed while Docker finished it. The next attempt
  found every layer cached and "succeeded" in a minute, which hides the bug from
  anyone who only retries.

The same step uncovered a wiring fault that no compiler can see: the decorator
`@app.post("/v1/sandboxes")` sat above `sandbox_runtime_of` instead of
`create_sandbox`, so every sandbox create answered 422 from a helper that takes
`container` as a parameter.

## Decision

* **The body cap is a guard against a runaway request, not a limit on a real
  image.** `MAX_DOCKERFILE_BYTES` is 1 MiB in the runner, with the reason next to
  it. A generated Dockerfile is never operator input, and its size grows with the
  scripts the pack ships.
* **A build gets its own timeout.** `HttpRunner` keeps `CLIENT_TIMEOUT` (660 s)
  for every ordinary call — an exec, a container start, a listing — and sends
  `POST /v1/builds` with `BUILD_TIMEOUT` (3600 s). The operator is answered when
  the job starts, so a long wait holds the job, not a request.
* **The route table is asserted.** `tests/test_routes.py` checks that the route
  for `POST /v1/sandboxes` is served by `create_sandbox`, that `POST /v1/builds`
  is served by `build_image`, and that `GET /healthz` is served by `healthz`. A
  decorator above the wrong function imports cleanly and is otherwise silent.
* **A recipe sandbox gets a new image by being instantiated again**, not by
  `POST /sandboxes/{id}/recreate`, which refuses recipe images (the entrypoint is
  generated at build time). Until the API can do that in place, the operator's
  path is: rebuild the recipe, delete the sandbox, instantiate it again under the
  same name. Its id changes; its persistent roots do not.

## Consequences

* A build that needs more than an hour is called failed, and its layers stay in
  Docker's cache, so the retry is cheap.
* Raising the caps cannot break a normal call: they only widen what is accepted.
* The route wiring cannot regress silently, and a create that answers 422 about a
  `container` parameter is now a test failure rather than a support call.
* Re-instantiation keeps the name and loses the sandbox id, which is why the
  console's sandbox links are by name in the places that matter.

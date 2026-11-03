# openchamber/ — the OpenChamber agent pack image

`entrypoint.sh` is the container command of the `openchamber` pack. It makes a UI
password (given or random, printed once to the sandbox log), then runs
`openchamber serve` in a restart loop: the process writes its PID to
`/tmp/nestbox-agents/openchamber.pid`, and a stopped server is started again
after 2 s. A sign-in mode change therefore restarts the process and not the
sandbox, and the sources `~/.nestbox/env.sh` on every start so the changed
environment takes effect.

`sh -n entrypoint.sh` must pass; the file is embedded in the generated
Dockerfile and hashed into the image tag, so editing it produces a new image.

-- 0089_desktop_control_mcp — the whole desktop as an MCP server, next to the browser.
--
-- `chrome-browser` (0060) controls exactly one process: Chrome. A desktop sandbox is more than its
-- browser — an agent that has to log into a native app, answer a dialog it caused, move a window
-- onto the second monitor or take a screenshot of the screen a person is watching has no tool for
-- any of it. `nestbox-desktop-mcp` runs inside the sandbox, talks to the same X display the console
-- shows over VNC, and drives it with xdotool, wmctrl, scrot and `nestbox-display` (U09).
--
-- stdio, like the browser server: it runs next to the screen it controls, so there is nothing to
-- expose on the network and nothing that could be reached from another sandbox.
--
-- `packs = {desktop}` is the binding: only a sandbox that runs the desktop pack is offered this
-- server, and the browser server above stays what it is — one application, not the screen.
INSERT INTO mcp_registry (key, name, description, publisher, homepage, transport, command, args, auth_type, packs) VALUES
    ('desktop-control', 'Desktop control',
     'See and operate the whole desktop: screenshots, clicks and typing on every monitor, windows, monitor layouts.',
     'Nestbox', NULL, 'stdio', '/usr/local/bin/nestbox-desktop-mcp', ARRAY[]::text[], 'none',
     ARRAY['desktop'])
ON CONFLICT (key) DO NOTHING;

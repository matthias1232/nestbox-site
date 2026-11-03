-- 0060_desktop_pack — a graphical desktop as an agent pack, and the browser MCP.
--
-- A sandbox with a desktop runs a VNC server and XFCE, and Chrome with remote
-- debugging on 127.0.0.1:9222. The console shows that desktop in a tab (noVNC);
-- an agent drives the same Chrome through the Chrome DevTools MCP server, so a
-- person can watch it, take over, and log in where the agent is asked to.
--
-- `desktop` is a new install kind because the install is a fixed script shipped
-- with the control plane (tool-images/desktop), not a URL an operator supplies.

ALTER TABLE agent_packs DROP CONSTRAINT agent_packs_install_kind_valid;
ALTER TABLE agent_packs ADD CONSTRAINT agent_packs_install_kind_valid CHECK (install_kind IN (
    'script', 'git', 'npm', 'pip', 'binary', 'image', 'none', 'desktop'
));

INSERT INTO agent_packs (key, name, vendor, description, base_os, install_kind, install_ref,
                         home_paths, requires_gpu, needs_inference, default_image,
                         default_command, ports, env_slots) VALUES
    ('desktop', 'Desktop (XFCE, Chrome, VNC)', 'Nestbox',
     'A graphical desktop you can open in the console, with Google Chrome that agents can drive through MCP.',
     ARRAY['ubuntu-24.04'], 'desktop', 'xfce-chrome',
     ARRAY['.config/nestbox-chrome', '.pki'], false, false,
     'nestbox-ubuntu-dev:local', ARRAY['/usr/local/bin/nestbox-desktop'], '{}', '{}')
ON CONFLICT (key) DO NOTHING;

-- The browser as an MCP server. stdio: it runs inside the sandbox next to the
-- Chrome it controls, so there is nothing to expose on the network.
INSERT INTO mcp_registry (key, name, description, publisher, homepage, transport, command, args, auth_type, packs) VALUES
    ('chrome-browser', 'Chrome browser',
     'Open pages, click, type, read the console and take screenshots in the sandbox''s own Chrome. The same browser is on the desktop tab, so a person can log in or take over.',
     'Google (Chrome DevTools)', 'https://github.com/ChromeDevTools/chrome-devtools-mcp', 'stdio',
     'chrome-devtools-mcp', ARRAY['--browserUrl=http://127.0.0.1:9222'], 'none', '{}')
ON CONFLICT (key) DO NOTHING;

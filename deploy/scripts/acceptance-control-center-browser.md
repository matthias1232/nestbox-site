# Control Center — browser acceptance checklist

Walk this with a signed-in admin on `http://192.168.55.50:8080`. Take one screenshot per item
and name it after the item number. Run `acceptance-control-center.py` first.

1. [ ] Sign in. The login page shows the illustration.
2. [ ] Home shows the stat tiles and the map.
3. [ ] The sidebar tree opens a node page; the page shows gauges.
4. [ ] `⌘K` / `Ctrl+K` finds a sandbox by name and opens it.
5. [ ] On a sandbox page, scroll down: the action bar stays sticky.
6. [ ] "Allow all 15 min" shows a countdown and the banner.
7. [ ] The workbox strip switches recording off and back to full.
8. [ ] In the sandbox shell run `curl https://example.org`: a blocked-connection alert pops up
       and can be allowed for 15 min.
9. [ ] Live logs pop out into their own window (`/pop/logs/...`).
10. [ ] The policy wizard previews the compiled YAML.
11. [ ] The mount editor flips a root between read-only and read-write and applies.
12. [ ] The bulk deploy wizard reaches the Review step for two nodes. **Do not deploy**: the box
        has 8 GB RAM.
13. [ ] A HelpTip opens the docs at its anchor.
14. [ ] The first-run tour runs (reset it by clearing `nestbox:ui`).
15. [ ] `?` shows the keyboard shortcuts sheet.
16. [ ] The login page (signed out) shows the illustration at `/illustrations/login.webp`.

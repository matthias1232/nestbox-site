"""Tests for the CA files a sandbox is given, without Docker and without FastAPI.

`app/ca_bundle.py` deliberately imports nothing but the standard library, so the
real module is tested here rather than a copy. What is checked is the decision:
which egress modes get a CA at all, and what the bundle file contains.
"""
from __future__ import annotations

import os
import tempfile
import unittest

from app import ca_bundle


class CaMountsTest(unittest.TestCase):
    def test_a_direct_sandbox_gets_no_ca(self):
        self.assertEqual(ca_bundle.ca_mounts("direct", "/x/ca.crt", "/x/bundle.crt"), [])

    def test_a_mode_that_uses_the_proxy_gets_the_ca_and_the_bundle(self):
        mounts = ca_bundle.ca_mounts("forward_proxy", "/x/ca.crt", "/x/bundle.crt")
        self.assertEqual(
            [mount["target"] for mount in mounts],
            [ca_bundle.EGRESS_CA_TARGET, ca_bundle.BUNDLE_TARGET],
        )
        self.assertEqual([mount["source"] for mount in mounts], ["/x/ca.crt", "/x/bundle.crt"])
        self.assertTrue(all(mount["read_only"] for mount in mounts))

    def test_the_proxy_off_mode_still_gets_the_ca(self):
        # `off_allow_all` tunnels instead of reading TLS, but it is still the
        # proxy in the path, so a tool that verifies its own hostname needs it.
        mounts = ca_bundle.ca_mounts("off_allow_all", "/x/ca.crt", None)
        self.assertEqual([mount["target"] for mount in mounts], [ca_bundle.EGRESS_CA_TARGET])

    def test_a_missing_ca_mounts_nothing(self):
        self.assertEqual(ca_bundle.ca_mounts("transparent", None, None), [])


class PublishBundleTest(unittest.TestCase):
    def test_the_bundle_holds_the_public_roots_and_the_ca(self):
        with tempfile.TemporaryDirectory() as tmp:
            roots = os.path.join(tmp, "roots.crt")
            public_ca = os.path.join(tmp, "ca.crt")
            with open(roots, "wb") as handle:
                handle.write(b"PUBLIC ROOTS\n")
            with open(public_ca, "wb") as handle:
                handle.write(b"EGRESS CA\n")
            path = ca_bundle.publish_ca_bundle(public_ca, roots)
            self.assertEqual(path, os.path.join(tmp, "bundle.crt"))
            with open(path, "rb") as handle:
                self.assertEqual(handle.read(), b"PUBLIC ROOTS\n\nEGRESS CA\n")
            self.assertEqual(os.stat(path).st_mode & 0o777, 0o644)
            self.assertEqual(sorted(os.listdir(tmp)), ["bundle.crt", "ca.crt", "roots.crt"])

    def test_no_ca_is_no_bundle(self):
        self.assertIsNone(ca_bundle.publish_ca_bundle(None))


if __name__ == "__main__":
    unittest.main()

import unittest

from app import registry_images as images


class ReferenceTests(unittest.TestCase):
    def test_a_tag_is_split_from_its_repository(self):
        self.assertEqual(images.split_reference("nestbox-claude-code:local"), ("nestbox-claude-code", "local"))
        self.assertEqual(images.split_reference("ubuntu"), ("ubuntu", "latest"))

    def test_a_registry_port_is_not_mistaken_for_a_tag(self):
        self.assertEqual(images.split_reference("10.77.0.1:5000/tool"), ("10.77.0.1:5000/tool", "latest"))
        self.assertEqual(images.split_reference("10.77.0.1:5000/tool:v2"), ("10.77.0.1:5000/tool", "v2"))

    def test_a_digest_reference_keeps_its_digest(self):
        repo, rest = images.split_reference("repo@sha256:abc")
        self.assertEqual((repo, rest), ("repo", "sha256:abc"))

    def test_without_a_registry_nothing_is_pulled(self):
        original = images.REGISTRY
        try:
            images.REGISTRY = ""
            self.assertIsNone(images.registry_reference("nestbox-ubuntu-dev:local"))
            images.REGISTRY = "10.77.0.1:5000"
            self.assertEqual(images.registry_reference("nestbox-ubuntu-dev:local"), "10.77.0.1:5000/nestbox-ubuntu-dev:local")
            self.assertEqual(images.registry_reference("10.77.0.1:5000/x:1"), "10.77.0.1:5000/x:1")
        finally:
            images.REGISTRY = original


if __name__ == "__main__":
    unittest.main()

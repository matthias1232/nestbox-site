"""The model publish port: which port a joined node hands to a model server.

A joined node reaches the control plane only through the overlay, so its model
servers have to be published there instead of answering by container name. The
port comes from a range compose publishes (`NESTBOX_MODEL_PUBLISH_RANGE`), and
picking it is a pure decision: a predicate in, a port out. That is what these
tests drive, without a Docker socket and without a real binding.

`app.main` imports FastAPI and calls `docker.from_env()` at module scope, so the
client is replaced first, exactly as `tests/test_terminal.py` replaces FastAPI.
The module under test is the real file.
"""
from __future__ import annotations

import os
import sys
import unittest
from unittest import mock

import docker

docker.from_env = lambda *args, **kwargs: None

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from app import main as runner  # noqa: E402


class FakeContainer:
    """The part of a Docker container object that publishing reads."""

    def __init__(self, ports: dict | None = None) -> None:
        self.attrs = {"NetworkSettings": {"Ports": ports or {}}}


class PickPortTests(unittest.TestCase):
    def test_the_first_free_port_in_the_range_is_taken(self):
        self.assertEqual(runner.pick_port("10.77.1.2", 22300, 22302, lambda bind, port: port == 22300), 22300)

    def test_a_busy_port_is_skipped(self):
        busy = {22300, 22301}
        self.assertEqual(
            runner.pick_port("10.77.1.2", 22300, 22305, lambda bind, port: port not in busy),
            22302,
        )

    def test_no_free_port_is_none_and_not_a_port_outside_the_range(self):
        self.assertIsNone(runner.pick_port("10.77.1.2", 22300, 22302, lambda bind, port: False))

    def test_the_bind_address_is_what_gets_asked(self):
        asked: list[tuple[str, int]] = []

        def is_free(bind: str, port: int) -> bool:
            asked.append((bind, port))
            return port == 22301

        runner.pick_port("10.77.1.2", 22300, 22302, is_free)
        self.assertEqual(asked, [("10.77.1.2", 22300), ("10.77.1.2", 22301)])

    def test_a_single_port_range_is_its_own_answer(self):
        self.assertEqual(runner.pick_port("10.77.1.2", 22300, 22300, lambda bind, port: True), 22300)


class PublishTargetTests(unittest.TestCase):
    def test_an_unset_range_means_nothing_is_published(self):
        with mock.patch.dict(os.environ, {"NESTBOX_FORWARD_BIND": "10.77.1.2"}, clear=False):
            os.environ.pop("NESTBOX_MODEL_PUBLISH_RANGE", None)
            self.assertIsNone(runner.publish_target())

    def test_an_unset_bind_means_nothing_is_published(self):
        with mock.patch.dict(os.environ, {"NESTBOX_MODEL_PUBLISH_RANGE": "22300-22399"}, clear=False):
            os.environ.pop("NESTBOX_FORWARD_BIND", None)
            self.assertIsNone(runner.publish_target())

    def test_both_variables_give_bind_and_range(self):
        env = {"NESTBOX_FORWARD_BIND": "10.77.1.2", "NESTBOX_MODEL_PUBLISH_RANGE": "22300-22399"}
        with mock.patch.dict(os.environ, env, clear=False):
            self.assertEqual(runner.publish_target(), ("10.77.1.2", 22300, 22399))

    def test_a_malformed_range_is_no_range(self):
        env = {"NESTBOX_FORWARD_BIND": "10.77.1.2", "NESTBOX_MODEL_PUBLISH_RANGE": "22300"}
        with mock.patch.dict(os.environ, env, clear=False):
            self.assertIsNone(runner.publish_target())

    def test_an_inverted_range_is_no_range(self):
        env = {"NESTBOX_FORWARD_BIND": "10.77.1.2", "NESTBOX_MODEL_PUBLISH_RANGE": "22399-22300"}
        with mock.patch.dict(os.environ, env, clear=False):
            self.assertIsNone(runner.publish_target())


class PublishedAddressTests(unittest.TestCase):
    def test_the_overlay_binding_is_read_back(self):
        item = FakeContainer(
            {"8080/tcp": [{"HostIp": "10.77.1.2", "HostPort": "22300"}]}
        )
        self.assertEqual(runner.published_address(item), "10.77.1.2:22300")

    def test_a_wildcard_binding_is_not_an_overlay_address(self):
        item = FakeContainer({"8080/tcp": [{"HostIp": "0.0.0.0", "HostPort": "22300"}]})
        self.assertIsNone(runner.published_address(item))

    def test_an_unpublished_container_has_no_address(self):
        self.assertIsNone(runner.published_address(FakeContainer()))

    def test_another_container_port_is_ignored(self):
        item = FakeContainer({"9000/tcp": [{"HostIp": "10.77.1.2", "HostPort": "9000"}]})
        self.assertIsNone(runner.published_address(item))


if __name__ == "__main__":
    unittest.main()

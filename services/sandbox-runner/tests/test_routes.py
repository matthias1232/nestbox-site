"""The route table points at the functions that implement it.

A decorator that ends up above the wrong function is invisible: the module
imports, FastAPI is content, and the only symptom is a request that answers 422
from a handler nobody meant to expose. So the wiring is asserted, not assumed.

`app.main` imports FastAPI and the Docker client at module scope; both are part
of the runner image, which is where these tests run.
"""
from __future__ import annotations

import unittest

from fastapi.routing import APIRoute

from app.main import app


def handler(method: str, path: str) -> str:
    """The name of the function FastAPI calls for one method and path."""
    for route in app.routes:
        if isinstance(route, APIRoute) and route.path == path and method in route.methods:
            return route.endpoint.__name__
    raise AssertionError(f"no {method} {path} route is registered")


class RouteWiringTests(unittest.TestCase):
    def test_creating_a_sandbox_is_served_by_the_create_handler(self):
        # It was served by the runtime helper instead in one release: the
        # decorator sat above the helper, and every create answered 422.
        self.assertEqual(handler("POST", "/v1/sandboxes"), "create_sandbox")

    def test_building_an_image_is_served_by_the_build_handler(self):
        self.assertEqual(handler("POST", "/v1/builds"), "build_image")

    def test_health_is_served_by_the_health_handler(self):
        self.assertEqual(handler("GET", "/healthz"), "healthz")


if __name__ == "__main__":
    unittest.main()

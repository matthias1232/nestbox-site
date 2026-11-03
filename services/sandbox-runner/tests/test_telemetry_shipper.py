"""The mapping from Envoy's access-log line to the control API's traffic entry."""
from __future__ import annotations

import json
import unittest

from app import telemetry_shipper as ship

SANDBOX = "f98aed0f-cc4d-44be-b877-4363808c8cac"


def line(**fields) -> str:
    base = {
        "ts": "2026-10-29T01:14:23.240Z",
        "host": "api.anthropic.com:443",
        "target_host": "api.anthropic.com",
        "target_port": "443",
        "method": "CONNECT",
        "connect": "true",
        "status": "200",
        "sandbox": SANDBOX,
        "decision": "allow",
        "reason": "rule",
        "rule_id": "-",
        "bytes_in": "10",
        "bytes_out": "20",
        "duration_ms": "12",
    }
    base.update(fields)
    return "2026-10-29T01:14:23.241Z " + json.dumps(base)


class ShipTest(unittest.TestCase):
    def test_a_batch_is_cut_by_size(self) -> None:
        big = {"host": "x", "request_body": "A" * 300_000}
        taken = ship.take_batch([dict(big) for _ in range(5)])
        self.assertEqual(len(taken), 2)
        self.assertEqual(len(ship.take_batch([{"host": "a"}] * 10)), 10)
        self.assertEqual(len(ship.take_batch([{"host": "x", "request_body": "A" * 2_000_000}])), 1)

    def test_a_refused_entry_is_dropped_and_the_rest_is_delivered(self) -> None:
        sent: list[list[str]] = []

        def fake(entries):
            hosts = [e["host"] for e in entries]
            if "bad" in hosts:
                return "refused"
            sent.append(hosts)
            return "ok"

        original = ship.post_batch
        ship.post_batch = fake
        try:
            batch = [{"host": name} for name in ("a", "b", "bad", "c", "d")]
            self.assertEqual(ship.ship(batch), 5)
        finally:
            ship.post_batch = original
        self.assertEqual(sorted(h for group in sent for h in group), ["a", "b", "c", "d"])

    def test_a_server_that_is_down_keeps_everything(self) -> None:
        original = ship.post_batch
        ship.post_batch = lambda entries: "retry"
        try:
            self.assertEqual(ship.ship([{"host": "a"}, {"host": "b"}]), 0)
        finally:
            ship.post_batch = original


class RecordTest(unittest.TestCase):
    def test_a_body_cut_by_docker_is_joined_again(self) -> None:
        # Docker splits a long line into pieces and stamps each piece.
        cut = line(request_body="QUJD@@")
        cut = cut.replace("@@", "2026-10-29T01:14:23.999Z REVG")
        record = ship.record_from_line(cut)
        self.assertEqual(record["request_body"], "QUJDREVG")
        self.assertEqual(record["ts"], "2026-10-29T01:14:23.240Z")

    def test_maps_the_fields_the_control_api_reads(self) -> None:
        record = ship.record_from_line(line())
        self.assertEqual(record["sandbox_id"], SANDBOX)
        self.assertEqual(record["host"], "api.anthropic.com")
        self.assertEqual(record["port"], 443)
        self.assertEqual(record["decision"], "allow")
        self.assertEqual(record["status"], 200)
        self.assertEqual(record["bytes_out"], 20)

    def test_envoys_dash_means_no_value_and_is_dropped(self) -> None:
        record = ship.record_from_line(line(rule_id="-", reason="-", path="-"))
        self.assertNotIn("rule_id", record)
        self.assertNotIn("reason", record)
        self.assertNotIn("path", record)

    def test_the_would_deny_verdict_is_carried_through(self) -> None:
        # The authz answer is "1" when the rules said deny even though a window
        # let the request through; a dash means the filter never ran.
        self.assertEqual(ship.record_from_line(line(would_deny="1"))["would_deny"], "1")
        self.assertEqual(ship.record_from_line(line(would_deny="0"))["would_deny"], "0")
        self.assertNotIn("would_deny", ship.record_from_line(line(would_deny="-")))
        self.assertNotIn("would_deny", ship.record_from_line(line()))

    def test_a_line_that_is_not_an_access_record_is_skipped(self) -> None:
        self.assertIsNone(ship.record_from_line("[2026-10-29 01:00:00][1][info][main] starting"))
        self.assertIsNone(ship.record_from_line("2026-10-29T01:00:00Z {not json"))
        self.assertIsNone(ship.record_from_line('2026-10-29T01:00:00Z {"unrelated": 1}'))

    def test_the_host_comes_from_the_authority_when_the_target_is_missing(self) -> None:
        record = ship.record_from_line(line(target_host="-", target_port="-", host="example.com:8443"))
        self.assertEqual((record["host"], record["port"]), ("example.com", 8443))

    def test_a_sandbox_that_is_not_a_uuid_is_not_invented(self) -> None:
        record = ship.record_from_line(line(sandbox="hermes-01"))
        self.assertNotIn("sandbox_id", record)

    def test_split_host(self) -> None:
        self.assertEqual(ship.split_host("a.b:80"), ("a.b", 80))
        self.assertEqual(ship.split_host("a.b"), ("a.b", None))
        self.assertEqual(ship.split_host("[2001:db8::1]:443"), ("2001:db8::1", 443))
        self.assertEqual(ship.split_host("-"), (None, None))


if __name__ == "__main__":
    unittest.main()

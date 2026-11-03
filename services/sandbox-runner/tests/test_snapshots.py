import unittest

from fastapi import HTTPException

from app import snapshots


class PathTests(unittest.TestCase):
    def test_a_path_outside_the_persistent_base_is_refused(self):
        for bad in ("/etc", "/", str(snapshots.BASE), str(snapshots.BASE) + "/../etc", "/var/lib/nestbox/persistent-evil/x"):
            with self.assertRaises(HTTPException, msg=bad):
                snapshots.inside_base(bad)

    def test_a_path_under_the_base_is_accepted(self):
        self.assertEqual(snapshots.inside_base(str(snapshots.BASE) + "/agent-home/x"), snapshots.BASE / "agent-home" / "x")


class SummaryTests(unittest.TestCase):
    def test_the_summary_line_is_found_among_progress_lines(self):
        out = '{"message_type":"status","percent_done":0.5}\n{"message_type":"summary","snapshot_id":"abcdef0123456789ff","data_added":10,"total_bytes_processed":99,"total_files_processed":3}\n'
        summary = snapshots.parse_summary(out)
        self.assertEqual(summary["snapshot_id"][:16], "abcdef0123456789")
        self.assertEqual(summary["data_added"], 10)

    def test_noise_without_a_summary_is_an_empty_result(self):
        self.assertEqual(snapshots.parse_summary("garbage\n{not json\n"), {})


class PruneTests(unittest.TestCase):
    def test_a_repository_name_cannot_be_a_path(self):
        from pydantic import ValidationError
        for bad in ("../etc", "a/b", "", "x" * 81):
            with self.assertRaises(ValidationError, msg=bad):
                snapshots.PruneRequest(name=bad, password="longenough")

    def test_a_plain_name_is_accepted(self):
        self.assertEqual(snapshots.PruneRequest(name="00000000-0000-0000-0000-000000000002", password="longenough").keep_last, 20)


if __name__ == "__main__":
    unittest.main()

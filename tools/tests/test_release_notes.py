import importlib.util
import unittest
from pathlib import Path
from unittest import mock

SPEC = importlib.util.spec_from_file_location("release_notes", Path(__file__).resolve().parents[1] / "release_notes.py")
notes = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(notes)


def pull(number, title, *labels):
    return {"number": number, "title": title, "labels": [{"name": label} for label in labels]}


class ReleaseNotes(unittest.TestCase):
    def test_features_come_first_and_skipped_pull_requests_are_left_out(self):
        title, body = notes.notes("v1.2.0", [
            pull(7, "Fix the update check offline"),
            pull(8, "Instances keep their own build", "feature"),
            pull(9, "CI: daily ignored tests", "skip-release"),
        ])
        self.assertEqual(title, "Sunrise Launcher 1.2.0")
        self.assertLess(body.index("Instances keep their own build (#8)"), body.index("Fix the update check offline (#7)"))
        self.assertNotIn("#9", body)

    def test_a_release_with_nothing_to_tell_says_so(self):
        self.assertIn("Maintenance release.", notes.notes("v1.2.1", [pull(1, "CI", "skip-release")])[1])

    def test_the_tag_is_cargo_tomls_version_and_never_released_twice(self):
        head = "a" * 40
        with mock.patch.object(notes, "version", return_value="1.3.0"), mock.patch.object(notes, "tags", return_value=["v1.2.0"]):
            self.assertEqual(notes.decide(head), {"version": "1.3.0", "tag": "v1.3.0", "head": head})
        with mock.patch.object(notes, "version", return_value="1.2.0"), mock.patch.object(notes, "tags", return_value=["v1.2.0"]):
            with self.assertRaises(SystemExit):
                notes.decide(head)

    def test_only_a_stable_version_newer_than_the_game_repositorys_last_launcher_is_released(self):
        head = "a" * 40
        # launcher@1.0.0 and launcher@1.0.1 went out from the game repository: installed copies would never take either again.
        for refused in ["1.0.0", "1.0.1", "0.9.9", "1.1.0-rc1", "1.1.0+build", "01.1.0", "1.1", "1.1.0.0", "1.1.0\n"]:
            with mock.patch.object(notes, "version", return_value=refused), mock.patch.object(notes, "tags", return_value=[]):
                with self.assertRaises(SystemExit, msg=refused):
                    notes.decide(head)
        with mock.patch.object(notes, "version", return_value="1.0.2"), mock.patch.object(notes, "tags", return_value=[]):
            self.assertEqual(notes.decide(head)["tag"], "v1.0.2")
            with self.assertRaises(SystemExit):
                notes.decide("main")

    def test_unpublished_tags_are_skipped_and_noted(self):
        previous, skipped = notes.previous_release(
            ["v1.3.9", "v1.3.8", "v1.3.7", "v1.3.6"],
            "v1.3.9",
            {"v1.3.7", "v1.3.5"},
        )
        self.assertEqual(previous, "v1.3.7")
        self.assertEqual(skipped, ["v1.3.8"])
        title, body = notes.notes("v1.3.9", [pull(647, "Remove automatic cache cleanup")], skipped)
        self.assertEqual(title, "Sunrise Launcher 1.3.9")
        self.assertTrue(body.startswith("1.3.8 was not released.\n\n### Fixed and changed\n"))
        self.assertIn("Remove automatic cache cleanup (#647)", body)

    def test_release_branch_notes_include_reachable_main_prs_without_duplicates(self):
        import json

        head = "f" * 40
        release_pr = {
            **pull(701, "Hotfix on release"),
            "mergeCommit": {"oid": "1" * 40},
        }
        main_carried = {
            **pull(702, "Feature from main", "feature"),
            "mergeCommit": {"oid": "2" * 40},
        }
        main_unreleased = {
            **pull(703, "Not yet on release"),
            "mergeCommit": {"oid": "3" * 40},
        }
        queries = []

        def fake_run(cmd, **kwargs):
            if cmd[:4] == ["git", "-C", str(notes.ROOT), "log"]:
                return mock.Mock(stdout="2026-11-28T00:00:00Z\n")
            if cmd[:3] == ["gh", "pr", "list"]:
                search = cmd[cmd.index("--search") + 1]
                queries.append(search)
                if "base:release" in search:
                    return mock.Mock(stdout=json.dumps([release_pr]))
                if "base:main" in search:
                    return mock.Mock(stdout=json.dumps([release_pr, main_carried, main_unreleased]))
            if cmd[:4] == ["git", "-C", str(notes.ROOT), "rev-list"]:
                return mock.Mock(stdout=f"{'1' * 40}\n{'2' * 40}\n")
            raise AssertionError(f"unexpected command: {cmd}")

        with mock.patch.object(notes.subprocess, "run", side_effect=fake_run):
            got = notes.merged_since("v1.3.11", head, base="release")
        self.assertEqual(len(queries), 2)
        self.assertTrue(any("base:release" in q for q in queries))
        self.assertTrue(any("base:main" in q for q in queries))
        self.assertEqual([p["number"] for p in got], [701, 702])


if __name__ == "__main__":
    unittest.main()

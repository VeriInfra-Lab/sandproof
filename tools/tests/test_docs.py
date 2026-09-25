from pathlib import Path
import tempfile
import unittest

from tools.check_docs import targets, validate


class DocumentationTests(unittest.TestCase):
    def test_local_paths_and_titles(self):
        self.assertEqual(list(targets('[a](a.md) [b](b.md#part) [c](c.md "title")')), ["a.md", "b.md", "c.md"])

    def test_external_and_anchor_links_are_not_files(self):
        self.assertEqual(list(targets('[a](https://example.com/a) [b](#part) [c](mailto:a@b.c) [d](//example.com/a)')), [])

    def test_fenced_examples_are_ignored(self):
        self.assertEqual(list(targets('```md\n[a](missing.md)\n```\n[a](ok.md)')), ["ok.md"])
        self.assertEqual(list(targets('~~~md\n[a](missing.md)\n~~~')), [])

    def test_spaces_images_and_queries(self):
        self.assertEqual(list(targets('[a](<a b.md>) ![b](a%20b.png) [c](x.md?plain=1)')), ["a b.md", "a b.png", "x.md"])

    def setup_documents(self, root, index='[topic](topic.md)', topic='[back](README.md)'):
        (root / "docs").mkdir()
        (root / "docs/README.md").write_text(index, encoding="utf-8")
        (root / "docs/topic.md").write_text(topic, encoding="utf-8")
        return ["docs/README.md", "docs/topic.md"]

    def test_valid_index_and_parent_links(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.assertEqual(validate(root, self.setup_documents(root)), [])

    def test_missing_file_is_reported(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            errors = validate(root, self.setup_documents(root, topic='[bad](gone.md)'))
            self.assertTrue(any("missing link target" in error for error in errors))

    def test_orphan_topic_is_reported(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            errors = validate(root, self.setup_documents(root, index="No topics"))
            self.assertTrue(any("not listed" in error for error in errors))

    def test_repository_escape_is_reported(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            errors = validate(root, self.setup_documents(root, topic='[bad](../../outside.md)'))
            self.assertTrue(any("escapes repository" in error for error in errors))

    def test_missing_index_is_reported(self):
        with tempfile.TemporaryDirectory() as temp:
            self.assertTrue(validate(Path(temp), []))


if __name__ == "__main__":
    unittest.main()

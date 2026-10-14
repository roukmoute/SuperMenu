from __future__ import annotations

import json
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
from build_site import DOCUMENTS, Site, heading_slug
from check_site import Page, validate


class DocumentationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workspace = tempfile.TemporaryDirectory()
        cls.output = Path(cls.workspace.name) / "site"
        cls.site = Site()
        cls.manifest = cls.site.build(cls.output)

    @classmethod
    def tearDownClass(cls):
        cls.workspace.cleanup()

    def mutated_output(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        target = Path(directory.name) / "site"
        shutil.copytree(self.output, target)
        return target

    def test_every_document_is_published_in_both_languages(self):
        result = validate(self.output)
        self.assertEqual(result["documentationPages"], 12)
        self.assertEqual(result["htmlPages"], 13)
        self.assertEqual(result["languages"], ["en", "fr"])
        for metadata in self.manifest["pages"].values():
            self.assertEqual(metadata["anchors"], self.site.anchor_maps[metadata["document"], metadata["language"]])

    def test_body_language_bar_marks_only_the_other_language(self):
        site = Site()
        site.pages["index", "en"]["body"] = '<p><a href="README.md">English</a> · <a href="README.fr.md">Français</a></p>'
        body = site.rewritten_body("index", "en")
        parsed = Page(body)
        self.assertEqual(len(parsed.language_links), 1)
        self.assertEqual(parsed.language_links[0]["hreflang"], "fr")
        self.assertEqual(parsed.language_links[0]["href"], "../fr/documentation/index.html")
        self.assertIn('<a href="index.html">English</a>', body)

    def test_user_guide_browser_title_is_distinct_from_the_product(self):
        en = (self.output / "documentation/index.html").read_text(encoding="utf-8")
        fr = (self.output / "fr/documentation/index.html").read_text(encoding="utf-8")
        self.assertIn("<title>User guide — Super Menu</title>", en)
        self.assertIn("<title>Guide d’utilisation — Super Menu</title>", fr)
        self.assertIn('<h1 id="super-menu">Super Menu</h1>', en)

    def test_settings_anchor_switches_to_the_matching_section(self):
        self.assertEqual(self.manifest["pages"]["documentation/index.html"]["anchors"]["settings-and-index"], "reglages-et-index")
        self.assertEqual(heading_slug("Réglages et index"), "reglages-et-index")

    def test_original_images_and_application_download_are_preserved(self):
        for source in (ROOT / "docs").iterdir():
            if source.is_file():
                self.assertEqual(source.read_bytes(), (self.output / "assets/docs" / source.name).read_bytes())
        landing = (self.output / "index.html").read_text(encoding="utf-8")
        self.assertIn("releases/download/v0.6.18/SuperMenu-Setup-x64.exe", landing)
        self.assertIn('data-doc-en="documentation/performance.html"', landing)
        self.assertIn('data-doc-fr="fr/documentation/performance.html"', landing)

    def test_changed_source_hash_is_rejected(self):
        output = self.mutated_output()
        manifest_path = output / "documentation-manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["pages"]["documentation/index.html"]["sourceSha256"] = "0" * 64
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        with self.assertRaises(AssertionError):
            validate(output)

    def test_broken_local_anchor_is_rejected(self):
        output = self.mutated_output()
        page = output / "documentation/index.html"
        value = page.read_text(encoding="utf-8").replace('href="#main"', 'href="#missing-section"', 1)
        page.write_text(value, encoding="utf-8")
        with self.assertRaisesRegex(AssertionError, "Broken local anchor"):
            validate(output)

    def test_stale_asset_fingerprint_is_rejected(self):
        output = self.mutated_output()
        page = output / "documentation/index.html"
        value = page.read_text(encoding="utf-8")
        original = value.split("documentation.css?v=", 1)[1][:12]
        page.write_text(value.replace(original, "0" * 12), encoding="utf-8")
        with self.assertRaisesRegex(AssertionError, "Stale asset fingerprint"):
            validate(output)

    def test_wrong_language_metadata_is_rejected(self):
        output = self.mutated_output()
        page = output / "fr/documentation/testing.html"
        page.write_text(page.read_text(encoding="utf-8").replace('<html lang="fr">', '<html lang="en">'), encoding="utf-8")
        with self.assertRaises(AssertionError):
            validate(output)

    def test_missing_translated_paragraph_is_rejected(self):
        output = self.mutated_output()
        page = output / "fr/documentation/index.html"
        value = page.read_text(encoding="utf-8")
        paragraph = next(part for part in re.findall(r"<p>.*?</p>", value, re.S) if "data-language-switch" not in part)
        value = value.replace(paragraph, "<p>Texte manquant.</p>", 1)
        page.write_text(value, encoding="utf-8")
        with self.assertRaisesRegex(AssertionError, "Incomplete rendered source"):
            validate(output)

    def test_existing_output_is_never_deleted(self):
        output = self.mutated_output()
        before = (output / "index.html").read_bytes()
        with self.assertRaisesRegex(ValueError, "must be empty"):
            self.site.build(output)
        self.assertEqual((output / "index.html").read_bytes(), before)

    def test_non_commit_revision_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "complete Git commit"):
            Site(revision="main")

    def source_fixture(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        fixture = Path(directory.name)
        for pair in DOCUMENTS.values():
            for source in pair:
                target = fixture / source
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / source, target)
        return fixture

    def test_unpaired_headings_are_rejected(self):
        fixture = self.source_fixture()
        target = fixture / "README.fr.md"
        target.write_text(target.read_text(encoding="utf-8") + "\n## Section non traduite\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "Unpaired heading hierarchy"):
            Site(root=fixture)

    def test_changed_executable_example_is_rejected(self):
        fixture = self.source_fixture()
        target = fixture / "README.fr.md"
        target.write_text(target.read_text(encoding="utf-8") + "\n```powershell\nWrite-Host 'unexpected example'\n```\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "snippets or license quotations"):
            Site(root=fixture)


if __name__ == "__main__":
    unittest.main()

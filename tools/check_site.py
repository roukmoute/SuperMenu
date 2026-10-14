#!/usr/bin/env python3
"""Validate complete paired documentation and every generated local reference."""
from __future__ import annotations

import argparse
import hashlib
import json
import posixpath
import re
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

from build_site import BASE_URL, DOCUMENTS, ROOT, DocumentHTML, Site, route


class Page(HTMLParser):
    def __init__(self, value):
        super().__init__(convert_charrefs=True)
        self.language = None
        self.ids = set()
        self.references = []
        self.metadata = {}
        self.alternates = {}
        self.canonical = None
        self.language_links = []
        self.documentation_links = []
        self.anchor_maps = []
        self.headings = DocumentHTML(value).headings
        self.code = DocumentHTML(value).code
        self.feed(value)

    def handle_starttag(self, tag, attributes):
        attrs = dict(attributes)
        if tag == "html":
            self.language = attrs.get("lang")
        if "id" in attrs:
            if attrs["id"] in self.ids:
                raise ValueError(f'Duplicate HTML id: {attrs["id"]}')
            self.ids.add(attrs["id"])
        if tag == "meta" and "name" in attrs:
            self.metadata[attrs["name"]] = attrs.get("content")
        if tag == "link" and attrs.get("rel") == "canonical":
            self.canonical = attrs["href"]
        if tag == "link" and attrs.get("rel") == "alternate":
            self.alternates[attrs["hreflang"]] = attrs["href"]
        for attribute in ("href", "src"):
            if attribute in attrs:
                self.references.append((tag, attrs[attribute]))
        if tag == "a" and "data-language-switch" in attrs:
            self.language_links.append(attrs)
        if tag == "a" and "data-documentation" in attrs:
            self.documentation_links.append(attrs)
        if "data-anchor-map" in attrs:
            self.anchor_maps.append(json.loads(attrs["data-anchor-map"]))


def validate(output: Path, root: Path = ROOT) -> dict:
    output = output.resolve()
    manifest = json.loads((output / "documentation-manifest.json").read_text(encoding="utf-8"))
    assert manifest["languages"] == ["en", "fr"]
    assert manifest["defaultDocumentationLanguage"] == "en"
    assert manifest["applicationVersion"] == "0.6.18"
    assert set(manifest["pages"]) == {route(document, language) for document in DOCUMENTS for language in ("en", "fr")}
    assert re.fullmatch(r"[0-9a-f]{40}", manifest["revision"])
    expected = Site(root=root, revision=manifest["revision"])
    files = {path.relative_to(output).as_posix(): path for path in output.rglob("*") if path.is_file()}
    pages = {path: Page(file.read_text(encoding="utf-8")) for path, file in files.items() if path.endswith(".html")}
    assert set(pages) == {"index.html", *manifest["pages"]}, "Unexpected/missing public HTML pages"
    assert ".nojekyll" in files
    for path, metadata in manifest["pages"].items():
        page = pages[path]
        language = metadata["language"]
        document = metadata["document"]
        source = root / metadata["source"]
        assert page.language == language
        assert hashlib.sha256(source.read_bytes()).hexdigest() == metadata["sourceSha256"]
        assert page.metadata["doc-source"] == metadata["source"]
        assert page.metadata["doc-source-sha256"] == metadata["sourceSha256"]
        assert page.metadata["doc-source-revision"] == manifest["revision"]
        assert page.canonical == BASE_URL + path
        assert page.alternates == {"en": BASE_URL + route(document, "en"), "fr": BASE_URL + route(document, "fr"), "x-default": BASE_URL + route(document, "en")}
        assert len(page.language_links) == 2, f"Missing navigation or body language switch: {path}"
        other = "fr" if language == "en" else "en"
        assert all(link["hreflang"] == other for link in page.language_links)
        assert page.anchor_maps == [metadata["anchors"]]
        assert page.headings == metadata["headings"]
        assert page.code == expected.pages[document, language]["parsed"].code
        assert expected.rewritten_body(document, language) in files[path].read_text(encoding="utf-8"), f"Incomplete rendered source: {path}"
        assert metadata["anchors"] == expected.anchor_maps[document, language]
        target_ids = pages[metadata["alternate"]].ids
        assert set(metadata["anchors"]) <= page.ids
        assert set(metadata["anchors"].values()) <= target_ids
        reverse = manifest["pages"][metadata["alternate"]]["anchors"]
        assert all(reverse[target] == original for original, target in metadata["anchors"].items())
    for path, page in pages.items():
        for tag, value in page.references:
            parts = urlsplit(value)
            if parts.scheme or parts.netloc:
                allowed_schemes = {"https", "http", "mailto"} if tag == "a" else {"https"}
                assert parts.scheme in allowed_schemes, f"Unsafe reference: {path}: {value}"
                if tag in {"script", "img"} or (tag == "link" and not value.startswith(BASE_URL)):
                    raise AssertionError(f"Unexpected remote runtime asset: {path}: {value}")
                continue
            target = posixpath.normpath(posixpath.join(posixpath.dirname(path), unquote(parts.path))) if parts.path else path
            assert target in files, f"Broken local reference: {path}: {value}"
            if parts.fragment and target in pages:
                assert unquote(parts.fragment) in pages[target].ids, f"Broken local anchor: {path}: {value}"
            if parts.query.startswith("v="):
                assert parts.query[2:] == hashlib.sha256(files[target].read_bytes()).hexdigest()[:12], f"Stale asset fingerprint: {path}: {value}"
    for asset, digest in manifest["assets"].items():
        assert asset in files
        assert hashlib.sha256(files[asset].read_bytes()).hexdigest() == digest
    assert set(manifest["assets"]) == {path for path in files if not path.endswith(".html") and path != "documentation-manifest.json"}
    for source in (root / "site").rglob("*"):
        if source.is_file() and source.name != "index.html":
            destination = output / source.relative_to(root / "site")
            assert destination.read_bytes() == source.read_bytes(), f"Modified landing asset: {source}"
    for source in (root / "docs").rglob("*"):
        if source.is_file():
            destination = output / "assets/docs" / source.relative_to(root / "docs")
            assert destination.read_bytes() == source.read_bytes(), f"Modified documentation image: {source}"
    landing = pages["index.html"]
    assert landing.documentation_links, "Landing page has no documentation entry"
    for link in landing.documentation_links:
        for language in ("en", "fr"):
            destination = urlsplit(link["data-doc-" + language])
            assert destination.path == route(link["data-documentation"], language)
            assert destination.path in pages
            if destination.fragment:
                assert unquote(destination.fragment) in pages[destination.path].ids
    assert 'releases/download/v0.6.18/SuperMenu-Setup-x64.exe' in files["index.html"].read_text(encoding="utf-8")
    result = {"documentationPages": len(manifest["pages"]), "htmlPages": len(pages), "languages": manifest["languages"], "assets": len(manifest["assets"]), "revision": manifest["revision"]}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    print(json.dumps(validate(args.output), ensure_ascii=False))


if __name__ == "__main__":
    main()

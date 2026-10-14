#!/usr/bin/env python3
"""Build the Super Menu landing page and complete bilingual documentation."""
from __future__ import annotations

import argparse
import hashlib
import html
import json
import posixpath
import re
import shutil
import subprocess
import unicodedata
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import quote, unquote, urlsplit, urlunsplit

import markdown

ROOT = Path(__file__).resolve().parents[1]
BASE_URL = "https://roukmoute.github.io/SuperMenu/"
REPOSITORY = "https://github.com/roukmoute/SuperMenu"
DOCUMENTS = {
    "index": ("README.md", "README.fr.md"),
    "testing": ("TESTING.md", "TESTING.fr.md"),
    "performance": ("PERFORMANCE.md", "PERFORMANCE.fr.md"),
    "site": ("SITE.md", "SITE.fr.md"),
    "translations": ("locales/README.md", "locales/README.fr.md"),
    "notices": ("THIRD_PARTY_NOTICES.md", "THIRD_PARTY_NOTICES.fr.md"),
}
LABELS = {
    "en": {"index": "User guide", "testing": "Testing", "performance": "Performance", "site": "Website maintenance", "translations": "Translation packs", "notices": "Third-party notices", "documentation": "Documentation", "home": "Discover Super Menu", "on_this_page": "On this page", "source": "Source of this page", "skip": "Skip to content", "navigation": "Documentation navigation", "language": "Language", "description": "Complete Super Menu documentation: usage, Windows testing, performance measurements, translation packs and website maintenance."},
    "fr": {"index": "Guide d’utilisation", "testing": "Tests", "performance": "Performances", "site": "Maintenance du site", "translations": "Packs de traduction", "notices": "Mentions tierces", "documentation": "Documentation", "home": "Découvrir Super Menu", "on_this_page": "Sur cette page", "source": "Source de cette page", "skip": "Aller au contenu", "navigation": "Navigation de la documentation", "language": "Langue", "description": "Documentation complète de Super Menu : utilisation, tests Windows, mesures de performances, packs de traduction et maintenance du site."},
}


def route(document: str, language: str) -> str:
    return ("fr/" if language == "fr" else "") + f"documentation/{document}.html"


def heading_slug(value: str, separator: str = "-") -> str:
    """Use stable ASCII heading anchors in both website editions."""
    value = html.unescape(re.sub(r"<[^>]+>", "", value)).strip().lower()
    value = unicodedata.normalize("NFKD", value).encode("ascii", "ignore").decode("ascii")
    return re.sub(r"\s", separator, re.sub(r"[^\w\- ]", "", value))


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def href(from_route: str, to_route: str, fragment: str = "", query: str = "") -> str:
    relative = posixpath.relpath(to_route, posixpath.dirname(from_route) or ".")
    return relative + ("?" + query if query else "") + ("#" + quote(fragment, safe="-_~") if fragment else "")


class DocumentHTML(HTMLParser):
    """Inspect Markdown's actual HTML without guessing heading text from source."""
    def __init__(self, value: str):
        super().__init__(convert_charrefs=True)
        self.headings: list[dict] = []
        self.links: list[tuple[str, str]] = []
        self.code: list[str] = []
        self._heading = None
        self._code_depth = 0
        self.feed(value)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if re.fullmatch(r"h[1-6]", tag):
            self._heading = {"level": int(tag[1]), "id": attrs.get("id", ""), "text": ""}
        if tag == "a" and "href" in attrs:
            self.links.append(("href", attrs["href"]))
        if tag == "img" and "src" in attrs:
            self.links.append(("src", attrs["src"]))
        if tag == "pre":
            self._code_depth += 1
            self.code.append("")

    def handle_endtag(self, tag):
        if self._heading is not None and tag == f'h{self._heading["level"]}':
            self.headings.append(self._heading)
            self._heading = None
        if tag == "pre":
            self._code_depth -= 1

    def handle_data(self, value):
        if self._heading is not None:
            self._heading["text"] += value
        if self._code_depth:
            self.code[-1] += value


def render_markdown(source: str) -> str:
    return markdown.markdown(source, extensions=["fenced_code", "tables", "sane_lists", "toc"], extension_configs={"toc": {"slugify": heading_slug, "permalink": False}})


def checked_revision(value: str | None) -> str:
    revision = value or subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("Documentation revision must be a complete Git commit SHA")
    return revision


class Site:
    def __init__(self, root: Path = ROOT, revision: str | None = None):
        self.root = root
        self.revision = checked_revision(revision)
        self.pages = {}
        self.sources = {}
        self.anchor_maps = {}
        for document, pair in DOCUMENTS.items():
            for language, source in zip(("en", "fr"), pair):
                raw = (root / source).read_bytes()
                body = render_markdown(raw.decode("utf-8"))
                parsed = DocumentHTML(body)
                if not parsed.headings or parsed.headings[0]["level"] != 1:
                    raise ValueError(f"Missing document title: {source}")
                self.pages[document, language] = {"source": source, "body": body, "parsed": parsed, "sha256": sha256(raw)}
                self.sources[source] = (document, language)
            en = self.pages[document, "en"]["parsed"]
            fr = self.pages[document, "fr"]["parsed"]
            if [h["level"] for h in en.headings] != [h["level"] for h in fr.headings]:
                raise ValueError(f"Unpaired heading hierarchy: {document}")
            if en.code != fr.code:
                raise ValueError(f"Executable snippets or license quotations differ between languages: {document}")
            for source_language, target_language in (("en", "fr"), ("fr", "en")):
                self.anchor_maps[document, source_language] = dict(zip(
                    (h["id"] for h in self.pages[document, source_language]["parsed"].headings),
                    (h["id"] for h in self.pages[document, target_language]["parsed"].headings),
                ))

    def rewrite_url(self, value: str, source: str, from_route: str, language: str) -> str:
        parts = urlsplit(html.unescape(value))
        if parts.scheme or parts.netloc or not parts.path:
            return value
        target = posixpath.normpath(posixpath.join(posixpath.dirname(source), unquote(parts.path)))
        if target.startswith("../") or target.startswith("/"):
            raise ValueError(f"Link escapes repository: {source}: {value}")
        candidate = self.root / target
        if not candidate.exists():
            raise ValueError(f"Missing source link: {source}: {value}")
        if target in self.sources:
            document, declared_language = self.sources[target]
            # Explicit *.fr.md / README.md language switches keep their requested locale.
            target_language = declared_language
            fragment = unquote(parts.fragment)
            ids = {heading["id"] for heading in self.pages[document, target_language]["parsed"].headings}
            if fragment and fragment not in ids:
                raise ValueError(f"Missing Markdown anchor: {source}: {value}")
            return href(from_route, route(document, target_language), fragment, parts.query)
        if target.startswith("docs/") and candidate.is_file() and candidate.suffix.lower() in {".png", ".svg", ".jpg", ".jpeg", ".webp"}:
            return href(from_route, "assets/" + target, unquote(parts.fragment), parts.query)
        kind = "tree" if candidate.is_dir() else "blob"
        return f"{REPOSITORY}/{kind}/{self.revision}/{quote(target, safe='/')}" + ("#" + parts.fragment if parts.fragment else "")

    def rewritten_body(self, document: str, language: str) -> str:
        page = self.pages[document, language]
        def replace(match):
            url = html.unescape(match.group(2))
            replacement = self.rewrite_url(url, page["source"], route(document, language), language)
            return f'{match.group(1)}="{html.escape(replacement, quote=True)}"'
        body = re.sub(r'\b(href|src)="([^"]*)"', replace, page["body"])
        other = "fr" if language == "en" else "en"
        alternate = route(document, other)
        def mark_language_link(match):
            tag = match.group(0)
            destination = re.search(r'\bhref="([^"]*)"', tag)
            if not destination:
                return tag
            parts = urlsplit(html.unescape(destination.group(1)))
            if parts.scheme or parts.netloc or not parts.path:
                return tag
            target = posixpath.normpath(posixpath.join(posixpath.dirname(route(document, language)), unquote(parts.path)))
            if target != alternate:
                return tag
            return tag[:-1] + f' data-language-switch hreflang="{other}" lang="{other}">'
        return re.sub(r'<a\b[^>]*>', mark_language_link, body)

    def document_html(self, document: str, language: str, css_hash: str, js_hash: str) -> str:
        page = self.pages[document, language]
        current = route(document, language)
        other = "fr" if language == "en" else "en"
        labels = LABELS[language]
        title = labels["index"] if document == "index" else page["parsed"].headings[0]["text"]
        canonical = BASE_URL + current
        escape = html.escape
        links = "".join(f'<a href="{href(current, route(key, language))}"{ " aria-current=\"page\"" if key == document else ""}>{escape(labels[key])}</a>' for key in DOCUMENTS)
        language_links = "".join(f'<a href="{href(current, route(document, lang))}" lang="{lang}" hreflang="{lang}"{ " aria-current=\"page\"" if lang == language else " data-language-switch"}>{name}</a>' for lang, name in (("en", "English"), ("fr", "Français")))
        toc = "".join(f'<li><a href="#{quote(h["id"], safe="-_~")}">{escape(h["text"])}</a></li>' for h in page["parsed"].headings if h["level"] == 2)
        source_url = f'{REPOSITORY}/blob/{self.revision}/{page["source"]}'
        anchor_data = escape(json.dumps(self.anchor_maps[document, language], ensure_ascii=False), quote=True)
        return f'''<!doctype html>
<html lang="{language}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="description" content="{escape(labels['description'], quote=True)}">
<meta name="theme-color" content="#f6f4fb">
<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; object-src 'none'; base-uri 'self'; form-action 'none'">
<meta name="doc-source" content="{escape(page['source'], quote=True)}">
<meta name="doc-source-sha256" content="{page['sha256']}">
<meta name="doc-source-revision" content="{self.revision}">
<meta property="og:type" content="article">
<meta property="og:title" content="{escape(title, quote=True)} — Super Menu">
<meta property="og:url" content="{canonical}">
<link rel="canonical" href="{canonical}">
<link rel="alternate" hreflang="en" href="{BASE_URL + route(document, 'en')}">
<link rel="alternate" hreflang="fr" href="{BASE_URL + route(document, 'fr')}">
<link rel="alternate" hreflang="x-default" href="{BASE_URL + route(document, 'en')}">
<link rel="icon" href="{href(current, 'assets/mark.svg')}" type="image/svg+xml">
<link rel="stylesheet" href="{href(current, 'documentation.css')}?v={css_hash}">
<script src="{href(current, 'documentation.js')}?v={js_hash}" defer></script>
<title>{escape(title)} — Super Menu</title>
</head>
<body>
<a class="skip-link" href="#main">{labels['skip']}</a>
<header class="doc-header"><a class="brand" href="{href(current, 'index.html', query='lang=' + language)}"><img src="{href(current, 'assets/mark.svg')}" width="25" height="25" alt="">SUPER MENU<span>.</span></a><a class="home-link" href="{href(current, 'index.html', query='lang=' + language)}">{labels['home']}</a><nav class="languages" aria-label="{labels['language']}" data-anchor-map="{anchor_data}">{language_links}</nav></header>
<div class="doc-layout">
<aside class="doc-sidebar"><p class="eyebrow">{labels['documentation']}</p><nav aria-label="{labels['navigation']}">{links}</nav>{'<div class="toc"><p class="eyebrow">' + labels['on_this_page'] + '</p><ol>' + toc + '</ol></div>' if toc else ''}</aside>
<main id="main" class="doc-content">{self.rewritten_body(document, language)}<footer class="doc-footer"><a href="{source_url}">{labels['source']} ↗</a><span>Super Menu 0.6.18</span></footer></main>
</div>
</body>
</html>
'''

    def landing_html(self) -> str:
        source = (self.root / "site/index.html").read_text(encoding="utf-8")
        def replace(match):
            tag = match.group(0)
            document_match = re.search(r'data-documentation="([^"]+)"', tag)
            if not document_match:
                return tag
            document = document_match.group(1)
            heading_match = re.search(r'data-documentation-heading="([^"]+)"', tag)
            fragments = {"en": "", "fr": ""}
            if heading_match:
                # The stable semantic key is the original French README heading.
                original = heading_match.group(1)
                if original not in self.anchor_maps[document, "fr"]:
                    raise ValueError(f"Landing documentation heading is missing: {original}")
                fragments = {"fr": original, "en": self.anchor_maps[document, "fr"][original]}
            targets = {lang: route(document, lang) + ("#" + quote(fragments[lang], safe="-_~") if fragments[lang] else "") for lang in ("en", "fr")}
            tag = re.sub(r' href="[^"]*"', f' href="{targets["fr"]}"', tag)
            return tag[:-1] + f' data-doc-en="{targets["en"]}" data-doc-fr="{targets["fr"]}">'
        source = re.sub(r'<a\b[^>]*>', replace, source)
        for asset in ("styles.css", "script.js"):
            fingerprint = sha256((self.root / "site" / asset).read_bytes())[:12]
            source = re.sub(re.escape(asset) + r'\?v=[0-9a-f]+', asset + "?v=" + fingerprint, source)
        return source

    def build(self, output: Path) -> dict:
        if output.exists() and any(output.iterdir()):
            raise ValueError("The output directory must be empty; no existing files are deleted")
        output.mkdir(parents=True, exist_ok=True)
        shutil.copytree(self.root / "site", output, dirs_exist_ok=True)
        shutil.copytree(self.root / "docs", output / "assets/docs")
        css_hash = sha256((self.root / "site/documentation.css").read_bytes())[:12]
        js_hash = sha256((self.root / "site/documentation.js").read_bytes())[:12]
        manifest = {"schemaVersion": 1, "defaultDocumentationLanguage": "en", "languages": ["en", "fr"], "revision": self.revision, "applicationVersion": "0.6.18", "pages": {}, "assets": {}}
        for (document, language), page in self.pages.items():
            physical = route(document, language)
            destination = output / physical
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(self.document_html(document, language, css_hash, js_hash), encoding="utf-8", newline="\n")
            manifest["pages"][physical] = {"document": document, "language": language, "source": page["source"], "sourceSha256": page["sha256"], "headings": page["parsed"].headings, "alternate": route(document, "fr" if language == "en" else "en"), "anchors": self.anchor_maps[document, language]}
        (output / "index.html").write_text(self.landing_html(), encoding="utf-8", newline="\n")
        for asset in sorted(output.rglob("*")):
            if asset.is_file() and asset.suffix != ".html":
                manifest["assets"][asset.relative_to(output).as_posix()] = sha256(asset.read_bytes())
        (output / "documentation-manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("_site"))
    parser.add_argument("--revision")
    args = parser.parse_args()
    manifest = Site(revision=args.revision).build(args.output.resolve())
    print(f'Built landing page and {len(manifest["pages"])} documentation pages at {args.output}')


if __name__ == "__main__":
    main()

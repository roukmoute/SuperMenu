"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

class Element {
  constructor(text = "", dataset = {}) {
    this.textContent = text; this.dataset = dataset; this.children = []; this.attributes = {}; this.listeners = {};
    this.classList = { add() {}, toggle() {} }; this.value = "artisan"; this.alt = "Capture française";
  }
  cloneNode() { return new Element(this.textContent, this.dataset); }
  querySelectorAll() { return []; }
  replaceChildren(...children) { this.children = children; this.textContent = children.map(child => child.textContent || "").join(""); }
  append(...children) { this.children.push(...children); this.textContent += children.map(child => child.textContent || "").join(""); }
  setAttribute(key, value) { this.attributes[key] = value; }
  getAttribute(key) { return this.attributes[key]; }
  addEventListener(key, listener) { this.listeners[key] = listener; }
}

function landing(url) {
  const languageButton = new Element("EN");
  const query = new Element();
  const screenshot = new Element();
  const docs = new Element("Documentation", { documentation: "index", docEn: "documentation/index.html#usage", docFr: "fr/documentation/index.html#utiliser" });
  const translated = new Element("La rapidité", { i18n: "navSpeed" });
  const generic = new Element();
  const nodes = { "language-button": languageButton, "demo-query": query, "demo-results": new Element(), "demo-category": new Element() };
  const document = {
    documentElement: { lang: "fr" }, title: "", getElementById: id => nodes[id],
    querySelector: selector => selector === "[data-i18n-alt]" ? screenshot : generic,
    querySelectorAll: selector => selector === "[data-i18n]" ? [translated] : selector === "[data-documentation]" ? [docs] : [],
    createElement: () => new Element(), createElementNS: () => new Element(), createTextNode: text => new Element(text)
  };
  const window = { location: { href: url }, history: { replaceState(_state, _title, value) { window.location.href = String(value); } } };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "../../site/script.js"), "utf8"), { document, window, URL });
  return { document, window, docs, languageButton, translated, query, nodes };
}

let state = landing("https://roukmoute.github.io/SuperMenu/?lang=en&from=blog#modules");
assert.equal(state.document.documentElement.lang, "en");
assert.equal(state.translated.textContent, "Responsiveness");
assert.equal(state.docs.attributes.href, "documentation/index.html#usage");
assert.equal(state.languageButton.textContent, "FR");
console.log("PASS explicit English selects translated landing and documentation");
state.languageButton.listeners.click();
assert.equal(state.document.documentElement.lang, "fr");
assert.equal(state.docs.attributes.href, "fr/documentation/index.html#utiliser");
assert.equal(new URL(state.window.location.href).searchParams.get("lang"), "fr");
assert.equal(new URL(state.window.location.href).searchParams.get("from"), "blog");
assert.equal(new URL(state.window.location.href).hash, "#modules");
console.log("PASS language choice retains query parameters and current section");
state = landing("https://roukmoute.github.io/SuperMenu/?lang=fr");
assert.equal(state.document.documentElement.lang, "fr");
assert.equal(state.translated.textContent, "La rapidité");
assert.equal(state.docs.attributes.href, "fr/documentation/index.html#utiliser");
console.log("PASS explicit French chooses complete French documentation");
state = landing("https://roukmoute.github.io/SuperMenu/?lang=unexpected");
assert.equal(state.document.documentElement.lang, "fr");
console.log("PASS unknown locale keeps the existing French fallback");

function documentation(url, href, map) {
  const link = { href, hreflang: "fr", listeners: {}, getAttribute() { return href; }, addEventListener(name, callback) { this.listeners[name] = callback; } };
  const bodyLink = { href, hreflang: "fr", listeners: {}, getAttribute() { return href; }, addEventListener(name, callback) { this.listeners[name] = callback; } };
  const nav = { dataset: { anchorMap: JSON.stringify(map) }, querySelectorAll() { return [link]; } };
  const listeners = {};
  const window = { location: { href: url }, addEventListener(name, callback) { listeners[name] = callback; } };
  const document = { querySelector() { return nav; }, querySelectorAll() { return [link, bodyLink]; } };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "../../site/documentation.js"), "utf8"), { document, window, URL });
  return { link, bodyLink, window, listeners };
}
let docs = documentation("https://roukmoute.github.io/SuperMenu/documentation/index.html?from=blog#settings-and-index", "../fr/documentation/index.html", { "settings-and-index": "reglages-et-index" });
assert.equal(docs.link.href, "https://roukmoute.github.io/SuperMenu/fr/documentation/index.html?from=blog#reglages-et-index");
console.log("PASS documentation language link selects the matching translated section");
assert.equal(docs.bodyLink.href, docs.link.href);
docs.window.location.href = "https://roukmoute.github.io/SuperMenu/documentation/index.html?from=body#settings-and-index";
docs.bodyLink.listeners.click();
assert.equal(docs.bodyLink.href, "https://roukmoute.github.io/SuperMenu/fr/documentation/index.html?from=body#reglages-et-index");
console.log("PASS Markdown language bar preserves query and translated section");
docs.window.location.href = "https://roukmoute.github.io/SuperMenu/documentation/index.html?from=blog#usage";
docs.listeners.hashchange();
assert.equal(new URL(docs.link.href).hash, "#usage");
console.log("PASS documentation switch updates when the reader changes section");
docs.window.location.href = "https://roukmoute.github.io/SuperMenu/documentation/index.html?from=blog#settings-and-index";
docs.link.listeners.click();
assert.equal(new URL(docs.link.href).hash, "#reglages-et-index");
console.log("PASS language click reads the latest section before navigation");
docs = documentation("https://roukmoute.github.io/SuperMenu/documentation/index.html#settings-and-index", "https://external.example/fr/", { "settings-and-index": "reglages-et-index" });
assert.equal(docs.link.href, "https://external.example/fr/");
console.log("PASS documentation helper does not rewrite external destinations");
docs = documentation("https://roukmoute.github.io/SuperMenu/documentation/index.html#%E0%A4%A", "../fr/documentation/index.html", {});
assert.equal(new URL(docs.link.href).hash, "");
console.log("PASS malformed percent-encoded section is handled without a script error");

for (const fragment of ["constructor", "toString", "__proto__"]) {
  docs = documentation(`https://roukmoute.github.io/SuperMenu/documentation/index.html#${fragment}`, "../fr/documentation/index.html", {});
  assert.equal(new URL(docs.link.href).hash, `#${fragment}`);
  assert.equal(new URL(docs.bodyLink.href).hash, `#${fragment}`);
}
console.log("PASS unknown prototype-named anchors stay literal in both language links");
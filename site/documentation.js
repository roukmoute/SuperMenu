"use strict";

(() => {
  const navigation = document.querySelector("[data-anchor-map]");
  if (!navigation) return;
  const anchors = JSON.parse(navigation.dataset.anchorMap);
  const links = [...document.querySelectorAll("[data-language-switch]")];
  const originals = new Map(links.map(link => [link, link.getAttribute("href")]));
  function updateLinks() {
    const current = new URL(window.location.href);
    let fragment;
    try { fragment = decodeURIComponent(current.hash.slice(1)); }
    catch { fragment = ""; }
    for (const link of links) {
      const target = new URL(originals.get(link), current);
      if (target.origin !== current.origin) continue;
      target.search = current.search;
      if (target.searchParams.has("lang")) target.searchParams.set("lang", link.hreflang);
      target.hash = fragment ? (Object.hasOwn(anchors, fragment) ? anchors[fragment] : fragment) : "";
      link.href = target.href;
    }
  }
  for (const link of links) {
    link.addEventListener("click", updateLinks);
    link.addEventListener("auxclick", updateLinks);
  }
  window.addEventListener("hashchange", updateLinks);
  window.addEventListener("popstate", updateLinks);
  updateLinks();
})();

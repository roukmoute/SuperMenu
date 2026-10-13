"use strict";

(() => {
  const english = {
    navSpeed: "Responsiveness",
    demoGames: "Steam games",
    factSteamTitle: "Steam games",
    factSteamText: "Your installed games, if you want them.",
    speedEyebrow: "RESPONSIVE FROM THE FIRST LETTERS",
    speedTitle: "A quick shortcut.\nA quick next step.",
    speed1Title: "Results that keep up with your typing.",
    speed1Text: "Search for apps in a local, in-memory index. Each letter narrows the results without waiting for a disk search or an online service.",
    speed2Title: "Ready when you come back.",
    speed2Text: "Win + Space brings back results that are already available. Index updates and icons are prepared in the background.",
    speed3Title: "Quiet between shortcuts.",
    speed3Text: "File changes are grouped while the menu is hidden. When you open it, results in memory appear before the update runs.",
    speedProof: "Search, display, memory: measurements and their methods are public. Results depend on your PC and indexed catalog.",
    viewPerformance: "View the measurements",
    moduleSteam: "Steam games",
    moduleSteamText: "Your installed games in app search. Enable the option in settings; it is off by default.",
    faqSteamQuestion: "How do I add my Steam games?",
    faqSteamAnswer: "Enable “Installed Steam games” in settings. Games registered by Steam on Windows appear in app search with their local icons. Enter launches them through Steam. The option is off by default; games without these registrations are not added automatically.",
    skip: "Skip to content", navModules: "What it can do", navSource: "Source code ↗", navDownload: "Download ↓",
    heroEyebrow: "NATIVE WINDOWS. SMALL. RESPONSIVE.", heroTitle: "A menu that", heroEmphasis: "keeps up with you.",
    heroDescription: "One shortcut, results that keep up with your typing. Apps, files, timers, and optional Steam games: everyday actions in a launcher around 1 MiB.",
    downloadWindows: "Download for Windows", releaseNotes: "Explore version 0.6.18", downloadNote: "Windows 10 / 11 · x64 · free and open source",
    space: "Space", shortcutHint: "Your new starting point.", previewLabel: "TRY THE PREVIEW", previewType: "illustrative demo", tagline: "Right at your fingertips",
    demoInputLabel: "Search the illustrative demo examples", demoApps: "Applications", demoFiles: "Files", demoTimer: "Timer", demoCalc: "Calculator", demoLocal: "local index", demoHelp: "↑ ↓ navigate   ↵ open   Esc close",
    previewHint: "Switch previews or try “artisan”, “chrome”, “/ project”, “timer 5m”.",
    sizeNumber: "~ 1", sizeUnit: "MiB", factSize: "The Windows executable in version 0.6.18", native: "native", factNative: "Rust + Win32. No embedded browser.", factProof: "Small and responsive.\nPublic measurements.",
    overviewEyebrow: "LET THE KEYBOARD LEAD THE WAY", overviewTitle: "Fewer detours.\nMore everyday flow.",
    screenshotAlt: "A real screenshot of Super Menu in its light theme: applications, installation, files, calculator, timers and clipboard.", screenshotCaption: "The real app, in its light theme.",
    note1Title: "The right result, front and center.", note1Text: "Without a prefix, search your applications. Files have their own entry with /. Real Windows shortcuts and icons keep their place.",
    note2Title: "Missing an app? Install it.", note2Text: "Chrome, Steam, VLC, Obsidian… Easily install 245 applications right from the menu. Installed applications can also be uninstalled and cleaned up.",
    note3Title: "Your games too, if you want them.", note3Text: "Enable “Installed Steam games” in settings. Find games registered by Steam on Windows with their icons, then launch them from the same search as your applications.",
    localNote: "Your application and file searches stay local. No remote service is required.",
    modulesEyebrow: "ONE SMALL WINDOW. PLENTY OF POSSIBILITIES.", modulesTitle: "Everyday actions,\nin a few characters.", modulesAside: "A prefix to switch context.\nEnter to take action.",
    moduleApps: "Applications", moduleAppsText: "Search without a prefix and launch your apps with their real Windows icons.", moduleFiles: "Files & folders", moduleFilesText: "Find files in the folders you choose to index.", moduleFilesExample: "/ project",
    moduleInstall: "Install & remove", moduleInstallText: "A WinGet catalog to install, an action to uninstall.", moduleTimer: "Timers", moduleTimerText: "Multiple countdowns, a pause button, and an alert when time is up.", moduleTimerExample: "timer 5m Tea",
    moduleCalc: "Calculator", moduleCalcText: "Calculate locally and copy the result without opening another app.", moduleClipboard: "Clipboard", moduleClipboardText: "A local, in-memory text history you can enable if you want it.",
    moduleWeb: "Web & commands", moduleWebText: "Search in your browser or run your own custom commands.", modulesMore: "Also included: open windows, symbols, system actions, and customizable themes.", allShortcuts: "All the shortcuts",
    downloadEyebrow: "A SMALL INSTALLER. AN AUTOMATIC LOCATION.", downloadTitle: "Three steps.", downloadEmphasis: "And it’s yours.", downloadSetup: "Download the installer", portableZip: "Portable version (.zip)", versionNote: "Version 0.6.18 · prerelease · Windows x64",
    step1Title: "Download and install.", step1Text: "Open the installer: it puts Super Menu in the right place and adds its Start Menu shortcut. No administrator rights required.",
    step2Title: "Launch Super Menu.", step2Text: "Choose French or English on first launch. English is downloaded when selected, cached locally, and refreshed for new app versions.",
    step3Title: "Press Win + Space.", step3Text: "Type, select, and confirm. The menu waits in the system tray, and you can customize its shortcut in settings.",
    faqEyebrow: "A FEW USEFUL ANSWERS", faqTitle: "Before your\nfirst shortcut.", faqIntro: "An independent, open project. Here’s what to know.", reportIssue: "An idea or an issue?",
    faq1Question: "Which systems are supported?", faq1Answer: "Windows 10 and Windows 11 on an x64 processor. Super Menu is a native application written in Rust with Win32. It requires neither .NET nor an embedded browser.",
    faq2Question: "Win + Space already switches my input language.", faq2Answer: "While Super Menu is running, its shortcut takes priority. You can choose another shortcut in settings, such as Ctrl + Alt + Space, to keep the Windows language switcher accessible.",
    faq3Question: "Does everything work offline?", faq3Answer: "Local search, timers, and the calculator work without a connection. WinGet installations, web searches, language downloads, and update checks require Internet access. Automatic updates download official releases in the background and preserve settings; you can disable them in settings. French is built into the executable.",
    faq4Question: "Can it install any application?", faq4Answer: "The catalog offers 245 applications you can easily install. Type a name, select the “Install” result, and confirm. WinGet needs to be available on your PC; Windows may request permission. The catalog covers these applications rather than every WinGet package.",
    faq5Question: "What does cleanup remove after an uninstall?", faq5Answer: "After a verified uninstall, cleanup targets program files, known caches, and certain system traces. It preserves profiles and personal data, and leaves residues whose ownership is uncertain. One confirmation precedes the operation; cleaned-up files go to the Recycle Bin. The application’s own uninstaller remains responsible for its changes.",
    faq6Question: "How is the project checked?", faq6Answer: "Version 0.6.18 passes 267 Rust tests and 1,275 Windows assertions, including real installation, upgrade, and uninstall tests. Performance measurements and their methodology are published in the repository. They depend on the PC and do not guarantee identical latency on every machine.", viewTests: "View the checks.",
    faq7Question: "Do timers keep running after I quit the app?", faq7Answer: "They keep running while the menu is hidden. They stay in memory: quitting Super Menu cancels them. Time spent asleep is counted, and an expired timer is reported when Windows wakes, without waking the computer.",
    lastTitle: "Your next move\nis at your fingertips.", lastDownload: "Try Super Menu", footerNote: "Built for Windows. Inspired by Walker. Independent.", license: "MIT License"
  };
  const french = {};
  const translatedNodes = [...document.querySelectorAll("[data-i18n]")];
  for (const element of translatedNodes) {
    const copy = element.cloneNode(true);
    copy.querySelectorAll("br").forEach(br => br.replaceWith("\n"));
    french[element.dataset.i18n] = copy.textContent.trim();
  }
  const screenshot = document.querySelector("[data-i18n-alt]");
  french.screenshotAlt = screenshot.alt;
  const languageButton = document.getElementById("language-button");
  const query = document.getElementById("demo-query");
  const results = document.getElementById("demo-results");
  const category = document.getElementById("demo-category");
  const tabs = [...document.querySelectorAll("[data-demo]")];
  let language = "fr";
  let mode = "apps";
  const strings = {
    fr: { apps: "APPLICATIONS", games: "JEUX STEAM", steamGame: "Jeu Steam", steamDemo: "Exemple avec l’option Steam activée : aucun jeu n’est lancé depuis cette page.", files: "FICHIERS", timer: "MINUTEUR", calc: "CALCULATRICE", app: "Application Windows", remove: "Désinstaller et nettoyer", install: "Installer · WinGet", demo: "Exemples de présentation : aucune application n’est lancée ou installée depuis cette page.", empty: "Aucun exemple pour cette recherche. Essayez obsidian, artisan, chrome, / projet ou minuteur 5m.", doc: "Document · Dossier choisi", folder: "Dossier · Index local", startTimer: "Démarrer le minuteur", minutes: "minutes", timerHelp: "Dans l’application, Entrée démarre le compte à rebours. L’alerte reste visible jusqu’à sa fermeture.", timerInvalid: "Essayez minuteur 5m. L’application accepte aussi les secondes et les heures.", copy: "Copier le résultat", calcHelp: "Exemple de calcul local. Dans l’application, le résultat est copié dans votre presse-papiers.", calcInvalid: "Essayez = (15 + 5) * 3, ou une expression avec + − * / et des parenthèses." },
    en: { apps: "APPLICATIONS", games: "STEAM GAMES", steamGame: "Steam game", steamDemo: "Example with the Steam option enabled: no game is launched from this page.", files: "FILES", timer: "TIMER", calc: "CALCULATOR", app: "Windows application", remove: "Uninstall and clean up", install: "Install · WinGet", demo: "Illustrative examples: no application is launched or installed from this page.", empty: "No example for this search. Try obsidian, artisan, chrome, / project, or timer 5m.", doc: "Document · Chosen folder", folder: "Folder · Local index", startTimer: "Start timer", minutes: "minutes", timerHelp: "In the application, Enter starts the countdown. The alert stays visible until dismissed.", timerInvalid: "Try timer 5m. The application also accepts seconds and hours.", copy: "Copy result", calcHelp: "An example of a local calculation. In the application, the result is copied to your clipboard.", calcInvalid: "Try = (15 + 5) * 3, or an expression with + − * / and parentheses." }
  };

  function textWithBreaks(element, value) {
    element.replaceChildren();
    value.split("\n").forEach((line, index) => {
      if (index) element.append(document.createElement("br"));
      element.append(document.createTextNode(line));
    });
  }

  function setLanguage(next) {
    language = next === "en" ? "en" : "fr";
    const dictionary = language === "en" ? english : french;
    for (const element of translatedNodes) {
      const value = dictionary[element.dataset.i18n];
      if (value !== undefined) textWithBreaks(element, value);
    }
    screenshot.alt = dictionary.screenshotAlt;
    document.documentElement.lang = language;
    document.title = language === "en" ? "Super Menu — Right at your fingertips" : "Super Menu — À portée de clavier";
    languageButton.textContent = language === "en" ? "FR" : "EN";
    languageButton.setAttribute("aria-label", language === "en" ? "Passer en français" : "Switch to English");
    languageButton.title = languageButton.getAttribute("aria-label");
    document.querySelector(".navigation").setAttribute("aria-label", language === "en" ? "Main navigation" : "Navigation principale");
    document.querySelector(".demo-tabs").setAttribute("aria-label", language === "en" ? "Search examples" : "Exemples de recherche");
    document.querySelector(".launcher-demo").setAttribute("aria-label", language === "en" ? "Interactive preview of Super Menu" : "Aperçu interactif de Super Menu");
    document.querySelector(".shortcut-keys").setAttribute("aria-label", language === "en" ? "Windows plus Space" : "Windows plus Espace");
    document.querySelector(".facts").setAttribute("aria-label", language === "en" ? "A few facts" : "En quelques chiffres");
    document.querySelector(".site-footer nav").setAttribute("aria-label", language === "en" ? "Project links" : "Liens du projet");
    document.querySelector(".site-header .brand").setAttribute("aria-label", language === "en" ? "Super Menu home" : "Super Menu, accueil");
    if (mode === "timer") query.value = language === "en" ? "timer 5m" : "minuteur 5m";
    if (mode === "files") query.value = language === "en" ? "/ project" : "/ projet";
    render();
  }

  function makeResult(title, subtitle, icon, selected = false) {
    const row = document.createElement("div");
    row.className = `demo-result${selected ? " is-selected" : ""}`;
    const symbol = document.createElement("span");
    symbol.className = "result-icon";
    symbol.setAttribute("aria-hidden", "true");
    if (icon === "obsidian") {
      symbol.classList.add("app-icon");
      const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      svg.setAttribute("viewBox", "0 0 28 28");
      for (const [fill, d] of [["#ac98ed", "m15 2 9 8-3 15-10 2-7-10L8 7Z"], ["#7b61bc", "m15 2-4 25-7-10L8 7Z"], ["#cfbffa", "m15 2 9 8-7 3Z"]]) {
        const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
        path.setAttribute("fill", fill); path.setAttribute("d", d); svg.append(path);
      }
      symbol.append(svg);
    } else if (icon === "game") {
      const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      svg.setAttribute("viewBox", "0 0 24 24"); svg.setAttribute("fill", "none"); svg.setAttribute("stroke", "currentColor"); svg.setAttribute("stroke-width", "1.4");
      for (const d of ["M8 7h8a4 4 0 0 1 4 3l1 6a2 2 0 0 1-3 2l-3-2H9l-3 2a2 2 0 0 1-3-2l1-6a4 4 0 0 1 4-3Z", "M6 11h4m-2-2v4m7-3h.01m3 3h.01"]) {
        const path = document.createElementNS("http://www.w3.org/2000/svg", "path"); path.setAttribute("d", d); path.setAttribute("stroke-linecap", "round"); svg.append(path);
      }
      symbol.append(svg);
    } else symbol.textContent = icon;
    const description = document.createElement("div");
    const strong = document.createElement("strong"); strong.textContent = title;
    const small = document.createElement("small"); small.textContent = subtitle;
    description.append(strong, small);
    const chevron = document.createElement("span"); chevron.className = "result-chevron"; chevron.textContent = "›"; chevron.setAttribute("aria-hidden", "true");
    row.append(symbol, description, chevron);
    results.append(row);
  }

  function note(message, empty = false) {
    const element = document.createElement("p");
    element.className = empty ? "demo-empty" : "demo-result-note";
    element.textContent = message;
    results.append(element);
  }

  // Bounded arithmetic parser. No evaluation of JavaScript or shell input.
  function calculate(expression) {
    if (!/^[\d\s.+\-*/()]+$/.test(expression) || expression.length > 80) return null;
    const tokens = expression.match(/(?:\d+\.?\d*|\.\d+)|[()+\-*/]/g) || [];
    let position = 0;
    function factor() {
      const token = tokens[position++];
      if (token === "+") return factor();
      if (token === "-") return -factor();
      if (token === "(") { const result = sum(); if (tokens[position++] !== ")") throw new Error(); return result; }
      if (token === undefined || !/^(?:\d+\.?\d*|\.\d+)$/.test(token)) throw new Error();
      return Number(token);
    }
    function product() { let value = factor(); while (tokens[position] === "*" || tokens[position] === "/") { const operator = tokens[position++]; const right = factor(); value = operator === "*" ? value * right : value / right; } return value; }
    function sum() { let value = product(); while (tokens[position] === "+" || tokens[position] === "-") { const operator = tokens[position++]; const right = product(); value = operator === "+" ? value + right : value - right; } return value; }
    try { const value = sum(); return position === tokens.length && Number.isFinite(value) ? Number(value.toPrecision(12)) : null; } catch { return null; }
  }

  function render() {
    const input = query.value.trim();
    const lower = input.toLocaleLowerCase(language);
    if (input.startsWith("/")) mode = "files";
    else if (input.startsWith("=")) mode = "calc";
    else if (/^(minuteur|timer)(?:\s|$)/i.test(input)) mode = "timer";
    else if (/^(?:@\s*)?(?:artisan|stardew)(?:\s|$)/i.test(input)) mode = "games";
    else mode = "apps";
    const s = strings[language];
    category.textContent = s[mode];
    for (const tab of tabs) { const active = tab.dataset.demo === mode; tab.classList.toggle("is-active", active); tab.setAttribute("aria-pressed", String(active)); }
    results.replaceChildren();
    if (mode === "apps" || mode === "games") {
      const term = lower.replace(/^[@+]\s*/, "");
      const examples = [{ name: "Obsidian", icon: "obsidian", installed: true }, { name: "Steam", icon: "◉", installed: true }, { name: "Artisan TD", icon: "game", installed: true, game: true }, { name: "Stardew Valley", icon: "game", installed: true, game: true }, { name: "VLC media player", icon: "△", installed: true }, { name: "Google Chrome", icon: "↓", installed: false }];
      const matches = examples.filter(app => app.name.toLowerCase().includes(term) && (!input.startsWith("+") || !app.installed) && (!input.startsWith("@") || app.installed));
      for (const [index, app] of matches.entries()) makeResult(app.name, app.game ? s.steamGame : app.installed ? s.app : s.install, app.icon, index === 0);
      if (matches.length === 1 && matches[0].installed && !matches[0].game && !input.startsWith("@")) makeResult(`${language === "en" ? "Uninstall" : "Désinstaller"} ${matches[0].name}`, s.remove, "−");
      note(matches.length ? (mode === "games" ? s.steamDemo : s.demo) : s.empty, !matches.length);
    } else if (mode === "files") {
      const term = lower.replace(/^\/\s*/, "");
      const names = language === "en" ? ["Project notes.md", "Projects", "Project sketch.pdf"] : ["Notes du projet.md", "Projets", "Croquis du projet.pdf"];
      const examples = names.map((name, index) => ({ name, folder: index === 1 }));
      const matches = examples.filter(file => file.name.toLowerCase().includes(term));
      matches.forEach((file, index) => makeResult(file.name, file.folder ? s.folder : s.doc, file.folder ? "▱" : "≡", index === 0));
      note(matches.length ? s.demo : s.empty, !matches.length);
    } else if (mode === "timer") {
      const duration = lower.match(/(?:minuteur|timer)\s+(\d{1,3})m(?:\s|$)/);
      if (duration && Number(duration[1]) > 0) { makeResult(`${duration[1]} ${s.minutes}`, s.startTimer, "◷", true); note(s.timerHelp); }
      else note(s.timerInvalid, true);
    } else {
      const value = calculate(input.replace(/^=\s*/, ""));
      if (value !== null) { makeResult(String(value), s.copy, "=", true); note(s.calcHelp); }
      else note(s.calcInvalid, true);
    }
  }

  languageButton.addEventListener("click", () => setLanguage(language === "fr" ? "en" : "fr"));
  query.addEventListener("input", render);
  query.addEventListener("keydown", event => { if (event.key === "Enter") event.preventDefault(); });
  for (const tab of tabs) tab.addEventListener("click", () => {
    const values = { apps: "obsidian", games: "artisan", files: language === "en" ? "/ project" : "/ projet", timer: language === "en" ? "timer 5m" : "minuteur 5m", calc: "= (15 + 5) * 3" };
    query.value = values[tab.dataset.demo]; render();
  });
  render();
})();

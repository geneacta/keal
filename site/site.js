// Keal site — the small shared behaviors the handoff asks for:
// language toggle (persisted), copy-to-clipboard, perf bars animating in,
// tour progress (persisted) and per-block "Run" reveals.

(function () {
  // ---- language toggle -------------------------------------------------
  var FR = {
    pill: "Auto-hébergé — le point fixe du bootstrap est vérifié à chaque exécution",
    h1: "Le langage qui se compile lui-même.",
    sub: "Typé statiquement, sûr face à null, la silhouette de Kotlin sur une syntaxe famille C. Trois moteurs d'accord sur chaque octet — et un compilateur écrit en Keal, compilé par lui-même.",
    cta1: "Commencer le tour →",
    cta2: "Lire la doc",
    c1h: "Le compilateur est écrit en Keal",
    c1p: "Lexeur, parseur, vérificateur et backend C — tenus octet pour octet face à leurs oracles Rust, reproduisant leur propre source.",
    c2h: "est un autre type",
    c2p: "Inférence partout, narrowing qui tient à travers and, les retours anticipés — même implies.",
    c3h: "Six langages, un fichier",
    ioh: "Un fichier, six langages.",
    iop1: "Les quatre natifs rencontrent Keal sur l'ABI C que ses binaires parlent déjà — pas de runtime, pas de couche de conversion. Java et Kotlin passent par un module passerelle, écrit en Keal.",
    iop2: "transforme n'importe quel en-tête C en déclarations",
    iop3: "vérifiées — une staticlib Rust, une c-archive Go, ou celui de sqlite.",
    perfcap: "fib(35) — le même programme, sur les trois moteurs.",
    teh: "×84 en natif — avec les mêmes garanties.",
    tep1: "L'interpréteur arborescent est la spécification. La VM à bytecode est le défaut. keal build compile via C11 vers un vrai exécutable.",
    tep2: "La suite de tests exécute chaque programme sur les trois et exige une sortie identique à l'octet — le débordement entier panique toujours, les bornes restent vérifiées.",
    gsh: "Opérationnel en une minute.",
    gsl: "Puis faites le tour — 30 minutes, chaque extrait s'exécute →",
    base: "Un langage de programmation typé statiquement et auto-hébergé. Construit par Geneacta.",
    "f-tour": "Le tour", "f-docs": "Docs & référence", "f-ex": "Exemples",
    "f-mem": "Modèle mémoire", "f-th": "Threads & acteurs", "f-de": "deinit déterministe",
    "f-vs": "Extension VS Code", "f-co": "Contribuer"
  };
  var EN = {};
  document.querySelectorAll("[data-i18n]").forEach(function (el) {
    EN[el.getAttribute("data-i18n")] = el.innerHTML;
  });

  function setLang(lang) {
    var dict = lang === "fr" ? FR : EN;
    document.querySelectorAll("[data-i18n]").forEach(function (el) {
      var k = el.getAttribute("data-i18n");
      if (dict[k] !== undefined) el.innerHTML = dict[k];
    });
    document.querySelectorAll(".lang button").forEach(function (b) {
      b.classList.toggle("on", b.getAttribute("data-lang") === lang);
    });
    try { localStorage.setItem("keal-lang", lang); } catch (e) {}
    document.documentElement.lang = lang;
  }
  document.querySelectorAll(".lang button").forEach(function (b) {
    b.addEventListener("click", function () { setLang(b.getAttribute("data-lang")); });
  });
  try {
    var saved = localStorage.getItem("keal-lang");
    if (saved === "fr") setLang("fr");
  } catch (e) {}

  // ---- copy the clone line --------------------------------------------
  var clone = document.getElementById("clone");
  if (clone) {
    clone.addEventListener("click", function () {
      var text = "git clone https://github.com/geneacta/keal && cd keal && ./bootstrap.sh";
      var done = function () {
        var cp = document.getElementById("clone-cp");
        if (cp) { cp.textContent = "copied"; setTimeout(function () { cp.textContent = "⧉"; }, 1400); }
      };
      if (navigator.clipboard) navigator.clipboard.writeText(text).then(done);
    });
  }
  document.querySelectorAll("[data-copy]").forEach(function (el) {
    el.addEventListener("click", function () {
      if (navigator.clipboard) navigator.clipboard.writeText(el.getAttribute("data-copy"));
      el.textContent = "copied";
      setTimeout(function () { el.textContent = "⧉ copy"; }, 1400);
    });
  });

  // ---- perf bars animate on entry -------------------------------------
  var bars = document.querySelectorAll(".bar > div[data-w]");
  if (bars.length && "IntersectionObserver" in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (en.isIntersecting) {
          en.target.style.width = en.target.getAttribute("data-w");
          io.unobserve(en.target);
        }
      });
    }, { threshold: 0.4 });
    bars.forEach(function (b) { io.observe(b); });
  } else {
    bars.forEach(function (b) { b.style.width = b.getAttribute("data-w"); });
  }

  // ---- tour: run buttons + progress -----------------------------------
  document.querySelectorAll(".run").forEach(function (btn) {
    btn.addEventListener("click", function () {
      btn.closest(".cwin").classList.add("ran");
    });
  });

  var chapters = document.querySelectorAll("[data-chapter]");
  if (chapters.length) {
    var doneSet;
    try { doneSet = new Set(JSON.parse(localStorage.getItem("keal-tour") || "[]")); }
    catch (e) { doneSet = new Set(); }

    function currentChapter() {
      var h = (location.hash || "#1").slice(1);
      var n = parseInt(h, 10);
      return isNaN(n) || n < 1 || n > chapters.length ? 1 : n;
    }
    function saveDone() {
      try { localStorage.setItem("keal-tour", JSON.stringify(Array.from(doneSet))); } catch (e) {}
    }
    function render() {
      var cur = currentChapter();
      chapters.forEach(function (c) {
        c.style.display = parseInt(c.getAttribute("data-chapter"), 10) === cur ? "" : "none";
      });
      document.querySelectorAll(".tch").forEach(function (item, i) {
        var n = i + 1;
        item.classList.toggle("now", n === cur);
        item.classList.toggle("done", n !== cur && doneSet.has(n));
        var badge = item.querySelector(".n");
        if (badge) badge.textContent = (n !== cur && doneSet.has(n)) ? "✓" : String(n);
      });
      var cnt = document.getElementById("tcount");
      if (cnt) cnt.textContent = cur + " / " + chapters.length;
      var fill = document.getElementById("tfill");
      if (fill) fill.style.width = Math.round(100 * cur / chapters.length) + "%";
      window.scrollTo(0, 0);
    }
    document.querySelectorAll(".tch").forEach(function (item, i) {
      item.addEventListener("click", function () { location.hash = "#" + (i + 1); });
    });
    document.querySelectorAll("[data-goto]").forEach(function (b) {
      b.addEventListener("click", function (ev) {
        ev.preventDefault();
        doneSet.add(currentChapter());
        saveDone();
        location.hash = "#" + b.getAttribute("data-goto");
      });
    });
    window.addEventListener("hashchange", render);
    render();
  }
})();

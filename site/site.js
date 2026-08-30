// The site's behaviour, such as it is: copy buttons, the performance bars
// animating when they come into view, and the tour's "Run" reveals.
// Language is not a toggle here — English and French are separate pages,
// so every URL says which language it is and can be linked to as such.

(function () {
  // ---- copy-to-clipboard on any code window -----------------------------
  document.querySelectorAll(".cwin").forEach(function (win) {
    var bar = win.querySelector(".cwin-bar");
    var pre = win.querySelector("pre");
    if (!bar || !pre || bar.querySelector(".copy")) return;
    var btn = document.createElement("span");
    btn.className = "copy";
    btn.textContent = "⧉";
    btn.title = "Copy";
    btn.addEventListener("click", function () {
      var text = pre.innerText;
      var done = function () {
        btn.textContent = "✓";
        setTimeout(function () { btn.textContent = "⧉"; }, 1400);
      };
      if (navigator.clipboard) navigator.clipboard.writeText(text).then(done, function () {});
    });
    bar.appendChild(btn);
  });

  // ---- the performance bars fill when they are scrolled to --------------
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

  // ---- the tour: a chapter's output is revealed, not claimed ------------
  document.querySelectorAll(".run").forEach(function (btn) {
    var win = btn.closest(".cwin");
    var out = win && win.querySelector(".cwin-out");
    if (!out) return;
    out.style.display = "none";
    btn.style.cursor = "pointer";
    btn.addEventListener("click", function () {
      var shown = out.style.display !== "none";
      out.style.display = shown ? "none" : "";
      btn.classList.toggle("on", !shown);
    });
  });

  // ---- highlight the table-of-contents entry you are reading ------------
  var entries = document.querySelectorAll(".dtoc-items a");
  if (entries.length && "IntersectionObserver" in window) {
    var byId = {};
    entries.forEach(function (a) { byId[a.getAttribute("href").slice(1)] = a; });
    var heads = [];
    Object.keys(byId).forEach(function (id) {
      var h = document.getElementById(id);
      if (h) heads.push(h);
    });
    var spy = new IntersectionObserver(function (es) {
      es.forEach(function (e) {
        if (!e.isIntersecting) return;
        entries.forEach(function (a) { a.classList.remove("on"); });
        var a = byId[e.target.id];
        if (a) a.classList.add("on");
      });
    }, { rootMargin: "-10% 0px -80% 0px" });
    heads.forEach(function (h) { spy.observe(h); });
  }
})();

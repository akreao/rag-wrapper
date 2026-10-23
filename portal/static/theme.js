// The top bar's light/dark switch. Without a choice saved in this browser
// the portal follows the system setting (style.css); a choice is kept in
// localStorage and put on <html> as data-theme before the page draws.
(function () {
  var root = document.documentElement;
  var key = "portal-theme";
  try {
    var saved = localStorage.getItem(key);
    if (saved === "light" || saved === "dark") root.setAttribute("data-theme", saved);
  } catch (e) {}

  function current() {
    var t = root.getAttribute("data-theme");
    if (t) return t;
    return window.matchMedia && matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  function label(button) {
    button.setAttribute("aria-label", current() === "dark" ? "Switch to the light theme" : "Switch to the dark theme");
    button.title = button.getAttribute("aria-label");
  }

  document.addEventListener("DOMContentLoaded", function () {
    var button = document.getElementById("theme-toggle");
    if (!button) return;
    button.hidden = false;
    label(button);
    button.addEventListener("click", function () {
      var next = current() === "dark" ? "light" : "dark";
      root.setAttribute("data-theme", next);
      try { localStorage.setItem(key, next); } catch (e) {}
      label(button);
    });
  });
})();

// The Light/Dark choice from Settings > App. Loaded before the stylesheet
// applies, so the window doesn't flash the other theme. With no choice saved,
// the page follows the desktop's setting.
(function () {
  function apply(theme) {
    if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
    else delete document.documentElement.dataset.theme;
  }
  let saved = null;
  try {
    saved = localStorage.getItem("theme");
  } catch {}
  apply(saved);
  // Changing it in the main window updates the Battle config window too.
  window.addEventListener("storage", (e) => {
    if (e.key === "theme") apply(e.newValue);
  });
  window.setTheme = function (theme) {
    try {
      if (theme === "light" || theme === "dark") localStorage.setItem("theme", theme);
      else localStorage.removeItem("theme");
    } catch {}
    apply(theme);
  };
})();

// Remember the language a visitor picks, so the English page stops redirecting Russian browsers.
for (const link of document.querySelectorAll("a[data-lang]")) {
  link.addEventListener("click", () => {
    try {
      localStorage.setItem("aeria-lang", link.dataset.lang);
    } catch (e) {}
  });
}

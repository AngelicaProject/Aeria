// Copy buttons: data-copy holds the text, data-done the confirmation label.
for (const button of document.querySelectorAll("button[data-copy]")) {
  const label = button.textContent;
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(button.dataset.copy);
    } catch (e) {
      return;
    }
    button.textContent = button.dataset.done || label;
    setTimeout(() => (button.textContent = label), 2000);
  });
}

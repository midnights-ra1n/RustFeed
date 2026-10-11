// Dark by default; applies a saved light preference before first paint.
try {
  if (localStorage.getItem("rustfeed-theme") === "light") document.documentElement.dataset.theme = "light";
} catch (_) {}

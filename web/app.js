"use strict";

const $ = (sel) => document.querySelector(sel);

const els = {
  list: $("#feed-list"),
  template: $("#feed-template"),
  empty: $("#empty"),
  noMatch: $("#no-match"),
  filter: $("#filter"),
  addForm: $("#add-form"),
  addUrl: $("#add-url"),
  addBtn: $("#add-btn"),
  addFeedback: $("#add-feedback"),
  statCount: $("#stat-count"),
  statOk: $("#stat-ok"),
  statInterval: $("#stat-interval"),
  dialog: $("#confirm-dialog"),
  confirmUrl: $("#confirm-url"),
  toasts: $("#toasts"),
  themeToggle: $("#theme-toggle"),
};

let feeds = [];
// url -> { state: "pending" | "ok" | "error", title, items, error }
const checks = new Map();

/* ---------- API ---------- */

async function api(method, path, body) {
  const res = await fetch(path, {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error || `Erreur HTTP ${res.status}`);
  return data;
}

function applyList(data) {
  feeds = data.feeds;
  els.statInterval.textContent = formatInterval(data.interval);
  render();
}

async function load() {
  try {
    applyList(await api("GET", "/api/feeds"));
    feeds.forEach((url) => checkFeed(url));
  } catch (err) {
    toast(`Impossible de charger les flux : ${err.message}`, true);
  }
}

async function checkFeed(url, force = false) {
  if (!force && checks.has(url)) return checks.get(url);
  checks.set(url, { state: "pending" });
  updateRow(url);
  let result;
  try {
    const data = await api("POST", "/api/check", { url });
    result = { state: "ok", title: data.title, items: data.items };
  } catch (err) {
    result = { state: "error", error: err.message };
  }
  checks.set(url, result);
  updateRow(url);
  updateStats();
  return result;
}

/* ---------- Rendering ---------- */

function render() {
  els.list.replaceChildren(...feeds.map(buildRow));
  applyFilter();
  updateStats();
}

function buildRow(url) {
  const li = els.template.content.firstElementChild.cloneNode(true);
  li.dataset.url = url;
  li.querySelector(".act-edit").addEventListener("click", () => startEdit(li, url));
  li.querySelector(".act-delete").addEventListener("click", () => confirmDelete(url));
  fillRow(li, url);
  return li;
}

function fillRow(li, url) {
  const check = checks.get(url) || { state: "pending" };
  const status = li.querySelector(".status");
  const title = li.querySelector(".feed-title");
  const link = li.querySelector(".feed-url");
  const badge = li.querySelector(".badge");

  status.className = `status ${check.state === "pending" ? "" : check.state}`;
  link.href = url;
  link.textContent = url;
  title.replaceChildren();

  if (check.state === "ok") {
    status.title = "Flux valide";
    title.textContent = check.title || hostname(url);
    badge.hidden = false;
    badge.textContent = `${check.items} article${check.items > 1 ? "s" : ""}`;
  } else if (check.state === "error") {
    status.title = check.error;
    title.append(hostname(url), " ");
    const err = document.createElement("span");
    err.className = "err";
    err.textContent = "· flux injoignable ou invalide";
    err.title = check.error;
    title.append(err);
    badge.hidden = true;
  } else {
    status.title = "Vérification…";
    title.textContent = hostname(url);
    badge.hidden = true;
  }
}

function rowFor(url) {
  return [...els.list.children].find((li) => li.dataset.url === url);
}

function updateRow(url) {
  const li = rowFor(url);
  if (li && !li.classList.contains("editing")) fillRow(li, url);
}

function updateStats() {
  els.statCount.textContent = feeds.length;
  const ok = feeds.filter((u) => checks.get(u)?.state === "ok").length;
  const pending = feeds.some((u) => (checks.get(u)?.state ?? "pending") === "pending");
  els.statOk.textContent = pending && ok === 0 && feeds.length ? "…" : `${ok}/${feeds.length}`;
  els.empty.hidden = feeds.length !== 0;
}

function applyFilter() {
  const q = els.filter.value.trim().toLowerCase();
  let visible = 0;
  for (const li of els.list.children) {
    const url = li.dataset.url;
    const title = checks.get(url)?.title || "";
    const match = !q || url.toLowerCase().includes(q) || title.toLowerCase().includes(q);
    li.hidden = !match;
    if (match) visible++;
  }
  els.noMatch.hidden = !(feeds.length && q && visible === 0);
}

/* ---------- Add ---------- */

els.addForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const url = els.addUrl.value.trim();
  if (!url) return;
  hideFeedback();

  if (feeds.includes(url)) {
    showFeedback("Ce flux est déjà dans la liste.");
    return;
  }

  setBusy(els.addBtn, true, "Vérification…");
  const check = await checkFeed(url, true);
  if (check.state === "ok") {
    await addFeed(url);
  } else {
    showFeedback(`Ce flux ne semble pas valide : ${check.error}`, () => addFeed(url));
  }
  setBusy(els.addBtn, false);
});

async function addFeed(url) {
  hideFeedback();
  setBusy(els.addBtn, true, "Ajout…");
  try {
    applyList(await api("POST", "/api/feeds", { url }));
    els.addUrl.value = "";
    const title = checks.get(url)?.title;
    toast(title ? `« ${title} » ajouté` : "Flux ajouté");
  } catch (err) {
    showFeedback(err.message);
  } finally {
    setBusy(els.addBtn, false);
  }
}

function showFeedback(message, forceAction) {
  const box = els.addFeedback;
  box.replaceChildren();
  const msg = document.createElement("span");
  msg.className = "msg";
  msg.textContent = message;
  box.append(msg);
  if (forceAction) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "btn btn-ghost btn-sm";
    btn.textContent = "Ajouter quand même";
    btn.addEventListener("click", forceAction);
    box.append(btn);
  }
  box.hidden = false;
}

function hideFeedback() {
  els.addFeedback.hidden = true;
}

/* ---------- Edit ---------- */

function startEdit(li, url) {
  li.classList.add("editing");
  const row = document.createElement("div");
  row.className = "edit-row";

  const input = document.createElement("input");
  input.className = "edit-input";
  input.type = "url";
  input.value = url;
  input.setAttribute("aria-label", "Nouvelle URL du flux");

  const save = document.createElement("button");
  save.type = "button";
  save.className = "btn btn-primary btn-sm";
  save.textContent = "Enregistrer";

  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.className = "btn btn-ghost btn-sm";
  cancel.textContent = "Annuler";

  const restore = () => li.replaceWith(buildRow(url));

  const submit = async () => {
    const next = input.value.trim();
    if (!next || next === url) return restore();
    setBusy(save, true, "…");
    try {
      applyList(await api("PUT", "/api/feeds", { old_url: url, url: next }));
      checks.delete(url);
      checkFeed(next, true);
      toast("Flux modifié");
    } catch (err) {
      toast(err.message, true);
      setBusy(save, false);
      input.focus();
    }
  };

  save.addEventListener("click", submit);
  cancel.addEventListener("click", restore);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); submit(); }
    if (e.key === "Escape") restore();
  });

  row.append(input, save, cancel);
  li.replaceChildren(row);
  input.focus();
  input.select();
}

/* ---------- Delete ---------- */

function confirmDelete(url) {
  els.confirmUrl.textContent = url;
  els.dialog.returnValue = "";
  els.dialog.showModal();
  els.dialog.addEventListener("close", async () => {
    if (els.dialog.returnValue !== "confirm") return;
    try {
      applyList(await api("DELETE", "/api/feeds", { url }));
      checks.delete(url);
      toast("Flux supprimé");
    } catch (err) {
      toast(err.message, true);
    }
  }, { once: true });
}

/* ---------- Helpers ---------- */

function setBusy(btn, busy, label) {
  btn.disabled = busy;
  if (!btn.dataset.label) btn.dataset.label = btn.textContent.trim();
  if (busy) {
    const spin = document.createElement("span");
    spin.className = "spinner";
    btn.replaceChildren(spin, label || btn.dataset.label);
  } else {
    btn.replaceChildren(btn.dataset.label);
  }
}

function toast(message, isError = false) {
  const el = document.createElement("div");
  el.className = `toast${isError ? " error" : ""}`;
  el.textContent = message;
  els.toasts.append(el);
  setTimeout(() => {
    el.classList.add("leaving");
    setTimeout(() => el.remove(), 300);
  }, isError ? 5000 : 2800);
}

function hostname(url) {
  try { return new URL(url).hostname.replace(/^www\./, ""); } catch { return url; }
}

function formatInterval(secs) {
  if (!Number.isFinite(secs)) return "–";
  if (secs < 60) return `${secs} s`;
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  if (h === 0) return `${m} min`;
  return m ? `${h} h ${m}` : `${h} h`;
}

/* ---------- Theme ---------- */

function currentTheme() {
  const forced = document.documentElement.dataset.theme;
  if (forced) return forced;
  return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

els.themeToggle.addEventListener("click", () => {
  const next = currentTheme() === "dark" ? "light" : "dark";
  document.documentElement.dataset.theme = next;
  try { localStorage.setItem("rustfeed-theme", next); } catch (_) {}
});

els.filter.addEventListener("input", applyFilter);

load();

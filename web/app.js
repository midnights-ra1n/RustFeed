"use strict";

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => [...document.querySelectorAll(sel)];

const INTERVALS = [
  [300, "5 min"], [900, "15 min"], [1800, "30 min"], [3600, "1 h"], [10800, "3 h"],
  [21600, "6 h"], [43200, "12 h"], [86400, "24 h"],
];
const MAIL_FREQUENCIES = [
  [0, "À chaque nouveauté"], [900, "Toutes les 15 min"], [3600, "Toutes les heures"], [10800, "Toutes les 3 h"],
  [21600, "Toutes les 6 h"], [43200, "Deux fois par jour"], [86400, "Une fois par jour"], [604800, "Une fois par semaine"],
];
const VIEW_TITLES = { reader: "Lecture", feeds: "Flux", delivery: "Envois", account: "Compte" };

const store = {
  feeds: [],
  defaultInterval: 1800,
  items: [],
  filter: "unread",
  feedFilter: "",
  current: null,
  settings: null,
  defaults: null,
};

/* ================================================================ API */

async function api(method, path, body) {
  const res = await fetch(path, {
    method,
    credentials: "same-origin",
    headers: body !== undefined ? { "Content-Type": "application/json" } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({}));
  if (res.status === 401 && !path.startsWith("/api/auth/")) {
    showAuth();
  }
  if (!res.ok) throw new Error(data.error || `Erreur HTTP ${res.status}`);
  return data;
}

/* ================================================================ Auth */

async function boot() {
  let status;
  try {
    status = await api("GET", "/api/auth/status");
  } catch (err) {
    toast(`Serveur injoignable : ${err.message}`, true);
    return;
  }
  if (status.authenticated) return startApp();
  showAuth(status);
}

async function showAuth(status) {
  status = status || (await api("GET", "/api/auth/status").catch(() => ({})));
  $("#app").hidden = true;
  $("#auth").hidden = false;
  $("#setup-form").hidden = !status.setup_required;
  $("#login-form").hidden = !!status.setup_required;
  $("#passkey-login").hidden = !(status.passkeys && window.PublicKeyCredential);
  (status.setup_required ? $("#setup-form [name=code]") : $("#login-form [name=username]")).focus();
}

function authError(msg) {
  const el = $("#auth-error");
  el.textContent = msg || "";
  el.hidden = !msg;
}

$("#login-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  authError();
  const f = e.target;
  const btn = f.querySelector("[type=submit]");
  setBusy(btn, true);
  try {
    await api("POST", "/api/auth/login", { username: f.username.value, password: f.password.value });
    f.reset();
    startApp();
  } catch (err) {
    authError(err.message);
  } finally {
    setBusy(btn, false);
  }
});

$("#setup-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  authError();
  const f = e.target;
  if (f.password.value !== f.confirm.value) return authError("Les mots de passe ne correspondent pas.");
  const btn = f.querySelector("[type=submit]");
  setBusy(btn, true);
  try {
    await api("POST", "/api/auth/setup", { code: f.code.value, username: f.username.value, password: f.password.value });
    f.reset();
    await startApp();
    location.hash = "#account";
    toast("Compte créé. Ajoutez une passkey pour vous connecter plus vite.");
  } catch (err) {
    authError(err.message);
  } finally {
    setBusy(btn, false);
  }
});

$("#passkey-login-btn").addEventListener("click", async () => {
  authError();
  try {
    const { id, options } = await api("POST", "/api/auth/passkey/start", {});
    const cred = await navigator.credentials.get({ publicKey: decodeRequest(options.publicKey) });
    await api("POST", "/api/auth/passkey/finish", { id, credential: encodeAssertion(cred) });
    startApp();
  } catch (err) {
    if (err.name !== "NotAllowedError") authError(err.message);
  }
});

/* ---------- WebAuthn (base64url <-> ArrayBuffer) ---------- */

function b64uToBuf(s) {
  const bin = atob(s.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(s.length / 4) * 4, "="));
  return Uint8Array.from(bin, (c) => c.charCodeAt(0)).buffer;
}

function bufToB64u(buf) {
  let bin = "";
  for (const b of new Uint8Array(buf)) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function decodeCreation(pk) {
  return {
    ...pk,
    challenge: b64uToBuf(pk.challenge),
    user: { ...pk.user, id: b64uToBuf(pk.user.id) },
    excludeCredentials: (pk.excludeCredentials || []).map((c) => ({ ...c, id: b64uToBuf(c.id) })),
  };
}

function decodeRequest(pk) {
  return {
    ...pk,
    challenge: b64uToBuf(pk.challenge),
    allowCredentials: (pk.allowCredentials || []).map((c) => ({ ...c, id: b64uToBuf(c.id) })),
  };
}

function encodeAttestation(cred) {
  return {
    id: cred.id,
    rawId: bufToB64u(cred.rawId),
    type: cred.type,
    extensions: cred.getClientExtensionResults(),
    response: {
      attestationObject: bufToB64u(cred.response.attestationObject),
      clientDataJSON: bufToB64u(cred.response.clientDataJSON),
      transports: cred.response.getTransports ? cred.response.getTransports() : undefined,
    },
  };
}

function encodeAssertion(cred) {
  const r = cred.response;
  return {
    id: cred.id,
    rawId: bufToB64u(cred.rawId),
    type: cred.type,
    extensions: cred.getClientExtensionResults(),
    response: {
      authenticatorData: bufToB64u(r.authenticatorData),
      clientDataJSON: bufToB64u(r.clientDataJSON),
      signature: bufToB64u(r.signature),
      userHandle: r.userHandle ? bufToB64u(r.userHandle) : null,
    },
  };
}

/* ================================================================ Shell */

let started = false;

async function startApp() {
  $("#auth").hidden = true;
  $("#app").hidden = false;
  authError();
  route();
  await Promise.all([loadFeeds(), loadItems(), loadSettings(), loadAccount()]).catch((err) => toast(err.message, true));
  loadUpdate();
  if (!started) {
    started = true;
    // Picks up new articles while the app stays open, even in a background (pinned) tab,
    // and catches up at once when the tab comes back (browsers throttle or freeze hidden tabs).
    const sync = () => { if (!$("#app").hidden) Promise.all([loadItems(), loadFeeds()]).catch(() => {}); };
    setInterval(sync, 60000);
    document.addEventListener("visibilitychange", () => { if (!document.hidden) sync(); });
  }
}

function route() {
  const view = (location.hash || "#reader").slice(1);
  const name = VIEW_TITLES[view] ? view : "reader";
  for (const v of Object.keys(VIEW_TITLES)) $(`#view-${v}`).hidden = v !== name;
  $$(".nav-item").forEach((a) => a.classList.toggle("active", a.dataset.view === name));
  $("#view-title").textContent = VIEW_TITLES[name];
  if (name !== "reader") closeArticle();
}

window.addEventListener("hashchange", route);

$("#refresh-btn").addEventListener("click", async () => {
  const btn = $("#refresh-btn");
  btn.classList.add("spinning");
  try {
    await api("POST", "/api/refresh", {});
    // The server checks feeds in the background: give it a moment.
    await new Promise((r) => setTimeout(r, 4000));
    await Promise.all([loadItems(), loadFeeds()]);
    toast("Flux relevés");
  } catch (err) {
    toast(err.message, true);
  } finally {
    btn.classList.remove("spinning");
  }
});

$("#theme-toggle").addEventListener("click", () => {
  const light = document.documentElement.dataset.theme !== "light";
  if (light) document.documentElement.dataset.theme = "light";
  else delete document.documentElement.dataset.theme;
  try { localStorage.setItem("rustfeed-theme", light ? "light" : "dark"); } catch (_) {}
  $('meta[name="theme-color"]').content = light ? "#fbf7f4" : "#120e0d";
  if (store.current) renderArticleFrame();
});

/* ================================================================ Reader */

async function loadItems() {
  const data = await api("GET", "/api/items");
  store.items = data.items;
  renderItems();
}

// store.feedFilter is "", "feed:<url>", "folder:<name>" or "tag:<name>".
function matchesFeedFilter(item) {
  const f = store.feedFilter;
  if (!f) return true;
  const [kind, value] = [f.slice(0, f.indexOf(":")), f.slice(f.indexOf(":") + 1)];
  if (kind === "feed") return item.feed === value;
  const feed = store.feeds.find((x) => x.url === item.feed);
  return kind === "folder" ? feed?.folder === value : !!feed?.tags.includes(value);
}

function visibleItems() {
  return store.items.filter((i) =>
    (store.filter === "all" || !i.read || i.link === store.current?.link) && matchesFeedFilter(i));
}

function renderItems() {
  const list = visibleItems();
  $("#items").replaceChildren(...list.slice(0, 200).map(itemRow));
  $("#items-empty").hidden = list.length !== 0;
  const unread = store.items.filter((i) => !i.read).length;
  $("#unread-badge").hidden = unread === 0;
  $("#unread-badge").textContent = unread > 99 ? "99+" : unread;
  if (navigator.setAppBadge) (unread ? navigator.setAppBadge(unread) : navigator.clearAppBadge()).catch(() => {});
}

function itemRow(item) {
  const li = document.createElement("li");
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = `item${item.read ? "" : " unread"}${store.current?.link === item.link ? " selected" : ""}`;
  btn.innerHTML = `<div class="item-meta"><span class="eyebrow"></span><time></time></div><div class="item-title"></div><div class="item-summary"></div>`;
  btn.querySelector(".eyebrow").textContent = item.feed_title;
  btn.querySelector("time").textContent = timeAgo(itemDate(item));
  btn.querySelector(".item-title").textContent = item.title;
  btn.querySelector(".item-summary").textContent = item.summary;
  btn.addEventListener("click", () => openArticle(item));
  li.append(btn);
  return li;
}

async function openArticle(item) {
  store.current = { ...item, content: null };
  item.read = true;
  $("#article .article-empty").hidden = true;
  $("#article .article-body").hidden = false;
  $("#article-feed").textContent = item.feed_title;
  $("#article-title").textContent = item.title;
  $("#article-date").textContent = formatDate(itemDate(item));
  $("#article-open").href = /^https?:\/\//.test(item.link) ? item.link : "#";
  $(".reader").classList.add("open");
  renderItems();
  renderArticleFrame();
  try {
    const data = await api("POST", "/api/items/content", { link: item.link });
    if (store.current?.link !== item.link) return;
    store.current.content = data.content;
    renderArticleFrame();
  } catch (err) {
    toast(err.message, true);
  }
}

function renderArticleFrame() {
  const s = getComputedStyle(document.documentElement);
  const color = s.getPropertyValue("--text").trim();
  const muted = s.getPropertyValue("--muted").trim();
  const accent = s.getPropertyValue("--accent-text").trim();
  const content = store.current?.content ?? `<p style="color:${muted}">Chargement…</p>`;
  // Sandboxed (no scripts, no same-origin): the feed's HTML can't touch the app.
  // The original page's address resolves relative links and images of the article.
  const base = /^https?:\/\//.test(store.current?.link || "") ? ` href="${store.current.link.replace(/"/g, "&quot;")}"` : "";
  $("#article-frame").srcdoc = `<!doctype html><html><head><meta charset="utf-8"><base${base} target="_blank">
<style>
  html{color-scheme:${document.documentElement.dataset.theme === "light" ? "light" : "dark"}}
  body{margin:0;padding:4px 24px 32px;font:16px/1.55 ui-serif,Georgia,serif;-webkit-font-smoothing:antialiased;color:${color};background:transparent;overflow-wrap:anywhere}
  a{color:${accent}} img,video,iframe{max-width:100%;height:auto;border-radius:10px} figure{margin:1em 0}
  pre{overflow:auto;padding:12px;border-radius:8px;background:rgb(127 127 127/.12)} blockquote{margin:1em 0;padding-left:1em;border-left:3px solid ${accent};color:${muted}}
  h1,h2,h3{font-family:system-ui,sans-serif;line-height:1.25}
</style></head><body>${content}</body></html>`;
}

function closeArticle() {
  $(".reader").classList.remove("open");
}

$("#article-back").addEventListener("click", () => {
  closeArticle();
  store.current = null;
  renderItems();
});

$("#article-unread").addEventListener("click", async () => {
  const cur = store.current;
  if (!cur) return;
  const item = store.items.find((i) => i.link === cur.link);
  if (item) item.read = false;
  renderItems();
  await api("POST", "/api/items/read", { links: [cur.link], read: false }).catch((e) => toast(e.message, true));
  toast("Marqué comme non lu");
});

$("#mark-all").addEventListener("click", async () => {
  const targets = visibleItems().filter((i) => !i.read);
  if (!targets.length) return;
  targets.forEach((i) => (i.read = true));
  renderItems();
  await api("POST", "/api/items/read", { links: targets.map((i) => i.link), read: true }).catch((e) => toast(e.message, true));
});

$$(".seg").forEach((b) => b.addEventListener("click", () => {
  $$(".seg").forEach((x) => x.classList.toggle("active", x === b));
  store.filter = b.dataset.filter;
  renderItems();
}));

$("#reader-feed").addEventListener("change", (e) => {
  store.feedFilter = e.target.value;
  renderItems();
});

// Swipe right from the article to go back on mobile.
(() => {
  let x0 = null;
  const el = $("#article");
  el.addEventListener("touchstart", (e) => { x0 = e.touches[0].clientX; }, { passive: true });
  el.addEventListener("touchend", (e) => {
    if (x0 !== null && x0 < 60 && e.changedTouches[0].clientX - x0 > 80) $("#article-back").click();
    x0 = null;
  });
})();

/* ================================================================ Feeds */

async function loadFeeds() {
  applyFeeds(await api("GET", "/api/feeds"));
}

function applyFeeds(data) {
  store.feeds = data.feeds;
  store.defaultInterval = data.interval;
  fillIntervals($("#default-interval"), data.interval);
  renderFeeds();

  const sel = $("#reader-feed");
  const prev = sel.value;
  const group = (label, options) => {
    const g = document.createElement("optgroup");
    g.label = label;
    g.append(...options);
    return g;
  };
  const folders = allFolders();
  const tags = allTags();
  const groups = [new Option("Tous les flux", "")];
  if (folders.length) groups.push(group("Dossiers", folders.map((f) => new Option(`📁 ${f}`, `folder:${f}`))));
  if (tags.length) groups.push(group("Tags", tags.map((t) => new Option(`# ${t}`, `tag:${t}`))));
  for (const [folder, feeds] of feedsByFolder()) {
    const opts = feeds.map((f) => new Option(feedName(f), `feed:${f.url}`));
    groups.push(...(folders.length ? [group(folder || "Sans dossier", opts)] : opts));
  }
  sel.replaceChildren(...groups);
  sel.value = [...sel.options].some((o) => o.value === prev) ? prev : "";
  if (sel.value !== store.feedFilter) { store.feedFilter = sel.value; renderItems(); }

  $("#folder-list").replaceChildren(...folders.map((f) => new Option(f)));
  renderExclusions();
}

const byName = (a, b) => a.localeCompare(b, "fr", { sensitivity: "base" });

function allFolders() {
  return [...new Set(store.feeds.map((f) => f.folder).filter(Boolean))].sort(byName);
}

function allTags() {
  return [...new Set(store.feeds.flatMap((f) => f.tags))].sort(byName);
}

/** [[folder, feeds]] sorted by folder name, feeds without a folder last. */
function feedsByFolder() {
  const map = new Map();
  for (const f of store.feeds) map.set(f.folder, [...(map.get(f.folder) || []), f]);
  return [...map].sort(([a], [b]) => (!a) - (!b) || byName(a, b));
}

function parseTags(value) {
  return value.split(",").map((t) => t.trim()).filter(Boolean);
}

function feedName(f) {
  return f.status?.title || hostname(f.url);
}

function fillIntervals(select, value, withDefault = false) {
  const opts = INTERVALS.map(([v, l]) => new Option(l, v));
  if (withDefault) opts.unshift(new Option(`Par défaut (${formatInterval(store.defaultInterval)})`, ""));
  if (value != null && !INTERVALS.some(([v]) => v === value)) opts.push(new Option(formatInterval(value), value));
  select.replaceChildren(...opts);
  select.value = value ?? "";
}

function renderFeeds() {
  const grouped = allFolders().length > 0;
  $("#feed-list").replaceChildren(...feedsByFolder().flatMap(([folder, feeds]) => {
    if (!grouped) return feeds.map(feedRow);
    const head = document.createElement("li");
    head.className = "folder-head";
    head.innerHTML = `<svg class="ico"><use href="#i-folder"/></svg><span></span>`;
    head.querySelector("span").textContent = folder || "Sans dossier";
    return [head, ...feeds.map(feedRow)];
  }));
  $("#feeds-empty").hidden = store.feeds.length !== 0;
  $("#feed-count").textContent = store.feeds.length;
  fillIntervals($("#add-interval"), null, true);
}

function feedRow(feed) {
  const li = document.createElement("li");
  li.className = "feed";
  const st = feed.status;
  li.innerHTML = `<span class="status"></span>
    <div class="feed-main"><div class="feed-title"></div><div class="feed-sub"></div></div>
    <div class="feed-actions">
      <button class="icon-btn act-edit" type="button" title="Modifier" aria-label="Modifier"><svg class="ico"><use href="#i-edit"/></svg></button>
      <button class="icon-btn act-delete" type="button" title="Supprimer" aria-label="Supprimer"><svg class="ico"><use href="#i-trash"/></svg></button>
    </div>`;
  li.querySelector(".status").classList.add(st ? (st.error ? "error" : "ok") : "pending");
  li.querySelector(".feed-title").textContent = feedName(feed);
  for (const t of feed.tags) {
    const tag = document.createElement("span");
    tag.className = "tag";
    tag.textContent = t;
    li.querySelector(".feed-title").append(tag);
  }

  const sub = li.querySelector(".feed-sub");
  const every = `toutes les ${formatInterval(feed.interval ?? store.defaultInterval)}`;
  if (st?.error) {
    const err = document.createElement("span");
    err.className = "err";
    err.textContent = "Injoignable · ";
    err.title = st.error;
    sub.append(err);
  } else if (st) {
    sub.append(`${st.items} articles · `);
  } else {
    sub.append("En attente · ");
  }
  const link = document.createElement("a");
  link.href = feed.url;
  link.target = "_blank";
  link.rel = "noopener noreferrer";
  link.textContent = hostname(feed.url);
  sub.append(link, ` · ${every}`);
  if (st?.last_check) sub.title = `Dernier relevé : ${formatDate(new Date(st.last_check * 1000))}`;

  li.querySelector(".act-edit").addEventListener("click", () => editFeed(li, feed));
  li.querySelector(".act-delete").addEventListener("click", async () => {
    if (!(await confirmDialog("Supprimer ce flux ?", feed.url))) return;
    try {
      applyFeeds(await api("DELETE", "/api/feeds", { url: feed.url }));
      store.items = store.items.filter((i) => i.feed !== feed.url);
      renderItems();
      toast("Flux supprimé");
    } catch (err) {
      toast(err.message, true);
    }
  });
  return li;
}

function editFeed(li, feed) {
  li.innerHTML = `<div class="feed-edit">
      <input type="url" aria-label="URL du flux">
      <select aria-label="Fréquence de vérification"></select>
      <span></span>
      <input class="f-folder" list="folder-list" placeholder="Dossier" maxlength="64" aria-label="Dossier">
      <input class="f-tags" placeholder="Tags, séparés par des virgules" aria-label="Tags">
      <div class="row"><button class="btn btn-ghost btn-sm" type="button" data-a="cancel">Annuler</button><button class="btn btn-primary btn-sm" type="button" data-a="save">Enregistrer</button></div>
    </div>`;
  const input = li.querySelector("input");
  const select = li.querySelector("select");
  const folder = li.querySelector(".f-folder");
  const tags = li.querySelector(".f-tags");
  input.value = feed.url;
  folder.value = feed.folder;
  tags.value = feed.tags.join(", ");
  fillIntervals(select, feed.interval, true);
  const restore = () => li.replaceWith(feedRow(feed));
  const save = async () => {
    const btn = li.querySelector("[data-a=save]");
    setBusy(btn, true);
    try {
      applyFeeds(await api("PUT", "/api/feeds", {
        old_url: feed.url, url: input.value.trim(), interval: numOrNull(select.value),
        folder: folder.value.trim(), tags: parseTags(tags.value),
      }));
      toast("Flux modifié");
    } catch (err) {
      toast(err.message, true);
      setBusy(btn, false);
    }
  };
  li.querySelector("[data-a=cancel]").addEventListener("click", restore);
  li.querySelector("[data-a=save]").addEventListener("click", save);
  li.querySelectorAll("input").forEach((el) => el.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); save(); }
    if (e.key === "Escape") restore();
  }));
  input.focus();
}

$("#add-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const url = $("#add-url").value.trim();
  const interval = numOrNull($("#add-interval").value);
  const btn = $("#add-btn");
  if (store.feeds.some((f) => f.url === url)) return toast("Ce flux est déjà dans la liste.", true);

  setBusy(btn, true, "Vérification…");
  try {
    let title = null;
    try {
      title = (await api("POST", "/api/check", { url })).title;
    } catch (err) {
      setBusy(btn, false);
      if (!(await confirmDialog("Ce flux semble invalide", `${err.message}\n\nL'ajouter quand même ?`, "Ajouter"))) return;
      setBusy(btn, true);
    }
    applyFeeds(await api("POST", "/api/feeds", { url, interval, folder: $("#add-folder").value.trim(), tags: parseTags($("#add-tags").value) }));
    $("#add-url").value = "";
    $("#add-tags").value = "";
    toast(title ? `« ${title} » ajouté` : "Flux ajouté");
    setTimeout(() => { loadFeeds(); loadItems(); }, 5000);
  } catch (err) {
    toast(err.message, true);
  } finally {
    setBusy(btn, false);
  }
});

$("#default-interval").addEventListener("change", async (e) => {
  if (!store.settings) return;
  try {
    store.settings = await api("PUT", "/api/settings", { ...store.settings, interval: Number(e.target.value) });
    await loadFeeds();
    toast("Fréquence par défaut enregistrée");
  } catch (err) {
    toast(err.message, true);
  }
});

/* ================================================================ Delivery */

const mf = {
  webhookEnabled: $("#webhook-enabled"), webhookUrl: $("#webhook-url"),
  enabled: $("#mail-enabled"), frequency: $("#mail-frequency"),
  host: $("#smtp-host"), port: $("#smtp-port"), security: $("#smtp-security"),
  user: $("#smtp-user"), pass: $("#smtp-pass"), from: $("#mail-from"), to: $("#mail-to"),
  subject: $("#mail-subject"), template: $("#mail-template"), item: $("#mail-item"),
  timezone: $("#mail-timezone"),
};
const DAYS = ["L", "M", "M", "J", "V", "S", "D"];
const DAY_NAMES = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];

async function loadSettings() {
  const s = await api("GET", "/api/settings");
  store.settings = s;
  if (!store.defaults) store.defaults = { subject: s.mail.subject, template: s.mail.template, item: s.mail.item_template };
  fillSettings(s);
}

function fillSettings(s) {
  mf.webhookEnabled.checked = s.webhook_enabled;
  mf.webhookUrl.value = s.webhook;
  mf.enabled.checked = s.mail.enabled;
  const freqs = MAIL_FREQUENCIES.map(([v, l]) => new Option(l, v));
  if (!MAIL_FREQUENCIES.some(([v]) => v === s.mail.frequency)) freqs.push(new Option(formatInterval(s.mail.frequency), s.mail.frequency));
  mf.frequency.replaceChildren(...freqs);
  mf.frequency.value = s.mail.frequency;
  mf.host.value = s.mail.smtp_host;
  mf.port.value = s.mail.smtp_port;
  mf.security.value = s.mail.security;
  mf.user.value = s.mail.username;
  mf.pass.value = "";
  mf.pass.placeholder = s.password_set ? "•••••••• (inchangé)" : "";
  mf.from.value = s.mail.from;
  mf.to.value = s.mail.to.join(", ");
  mf.subject.value = s.mail.subject;
  mf.template.value = s.mail.template;
  mf.item.value = s.mail.item_template;
  mf.timezone.value = s.mail.timezone || Intl.DateTimeFormat().resolvedOptions().timeZone || "";
  $("#quiet-list").replaceChildren(...s.mail.quiet_hours.map(quietRow));
  renderExclusions({ folders: s.mail.exclude_folders, tags: s.mail.exclude_tags, feeds: s.mail.exclude_feeds });
  setDirty(false);
  schedulePreview();
}

function mailFromForm() {
  return {
    enabled: mf.enabled.checked,
    smtp_host: mf.host.value.trim(),
    smtp_port: Number(mf.port.value) || 587,
    security: mf.security.value,
    username: mf.user.value.trim(),
    password: mf.pass.value,
    from: mf.from.value.trim(),
    to: mf.to.value.split(/[,;\n]/).map((t) => t.trim()).filter(Boolean),
    frequency: Number(mf.frequency.value),
    subject: mf.subject.value,
    template: mf.template.value,
    item_template: mf.item.value,
    timezone: mf.timezone.value.trim(),
    quiet_hours: $$("#quiet-list .quiet-row").map((row) => ({
      days: [...row.querySelectorAll(".day input")].flatMap((c, i) => (c.checked ? [i + 1] : [])),
      start: row.querySelector(".q-start").value,
      end: row.querySelector(".q-end").value,
    })),
    exclude_folders: checkedValues("#ex-folders"),
    exclude_tags: checkedValues("#ex-tags"),
    exclude_feeds: checkedValues("#ex-feeds"),
  };
}

function chip(value, label, checked, extraClass = "") {
  const el = document.createElement("label");
  el.className = `chip ${extraClass}`;
  el.innerHTML = `<input type="checkbox"><span></span>`;
  el.querySelector("input").value = value;
  el.querySelector("input").checked = checked;
  el.querySelector("span").textContent = label;
  return el;
}

function checkedValues(container) {
  return $$(`${container} input:checked`).map((c) => c.value);
}

function quietRow(range = { days: [1, 2, 3, 4, 5, 6, 7], start: "22:00", end: "07:00" }) {
  const row = document.createElement("div");
  row.className = "quiet-row";
  row.innerHTML = `<div class="chips" role="group" aria-label="Jours"></div>
    <input class="q-start" type="time" required aria-label="Début"> → <input class="q-end" type="time" required aria-label="Fin">
    <span class="spacer"></span>
    <button class="icon-btn" type="button" title="Supprimer la plage" aria-label="Supprimer la plage"><svg class="ico"><use href="#i-trash"/></svg></button>`;
  row.querySelector(".chips").append(...DAYS.map((d, i) => {
    const c = chip(i + 1, d, range.days.includes(i + 1), "day");
    c.title = DAY_NAMES[i];
    c.querySelector("input").setAttribute("aria-label", DAY_NAMES[i]);
    return c;
  }));
  row.querySelector(".q-start").value = range.start;
  row.querySelector(".q-end").value = range.end;
  row.querySelector("button").addEventListener("click", () => { row.remove(); setDirty(true); });
  return row;
}

$("#quiet-add").addEventListener("click", () => {
  $("#quiet-list").append(quietRow());
  setDirty(true);
});

/** Redraws the exclusion chips from the feed list, keeping what is checked (or `selected`). */
function renderExclusions(selected) {
  selected ??= { folders: checkedValues("#ex-folders"), tags: checkedValues("#ex-tags"), feeds: checkedValues("#ex-feeds") };
  const fill = (id, entries, picked) => {
    // Keep exclusions whose feed/folder/tag no longer exists, so saving doesn't silently drop them.
    const values = new Set(entries.map(([v]) => v));
    const all = [...entries, ...picked.filter((v) => !values.has(v)).map((v) => [v, v])];
    $(id).replaceChildren(...all.map(([v, l]) => chip(v, l, picked.includes(v))));
    if (!all.length) $(id).innerHTML = `<span class="muted small">Aucun pour l'instant.</span>`;
  };
  fill("#ex-folders", allFolders().map((f) => [f, f]), selected.folders);
  fill("#ex-tags", allTags().map((t) => [t, t]), selected.tags);
  fill("#ex-feeds", store.feeds.map((f) => [f.url, feedName(f)]), selected.feeds);
}

function setDirty(dirty) {
  $("#dirty-hint").hidden = !dirty;
}

$("#delivery-form").addEventListener("input", (e) => {
  setDirty(true);
  if ([mf.subject, mf.template, mf.item].includes(e.target)) schedulePreview();
});
$("#delivery-form").addEventListener("change", () => setDirty(true));

// Suggest the usual port when the security mode changes.
mf.security.addEventListener("change", () => {
  mf.port.value = { starttls: 587, tls: 465, none: 25 }[mf.security.value];
});

let previewTimer;
function schedulePreview() {
  clearTimeout(previewTimer);
  previewTimer = setTimeout(async () => {
    try {
      const p = await api("POST", "/api/settings/preview", mailFromForm());
      $("#preview-subject").textContent = p.subject;
      $("#preview-frame").srcdoc = `<!doctype html><meta charset="utf-8"><base target="_blank"><body style="margin:0">${p.html}</body>`;
    } catch (_) {}
  }, 350);
}

$$(".placeholders code").forEach((c) => c.addEventListener("click", async () => {
  try { await navigator.clipboard.writeText(c.textContent); toast(`${c.textContent} copié`); } catch (_) {}
}));

$("#mail-reset").addEventListener("click", () => {
  if (!store.defaults) return;
  mf.subject.value = store.defaults.subject;
  mf.template.value = store.defaults.template;
  mf.item.value = store.defaults.item;
  setDirty(true);
  schedulePreview();
});

$("#delivery-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const btn = e.target.querySelector("[type=submit]");
  setBusy(btn, true);
  try {
    const s = await api("PUT", "/api/settings", {
      interval: store.defaultInterval,
      webhook: mf.webhookUrl.value.trim(),
      webhook_enabled: mf.webhookEnabled.checked,
      mail: mailFromForm(),
    });
    store.settings = s;
    fillSettings(s);
    toast("Paramètres d'envoi enregistrés");
  } catch (err) {
    toast(err.message, true);
  } finally {
    setBusy(btn, false);
  }
});

$("#webhook-test").addEventListener("click", async (e) => {
  const btn = e.currentTarget;
  setBusy(btn, true);
  try {
    await api("POST", "/api/settings/test-webhook", { link: mf.webhookUrl.value.trim() });
    toast("Message de test envoyé");
  } catch (err) {
    toast(err.message, true);
  } finally {
    setBusy(btn, false);
  }
});

$("#mail-test").addEventListener("click", async (e) => {
  const btn = e.currentTarget;
  setBusy(btn, true, "Envoi…");
  try {
    await api("POST", "/api/settings/test-mail", mailFromForm());
    toast("Mail de test envoyé");
  } catch (err) {
    toast(err.message, true);
  } finally {
    setBusy(btn, false);
  }
});

/* ================================================================ Account */

async function loadAccount() {
  renderAccount(await api("GET", "/api/account"));
}

function renderAccount(acc) {
  $("#account-name").textContent = acc.username;
  $("#password-form [name=username]").value = acc.username;
  const supported = !!window.PublicKeyCredential;
  const hint = $("#passkey-hint");
  hint.hidden = acc.passkeys_available && supported && acc.public_url === location.origin;
  $("#public-url").textContent = acc.public_url;
  $("#use-origin").hidden = acc.public_url === location.origin;
  $("#behind-proxy").checked = acc.behind_proxy;
  $("#mcp-enabled").checked = acc.mcp.enabled;
  $("#auto-update").checked = acc.auto_update;
  $("#mcp-details").hidden = !acc.mcp.enabled || !acc.mcp.url;
  $("#mcp-url").value = acc.mcp.url || "";
  $("#mcp-hint").hidden = acc.mcp.https;
  $("#mcp-hint").textContent = "Disponible uniquement si l'adresse publique (Accès ci-dessus) est en HTTPS.";
  hint.textContent = !supported
    ? "Ce navigateur ne gère pas les passkeys."
    : acc.public_url !== location.origin
      ? `Passkeys liées à ${acc.public_url}. Utilisez « Utiliser l'adresse actuelle » dans Accès ci-dessous.`
      : "Passkeys indisponibles sur cette adresse : il faut un nom de domaine en HTTPS (ou localhost).";
  $("#passkey-add").hidden = !hint.hidden;

  $("#passkeys").replaceChildren(...acc.passkeys.map((p) => {
    const li = document.createElement("li");
    li.className = "passkey";
    li.innerHTML = `<svg class="ico"><use href="#i-key"/></svg><div><div class="feed-title"></div><div class="feed-sub"></div></div>
      <button class="icon-btn" type="button" aria-label="Supprimer"><svg class="ico"><use href="#i-trash"/></svg></button>`;
    li.querySelector(".feed-title").textContent = p.name;
    li.querySelector(".feed-sub").textContent = `Ajoutée le ${formatDate(new Date(p.created * 1000))}`;
    li.querySelector("button").addEventListener("click", async () => {
      if (!(await confirmDialog("Supprimer cette passkey ?", p.name))) return;
      try {
        renderAccount(await api("DELETE", "/api/account/passkey", { id: p.id }));
        toast("Passkey supprimée");
      } catch (err) {
        toast(err.message, true);
      }
    });
    return li;
  }));
  if (!acc.passkeys.length) {
    const li = document.createElement("li");
    li.className = "muted small";
    li.textContent = "Aucune passkey pour l'instant.";
    $("#passkeys").append(li);
  }
}

$("#passkey-add").addEventListener("submit", async (e) => {
  e.preventDefault();
  const btn = e.target.querySelector("[type=submit]");
  setBusy(btn, true);
  try {
    const options = await api("POST", "/api/account/passkey/start", {});
    const cred = await navigator.credentials.create({ publicKey: decodeCreation(options.publicKey) });
    renderAccount(await api("POST", "/api/account/passkey/finish", { name: $("#passkey-name").value || guessDevice(), credential: encodeAttestation(cred) }));
    $("#passkey-name").value = "";
    toast("Passkey ajoutée");
  } catch (err) {
    if (err.name !== "NotAllowedError") toast(err.message, true);
  } finally {
    setBusy(btn, false);
  }
});

async function saveAccess(useOrigin) {
  try {
    renderAccount(await api("POST", "/api/account/access", { behind_proxy: $("#behind-proxy").checked, use_current_origin: useOrigin }));
    toast("Accès enregistré");
  } catch (err) {
    toast(err.message, true);
    loadAccount().catch(() => {});
  }
}

$("#use-origin").addEventListener("click", () => saveAccess(true));

async function saveMcp(regenerate) {
  try {
    renderAccount(await api("POST", "/api/account/mcp", { enabled: $("#mcp-enabled").checked, regenerate }));
    toast(regenerate ? "Nouvelle URL générée : l'ancienne ne fonctionne plus" : "Serveur MCP enregistré");
  } catch (err) {
    toast(err.message, true);
    loadAccount().catch(() => {});
  }
}

$("#mcp-enabled").addEventListener("change", () => saveMcp(false));

async function loadUpdate() {
  try {
    const u = await api("GET", "/api/update");
    $("#update-status").textContent = u.available
      ? `Version ${u.latest} disponible (installée : ${u.current}).`
      : `Version ${u.current}, à jour.`;
    $("#update-install").hidden = !(u.available && u.can_install);
    $("#update-hint").hidden = !u.available || u.can_install;
    $("#auto-update-row").hidden = !u.can_install;
  } catch (err) {
    $("#update-status").textContent = `Impossible de vérifier les mises à jour : ${err.message}`;
  }
}

$("#auto-update").addEventListener("change", async (e) => {
  try {
    renderAccount(await api("POST", "/api/update/auto", { enabled: e.target.checked }));
    toast(e.target.checked ? "Mise à jour automatique activée" : "Mise à jour automatique désactivée");
  } catch (err) {
    toast(err.message, true);
    loadAccount().catch(() => {});
  }
});

$("#update-install").addEventListener("click", async (e) => {
  if (!(await confirmDialog("Installer la mise à jour ?", "RustFeed va redémarrer pendant l'installation.", "Installer"))) return;
  e.target.disabled = true;
  try {
    const { version } = await api("GET", "/api/account");
    await api("POST", "/api/update", {});
    $("#update-status").textContent = "Installation en cours…";
    // The server restarts once the package is installed: reload when the new version answers.
    // Sessions live in memory, so a 401 also means it restarted.
    const started = Date.now();
    const poll = setInterval(async () => {
      const res = await fetch("/api/account", { credentials: "same-origin" }).catch(() => null);
      const restarted = res && (res.status === 401 || (res.ok && (await res.json()).version !== version));
      if (restarted) { clearInterval(poll); location.reload(); }
      else if (Date.now() - started > 180000) {
        clearInterval(poll);
        e.target.disabled = false;
        $("#update-status").textContent = "La mise à jour n'a pas abouti (voir journalctl -u rustfeed-update).";
      }
    }, 3000);
  } catch (err) {
    e.target.disabled = false;
    toast(err.message, true);
  }
});
$("#mcp-regenerate").addEventListener("click", async () => {
  if (await confirmDialog("Régénérer l'URL MCP ?", "Les connecteurs utilisant l'ancienne URL devront être reconfigurés.", "Régénérer")) saveMcp(true);
});
$("#mcp-copy").addEventListener("click", async () => {
  try { await navigator.clipboard.writeText($("#mcp-url").value); toast("URL copiée"); } catch (_) { $("#mcp-url").select(); }
});
$("#behind-proxy").addEventListener("change", () => saveAccess(false));

$("#password-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = e.target;
  const btn = f.querySelector("[type=submit]");
  setBusy(btn, true);
  try {
    await api("POST", "/api/account/password", { current: f.current.value, new: f.new.value });
    f.current.value = "";
    f.new.value = "";
    toast("Mot de passe changé. Les autres sessions ont été déconnectées.");
  } catch (err) {
    toast(err.message, true);
  } finally {
    setBusy(btn, false);
  }
});

$("#logout").addEventListener("click", async () => {
  await api("POST", "/api/auth/logout", {}).catch(() => {});
  store.items = [];
  showAuth();
});

/* ================================================================ Helpers */

function confirmDialog(title, text, okLabel = "Supprimer") {
  const dlg = $("#confirm-dialog");
  $("#confirm-title").textContent = title;
  $("#confirm-text").textContent = text;
  dlg.querySelector("[value=confirm]").textContent = okLabel;
  dlg.returnValue = "";
  dlg.showModal();
  return new Promise((resolve) => dlg.addEventListener("close", () => resolve(dlg.returnValue === "confirm"), { once: true }));
}

function setBusy(btn, busy, label) {
  btn.disabled = busy;
  if (busy) {
    btn.dataset.html ??= btn.innerHTML;
    const spin = document.createElement("span");
    spin.className = "spinner";
    btn.replaceChildren(spin, label || btn.textContent.trim());
  } else if (btn.dataset.html !== undefined) {
    btn.innerHTML = btn.dataset.html;
    delete btn.dataset.html;
  }
}

function toast(message, isError = false) {
  const el = document.createElement("div");
  el.className = `toast${isError ? " error" : ""}`;
  el.textContent = message;
  $("#toasts").append(el);
  setTimeout(() => {
    el.classList.add("leaving");
    setTimeout(() => el.remove(), 300);
  }, isError ? 5500 : 2800);
}

function numOrNull(v) {
  return v === "" || v == null ? null : Number(v);
}

function hostname(url) {
  try { return new URL(url).hostname.replace(/^www\./, ""); } catch { return url; }
}

function itemDate(item) {
  const d = item.published ? new Date(item.published) : null;
  return d && !isNaN(d) ? d : new Date(item.fetched_at * 1000);
}

function timeAgo(date) {
  const s = (Date.now() - date) / 1000;
  if (s < 60) return "à l'instant";
  if (s < 3600) return `${Math.floor(s / 60)} min`;
  if (s < 86400) return `${Math.floor(s / 3600)} h`;
  if (s < 7 * 86400) return `${Math.floor(s / 86400)} j`;
  return date.toLocaleDateString("fr-FR", { day: "numeric", month: "short" });
}

function formatDate(date) {
  return date.toLocaleString("fr-FR", { dateStyle: "medium", timeStyle: "short" });
}

function formatInterval(secs) {
  if (!Number.isFinite(secs)) return "–";
  if (secs < 3600) return `${Math.round(secs / 60)} min`;
  if (secs < 86400) return `${Math.round(secs / 360) / 10} h`.replace(".", ",");
  return `${Math.round(secs / 8640) / 10} j`.replace(".", ",");
}

function guessDevice() {
  const ua = navigator.userAgent;
  if (/iPhone/.test(ua)) return "iPhone";
  if (/iPad/.test(ua)) return "iPad";
  if (/Android/.test(ua)) return "Android";
  if (/Mac/.test(ua)) return "Mac";
  if (/Windows/.test(ua)) return "Windows";
  return "Passkey";
}

/* ================================================================ Start */

boot();
if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});

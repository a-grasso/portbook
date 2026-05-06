const grid = document.getElementById("grid");
const tpl = document.getElementById("card-tpl");
const statusDot = document.getElementById("status");
const countEl = document.getElementById("count");
const tabs = document.querySelectorAll(".tab");

let currentTab = "live";
let lastSnapshot = { ports: [] };
const nodes = new Map(); // port -> { el, sig }
let emptyEl = null;

tabs.forEach((t) => {
  t.addEventListener("click", () => {
    tabs.forEach((x) => x.classList.toggle("active", x === t));
    currentTab = t.dataset.tab;
    render(lastSnapshot);
  });
});

function sig(c) {
  return [c.kind, c.url, c.reason || "", c.project_name || "", c.title || "", c.description || "", c.cmdline || c.command || ""].join("");
}

function fillCard(node, c) {
  node.href = c.url;
  node.className = `card kind-${c.kind}`;
  node.querySelector(".port").textContent = `:${c.port}`;
  const badge = node.querySelector(".kind-badge");
  if (c.kind === "live") {
    badge.style.display = "none";
    badge.className = "kind-badge";
    badge.textContent = "";
  } else {
    badge.style.display = "";
    badge.textContent = c.reason || c.kind;
    badge.className = `kind-badge badge-${c.kind}`;
  }
  node.querySelector(".project").textContent = c.project_name || "";
  node.querySelector(".title").textContent = c.title || "";
  node.querySelector(".desc").textContent = c.description || "";
  node.querySelector(".cmd").textContent = c.cmdline || c.command || "";
}

function render(snapshot) {
  lastSnapshot = snapshot;
  const all = snapshot.ports || [];
  const ports = currentTab === "live" ? all.filter((p) => p.kind === "live") : all;

  const liveCount = all.filter((p) => p.kind === "live").length;
  countEl.textContent = `${liveCount} live · ${all.length} total`;

  if (ports.length === 0) {
    for (const { el } of nodes.values()) el.remove();
    nodes.clear();
    if (!emptyEl) {
      emptyEl = document.createElement("div");
      emptyEl.className = "empty";
      grid.appendChild(emptyEl);
    }
    emptyEl.textContent = currentTab === "live"
      ? "No HTTP services discovered yet."
      : "No listening ports detected.";
    return;
  }
  if (emptyEl) { emptyEl.remove(); emptyEl = null; }

  const seen = new Set();
  let prev = null;
  for (const c of ports) {
    seen.add(c.port);
    let entry = nodes.get(c.port);
    if (!entry) {
      const el = tpl.content.firstElementChild.cloneNode(true);
      el.addEventListener("click", (e) => {
        if (!el.classList.contains("kind-live")) e.preventDefault();
      });
      fillCard(el, c);
      nodes.set(c.port, { el, sig: sig(c) });
      entry = nodes.get(c.port);
    } else {
      const s = sig(c);
      if (s !== entry.sig) {
        fillCard(entry.el, c);
        entry.sig = s;
      }
    }
    const expected = prev ? prev.nextSibling : grid.firstChild;
    if (entry.el !== expected) {
      grid.insertBefore(entry.el, expected);
    }
    prev = entry.el;
  }
  for (const [port, { el }] of nodes) {
    if (!seen.has(port)) { el.remove(); nodes.delete(port); }
  }
}

function connect() {
  const es = new EventSource("/api/stream");
  es.onopen = () => { statusDot.className = "dot live"; statusDot.title = "live"; };
  es.onmessage = (ev) => {
    try { render(JSON.parse(ev.data)); } catch (e) { console.error(e); }
  };
  es.onerror = () => {
    statusDot.className = "dot dead";
    statusDot.title = "reconnecting";
  };
}

connect();

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

const SEP = "";

function sig(c) {
  return [
    c.kind, c.url, c.reason || "", c.project_name || "",
    c.title || "", c.description || "", c.cmdline || c.command || "",
    c.status ?? "", c.elapsed_ms ?? "", c.error_class || "", c.error_detail || "",
    c.probed_url || "", c.probed_at_unix ?? "", c.attempts ?? "",
  ].join(SEP);
}

function fmtTimestamp(unix) {
  if (!unix) return "—";
  try { return new Date(unix * 1000).toISOString(); } catch (_) { return `unix ${unix}`; }
}

const DIAG_FIELDS = [
  ["pid",          (c) => c.pid],
  ["command",      (c) => c.cmdline || c.command || "—"],
  ["cwd",          (c) => c.cwd || "—"],
  ["project",      (c) => c.project_name || "—"],
  ["kind",         (c) => c.kind],
  ["reason",       (c) => c.reason || "—"],
  ["http status",  (c) => c.status ?? "—"],
  ["title",        (c) => c.title || "—"],
  ["probed url",   (c) => c.probed_url || "—"],
  ["elapsed (ms)", (c) => c.elapsed_ms ?? "—"],
  ["attempts",     (c) => c.attempts ?? "—"],
  ["error class",  (c) => c.error_class || "—"],
  ["error detail", (c) => c.error_detail || "—"],
  ["probed at",    (c) => fmtTimestamp(c.probed_at_unix)],
];

function renderDiag(node, c) {
  const dl = node.querySelector(".diag-grid");
  if (!dl) return;
  dl.innerHTML = "";
  for (const [label, get] of DIAG_FIELDS) {
    const dt = document.createElement("dt");
    dt.textContent = label;
    const dd = document.createElement("dd");
    dd.textContent = String(get(c));
    dl.append(dt, dd);
  }
}

function buildPasteReport(c) {
  const lines = [`portbook explain :${c.port}`, "─────────────────────────────────────────"];
  lines.push(`port             : ${c.port}`);
  for (const [label, get] of DIAG_FIELDS) {
    lines.push(`${label.padEnd(16)} : ${get(c)}`);
  }
  return lines.join("\n");
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
  renderDiag(node, c);
}

function attachDiagHandlers(node, getCard) {
  const details = node.querySelector(".diag");
  if (details) {
    details.addEventListener("click", (e) => e.stopPropagation());
  }
  const btn = node.querySelector(".diag-copy");
  if (btn) {
    btn.addEventListener("click", async (e) => {
      e.preventDefault();
      e.stopPropagation();
      const text = buildPasteReport(getCard());
      try {
        await navigator.clipboard.writeText(text);
        const original = btn.textContent;
        btn.textContent = "copied";
        setTimeout(() => { btn.textContent = original; }, 1200);
      } catch (_) {
        btn.textContent = "copy failed";
      }
    });
  }
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
        if (!el.classList.contains("kind-live")) { e.preventDefault(); return; }
        e.preventDefault();
        window.open(el.href, `portbook-${c.port}`);
      });
      fillCard(el, c);
      const newEntry = { el, sig: sig(c), card: c };
      attachDiagHandlers(el, () => newEntry.card);
      nodes.set(c.port, newEntry);
      entry = newEntry;
    } else {
      entry.card = c;
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

async function loadVersion() {
  const el = document.getElementById("version");
  if (!el) return;
  try {
    const res = await fetch("/api/version");
    if (!res.ok) return;
    const v = await res.json();
    el.textContent = `v${v.current}`;
    if (v.update_available && v.latest) {
      const link = document.createElement("a");
      link.href = "https://github.com/a-grasso/portbook/releases/latest";
      link.target = "_blank";
      link.rel = "noopener";
      link.className = "update";
      link.textContent = `update available → v${v.latest}`;
      el.after(link);
    }
  } catch (_) { /* silent */ }
}
loadVersion();

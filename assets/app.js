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
    c.kind, c.url, c.reason || "", c.project_name || "", c.cwd_short || "",
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

const diagPop = document.getElementById("diag-pop");
const diagPopTitle = diagPop?.querySelector(".diag-pop-title");
const diagPopGrid = diagPop?.querySelector(".diag-grid");
const diagPopCopy = diagPop?.querySelector(".diag-copy");
const diagPopClose = diagPop?.querySelector(".diag-pop-close");
let diagPopCard = null;

function renderDiagInto(dl, c) {
  dl.innerHTML = "";
  for (const [label, get] of DIAG_FIELDS) {
    const dt = document.createElement("dt");
    dt.textContent = label;
    const dd = document.createElement("dd");
    dd.textContent = String(get(c));
    dl.append(dt, dd);
  }
}

function openDiagPop(c) {
  if (!diagPop) return;
  diagPopCard = c;
  if (diagPopTitle) diagPopTitle.textContent = `:${c.port} · ${c.project_name || c.title || c.kind}`;
  if (diagPopGrid) renderDiagInto(diagPopGrid, c);
  if (diagPopCopy) diagPopCopy.textContent = "copy paste-ready report";
  diagPop.showPopover();
}

if (diagPopClose) {
  diagPopClose.addEventListener("click", () => diagPop.hidePopover());
}
if (diagPopCopy) {
  diagPopCopy.addEventListener("click", async () => {
    if (!diagPopCard) return;
    const text = buildPasteReport(diagPopCard);
    try {
      await navigator.clipboard.writeText(text);
      diagPopCopy.textContent = "copied";
      setTimeout(() => { diagPopCopy.textContent = "copy paste-ready report"; }, 1200);
    } catch (_) {
      diagPopCopy.textContent = "copy failed";
    }
  });
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
  const pending = isPending(c);
  // Pending rows get their own class so CSS can render them neutral
  // (not red) — they're a transient skeleton, not a failure.
  node.className = `card kind-${pending ? "pending" : c.kind}`;
  node.querySelector(".port").textContent = `:${c.port}`;
  const badge = node.querySelector(".kind-badge");
  if (c.kind === "live" && !pending) {
    badge.style.display = "none";
    badge.className = "kind-badge";
    badge.textContent = "";
  } else if (pending) {
    badge.style.display = "";
    badge.textContent = "probing…";
    badge.className = "kind-badge badge-pending";
  } else {
    badge.style.display = "";
    badge.textContent = c.reason || c.kind;
    badge.className = `kind-badge badge-${c.kind}`;
  }
  node.querySelector(".project").textContent = c.project_name || "";
  node.querySelector(".cwd").textContent = c.cwd_short || "";
  node.querySelector(".title").textContent = c.title || "";
  node.querySelector(".desc").textContent = c.description || "";
  node.querySelector(".cmd").textContent = c.cmdline || c.command || "";
}

function attachDiagHandlers(node, getCard) {
  const btn = node.querySelector(".diag-toggle");
  if (btn) {
    btn.addEventListener("click", (e) => {
      e.preventDefault();
      e.stopPropagation();
      openDiagPop(getCard());
    });
  }
}

function isPending(p) {
  // Prefer the explicit `pending` flag (v0.1.7+). Fall back to the
  // historical reason+attempts heuristic so older daemons still render
  // skeleton rows correctly during a rolling upgrade.
  if (p.pending === true) return true;
  return p.reason === "probing…" && (p.attempts ?? 0) === 0;
}

function emptyMessage(snapshot, tab) {
  const all = snapshot.ports || [];
  const scanned = snapshot.scan_elapsed_ms != null;
  if (all.length === 0) {
    return scanned
      ? "No listening ports detected on this host."
      : "Probing…";
  }
  // We're on a tab that filters everything out.
  if (tab === "live") return "No live HTTP services right now.";
  return "No matching ports.";
}

function render(snapshot) {
  lastSnapshot = snapshot;
  const all = snapshot.ports || [];
  // Live tab includes pending rows so the user sees the skeleton on
  // first paint instead of a misleading "nothing here" message during
  // the probe window.
  const ports = currentTab === "live"
    ? all.filter((p) => p.kind === "live" || isPending(p))
    : all;

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
    emptyEl.textContent = emptyMessage(snapshot, currentTab);
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
        const card = nodes.get(c.port)?.card ?? c;
        if (card.kind !== "live" || isPending(card)) { e.preventDefault(); return; }
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

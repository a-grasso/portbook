const grid = document.getElementById("grid");
const tpl = document.getElementById("card-tpl");
const statusDot = document.getElementById("status");
const countEl = document.getElementById("count");
const tabs = document.querySelectorAll(".tab");

let currentTab = "live";
let lastSnapshot = { ports: [] };

tabs.forEach((t) => {
  t.addEventListener("click", () => {
    tabs.forEach((x) => x.classList.toggle("active", x === t));
    currentTab = t.dataset.tab;
    render(lastSnapshot);
  });
});

function render(snapshot) {
  lastSnapshot = snapshot;
  const all = snapshot.ports || [];
  const ports = currentTab === "live" ? all.filter((p) => p.kind === "live") : all;

  const liveCount = all.filter((p) => p.kind === "live").length;
  countEl.textContent = `${liveCount} live · ${all.length} total`;

  grid.innerHTML = "";
  if (ports.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = currentTab === "live"
      ? "No HTTP services discovered yet."
      : "No listening ports detected.";
    grid.appendChild(empty);
    return;
  }
  for (const c of ports) {
    const node = tpl.content.firstElementChild.cloneNode(true);
    node.href = c.url;
    node.classList.add(`kind-${c.kind}`);
    if (c.kind !== "live") {
      node.addEventListener("click", (e) => e.preventDefault());
    }
    node.querySelector(".port").textContent = `:${c.port}`;
    const badge = node.querySelector(".kind-badge");
    if (c.kind === "live") {
      badge.remove();
    } else {
      badge.textContent = c.reason || c.kind;
      badge.classList.add(`badge-${c.kind}`);
    }
    node.querySelector(".project").textContent = c.project_name || "";
    node.querySelector(".title").textContent = c.title || "";
    node.querySelector(".desc").textContent = c.description || "";
    node.querySelector(".cmd").textContent = c.cmdline || c.command || "";
    grid.appendChild(node);
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

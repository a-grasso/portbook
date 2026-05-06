const grid = document.getElementById("grid");
const tpl = document.getElementById("card-tpl");
const statusDot = document.getElementById("status");
const countEl = document.getElementById("count");

function render(snapshot) {
  const ports = snapshot.ports || [];
  countEl.textContent = ports.length === 0 ? "" : `${ports.length} port${ports.length === 1 ? "" : "s"}`;
  grid.innerHTML = "";
  if (ports.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No HTTP services discovered yet.";
    grid.appendChild(empty);
    return;
  }
  for (const c of ports) {
    const node = tpl.content.firstElementChild.cloneNode(true);
    node.href = c.url;
    node.querySelector(".port").textContent = `:${c.port}`;
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

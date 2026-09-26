(() => {
  "use strict";

  const $ = (id) => document.getElementById(id);
  const pairingView = $("pairingView");
  const dashboardView = $("dashboardView");
  const connStatus = $("connStatus");

  const store = {
    get token() { return localStorage.getItem("cliplan_token"); },
    set token(v) { v ? localStorage.setItem("cliplan_token", v) : localStorage.removeItem("cliplan_token"); },
    get autoSync() { return localStorage.getItem("cliplan_autosync") !== "off"; },
    set autoSync(v) { localStorage.setItem("cliplan_autosync", v ? "on" : "off"); },
  };

  async function api(path, opts = {}) {
    const headers = Object.assign({}, opts.headers || {});
    if (store.token) headers["Authorization"] = "Bearer " + store.token;
    if (opts.body && !(opts.body instanceof FormData)) {
      headers["Content-Type"] = "application/json";
    }
    const res = await fetch("/api" + path, Object.assign({}, opts, { headers }));
    if (res.status === 401) {
      // Token invalid/revoked -- drop it and send the user back through pairing.
      store.token = null;
      renderRoute();
      throw new Error("unauthorized");
    }
    let data = null;
    try { data = await res.json(); } catch (_) { /* no body */ }
    if (!res.ok) {
      const msg = (data && data.error) || res.statusText;
      throw new Error(msg);
    }
    return data;
  }

  // ---------------- Routing ----------------

  function renderRoute() {
    const m = location.pathname.match(/^\/pair\/([^/]+)$/);
    const sessionId = m ? decodeURIComponent(m[1]) : null;

    if (!store.token || sessionId) {
      pairingView.classList.remove("hidden");
      dashboardView.classList.add("hidden");
      initPairingView(sessionId);
    } else {
      pairingView.classList.add("hidden");
      dashboardView.classList.remove("hidden");
      initDashboard();
    }
  }

  // ---------------- Pairing ----------------

  function initPairingView(sessionId) {
    const form = $("pairForm");
    const errorEl = $("pairError");
    const pendingEl = $("pairPending");
    errorEl.classList.add("hidden");
    pendingEl.classList.add("hidden");
    form.classList.remove("hidden");

    form.onsubmit = async (e) => {
      e.preventDefault();
      errorEl.classList.add("hidden");
      const deviceName = $("deviceNameInput").value.trim();
      const code = $("codeInput").value.trim();
      if (!deviceName || !code) return;

      try {
        const body = { device_name: deviceName, code };
        if (sessionId) body.session_id = sessionId;
        const initRes = await api("/devices/pair", { method: "POST", body: JSON.stringify(body) });
        form.classList.add("hidden");
        pendingEl.classList.remove("hidden");
        pollPairing(initRes.request_id, deviceName);
      } catch (err) {
        errorEl.textContent = "Could not request access: " + err.message;
        errorEl.classList.remove("hidden");
      }
    };
  }

  async function pollPairing(requestId, deviceName) {
    const errorEl = $("pairError");
    const pendingEl = $("pairPending");
    const started = Date.now();

    const tick = async () => {
      let status;
      try {
        status = await api("/devices/pair/" + requestId);
      } catch (err) {
        pendingEl.classList.add("hidden");
        errorEl.textContent = "Something went wrong while waiting for approval.";
        errorEl.classList.remove("hidden");
        return;
      }

      if (status.status === "approved" && status.token) {
        store.token = status.token;
        localStorage.setItem("cliplan_device_name", deviceName);
        history.replaceState(null, "", "/");
        renderRoute();
        return;
      }
      if (status.status === "rejected" || status.status === "expired") {
        pendingEl.classList.add("hidden");
        errorEl.textContent = "Pairing request was " + status.status + ". Try again.";
        errorEl.classList.remove("hidden");
        $("pairForm").classList.remove("hidden");
        return;
      }
      if (Date.now() - started > 10 * 60 * 1000) {
        pendingEl.classList.add("hidden");
        errorEl.textContent = "Request timed out waiting for approval.";
        errorEl.classList.remove("hidden");
        $("pairForm").classList.remove("hidden");
        return;
      }
      setTimeout(tick, 2000);
    };

    tick();
  }

  // ---------------- Dashboard ----------------

  let ws = null;
  let wsRetryDelay = 1000;

  function initDashboard() {
    refreshClipboard();
    refreshDevices();
    refreshDiscovered();
    refreshTransfers();
    refreshPending();
    connectWs();
    wireDashboardEvents();
  }

  function connectWs() {
    if (ws) { try { ws.close(); } catch (_) {} }
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    ws = new WebSocket(`${proto}//${location.host}/ws?token=${encodeURIComponent(store.token)}`);

    ws.onopen = () => {
      connStatus.textContent = "● Connected";
      connStatus.className = "conn-status online";
      wsRetryDelay = 1000;
    };
    ws.onclose = () => {
      connStatus.textContent = "● Offline";
      connStatus.className = "conn-status offline";
      if (store.token) {
        setTimeout(connectWs, wsRetryDelay);
        wsRetryDelay = Math.min(wsRetryDelay * 2, 15000);
      }
    };
    ws.onerror = () => ws.close();
    ws.onmessage = (evt) => {
      let event;
      try { event = JSON.parse(evt.data); } catch (_) { return; }
      handleWsEvent(event);
    };
  }

  function handleWsEvent(event) {
    switch (event.type) {
      case "clipboard.updated":
      case "clipboard.deleted":
      case "clipboard.cleared":
        refreshClipboard(event.type === "clipboard.updated");
        break;
      case "device.connected":
      case "device.disconnected":
      case "device.updated":
        refreshDevices();
        break;
      case "file.started":
      case "file.completed":
      case "file.failed":
        refreshTransfers();
        break;
      case "pairing.request":
      case "pairing.approved":
      case "pairing.rejected":
        refreshPending();
        break;
      default:
        break;
    }
  }

  function wireDashboardEvents() {
    $("sendClipBtn").onclick = async () => {
      const el = $("clipInput");
      const text = el.value.trim();
      if (!text) return;
      try {
        await api("/clipboard", { method: "POST", body: JSON.stringify({ content: text }) });
        el.value = "";
      } catch (err) {
        alert("Could not share clipboard: " + err.message);
      }
    };

    $("clearClipBtn").onclick = async () => {
      if (!confirm("Clear clipboard history?")) return;
      await api("/clipboard/clear", { method: "POST" });
      refreshClipboard();
    };

    $("autoSyncToggle").checked = store.autoSync;
    $("autoSyncToggle").onchange = (e) => { store.autoSync = e.target.checked; };

    $("clearTransfersBtn").onclick = async () => {
      if (!confirm("Clear transfer history?")) return;
      await api("/transfers/clear", { method: "POST" });
      refreshTransfers();
    };

    $("selectFilesBtn").onclick = () => $("fileInput").click();
    $("fileInput").onchange = (e) => uploadFiles(e.target.files);

    const dz = $("dropZone");
    ["dragenter", "dragover"].forEach((ev) =>
      dz.addEventListener(ev, (e) => { e.preventDefault(); dz.classList.add("dragover"); })
    );
    ["dragleave", "drop"].forEach((ev) =>
      dz.addEventListener(ev, (e) => { e.preventDefault(); dz.classList.remove("dragover"); })
    );
    dz.addEventListener("drop", (e) => {
      if (e.dataTransfer.files.length) uploadFiles(e.dataTransfer.files);
    });
  }

  // ---- Clipboard ----

  async function refreshClipboard(attemptAutoCopy) {
    let entries;
    try {
      entries = await api("/clipboard");
    } catch (_) {
      return;
    }
    const current = $("currentClip");
    const list = $("clipHistory");

    if (entries.length === 0) {
      current.textContent = "Nothing yet — copy something to get started.";
      list.innerHTML = "";
      return;
    }

    const latest = entries[0];
    current.textContent = latest.content;

    if (attemptAutoCopy && store.autoSync && navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(latest.content).catch(() => {
        // Browser blocked the background write (no user gesture); the
        // "Copy" button in the history list below is always available.
      });
    }

    list.innerHTML = "";
    for (const entry of entries) {
      const li = document.createElement("li");
      li.className = "list-item";
      const icon = entry.content_type === "url" ? "🔗 " : "";
      li.innerHTML = `
        <div class="content">
          <div class="primary">${icon}${escapeHtml(truncate(entry.content, 140))}</div>
          <div class="secondary">${entry.device_name ? escapeHtml(entry.device_name) + " · " : ""}${formatTime(entry.created_at)}</div>
        </div>
        <div class="actions">
          ${entry.content_type === "url" ? `<button data-act="open">Open</button>` : ""}
          <button data-act="copy">Copy</button>
          <button data-act="delete">Delete</button>
        </div>`;
      li.querySelector('[data-act="copy"]').onclick = () => copyText(entry.content);
      const openBtn = li.querySelector('[data-act="open"]');
      if (openBtn) openBtn.onclick = () => window.open(entry.content, "_blank", "noopener");
      li.querySelector('[data-act="delete"]').onclick = async () => {
        await api("/clipboard/" + entry.id, { method: "DELETE" });
        refreshClipboard();
      };
      list.appendChild(li);
    }
  }

  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).catch(() => fallbackCopy(text));
    } else {
      fallbackCopy(text);
    }
  }

  function fallbackCopy(text) {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand("copy"); } catch (_) {}
    document.body.removeChild(ta);
  }

  // ---- Devices ----

  async function refreshDevices() {
    let devices;
    try {
      devices = await api("/devices");
    } catch (_) {
      return;
    }
    $("deviceCount").textContent = `(${devices.length} paired)`;
    const list = $("deviceList");
    list.innerHTML = "";
    for (const d of devices) {
      const li = document.createElement("li");
      li.className = "list-item";
      const icon = d.name.toLowerCase().includes("phone") ? "📱" : "💻";
      li.innerHTML = `
        <div class="content">
          <div class="primary">${icon} ${escapeHtml(d.name)} ${d.is_self ? '<span class="badge">this device</span>' : ""}</div>
          <div class="secondary"><span class="badge ${d.status}">${d.status}</span> · last seen ${formatTime(d.last_seen)}</div>
        </div>
        <div class="actions">
          <button data-act="rename">Rename</button>
          <button data-act="remove">Remove</button>
        </div>`;
      li.querySelector('[data-act="rename"]').onclick = async () => {
        const name = prompt("Rename device", d.name);
        if (!name) return;
        await api("/devices/" + d.id, { method: "PATCH", body: JSON.stringify({ name }) });
        refreshDevices();
      };
      li.querySelector('[data-act="remove"]').onclick = async () => {
        if (!confirm(`Remove ${d.name}? It will need to be paired again.`)) return;
        await api("/devices/" + d.id, { method: "DELETE" });
        if (d.is_self) { store.token = null; renderRoute(); }
        else refreshDevices();
      };
      list.appendChild(li);
    }
  }

  async function refreshDiscovered() {
    let discovered;
    try {
      discovered = await fetch("/api/devices/discovered").then((r) => r.json());
    } catch (_) {
      return;
    }
    const list = $("discoveredList");
    list.innerHTML = "";
    $("discoveredWrap").classList.toggle("hidden", discovered.length === 0);
    for (const d of discovered) {
      const li = document.createElement("li");
      li.className = "list-item";
      li.innerHTML = `<div class="content"><div class="primary">💻 ${escapeHtml(d.device_name)}</div>
        <div class="secondary">${escapeHtml(d.ip)}:${d.port}</div></div>`;
      list.appendChild(li);
    }
  }

  async function refreshPending() {
    let pending;
    try {
      pending = await api("/devices/pairing/pending");
    } catch (_) {
      $("pendingWrap").classList.add("hidden");
      return; // likely 403 -- this dashboard isn't on the host, that's fine
    }
    $("pendingWrap").classList.toggle("hidden", pending.length === 0);
    const list = $("pendingList");
    list.innerHTML = "";
    for (const req of pending) {
      const li = document.createElement("li");
      li.className = "list-item";
      li.innerHTML = `
        <div class="content">
          <div class="primary">${escapeHtml(req.device_name)}</div>
          <div class="secondary">Fingerprint ${escapeHtml(req.fingerprint)}</div>
        </div>
        <div class="actions">
          <button data-act="allow">Allow</button>
          <button data-act="reject">Reject</button>
        </div>`;
      li.querySelector('[data-act="allow"]').onclick = async () => {
        await api("/devices/pairing/" + req.id + "/approve", { method: "POST" });
        refreshPending();
        refreshDevices();
      };
      li.querySelector('[data-act="reject"]').onclick = async () => {
        await api("/devices/pairing/" + req.id + "/reject", { method: "POST" });
        refreshPending();
      };
      list.appendChild(li);
    }
  }

  // ---- Files / transfers ----

  function uploadFiles(fileList) {
    for (const file of fileList) uploadOne(file);
  }

  function uploadOne(file) {
    const container = $("activeTransfers");
    const li = document.createElement("li");
    li.className = "list-item";
    li.innerHTML = `
      <div class="content">
        <div class="primary">${escapeHtml(file.name)}</div>
        <div class="secondary">${formatBytes(file.size)}</div>
        <div class="progress"><div style="width:0%"></div></div>
      </div>
      <div class="actions"><button data-act="cancel">Cancel</button></div>`;
    container.appendChild(li);
    const bar = li.querySelector(".progress > div");
    const sub = li.querySelector(".secondary");

    const xhr = new XMLHttpRequest();
    xhr.open("POST", "/api/files/upload");
    xhr.setRequestHeader("Authorization", "Bearer " + store.token);
    xhr.upload.onprogress = (e) => {
      if (!e.lengthComputable) return;
      const pct = Math.round((e.loaded / e.total) * 100);
      bar.style.width = pct + "%";
      sub.textContent = `${formatBytes(e.loaded)} / ${formatBytes(e.total)} — ${pct}%`;
    };
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        sub.textContent = "✓ Transfer complete";
        bar.style.width = "100%";
      } else {
        sub.textContent = "✗ Upload failed";
      }
      setTimeout(() => li.remove(), 2500);
      refreshTransfers();
    };
    xhr.onerror = () => {
      sub.textContent = "✗ Upload failed (network error)";
      setTimeout(() => li.remove(), 3000);
    };
    li.querySelector('[data-act="cancel"]').onclick = () => { xhr.abort(); li.remove(); };

    const form = new FormData();
    form.append("file", file, file.name);
    xhr.send(form);
  }

  async function refreshTransfers() {
    let transfers;
    try {
      transfers = await api("/transfers");
    } catch (_) {
      return;
    }
    const list = $("transferHistory");
    list.innerHTML = "";
    for (const t of transfers) {
      const li = document.createElement("li");
      li.className = "list-item";
      const dir = t.from_device ? "↑" : "↓";
      li.innerHTML = `
        <div class="content">
          <div class="primary">${dir} ${escapeHtml(t.filename)}</div>
          <div class="secondary">${formatBytes(t.size)} · <span class="badge ${t.status === "completed" ? "online" : t.status === "failed" ? "failed" : ""}">${t.status}</span></div>
        </div>
        <div class="actions">
          ${t.status === "completed" ? '<button data-act="download">Download</button>' : ""}
          <button data-act="delete">Delete</button>
        </div>`;
      const dl = li.querySelector('[data-act="download"]');
      if (dl) dl.onclick = () => downloadTransfer(t.id, t.filename);
      li.querySelector('[data-act="delete"]').onclick = async () => {
        await api("/transfers/" + t.id, { method: "DELETE" });
        refreshTransfers();
      };
      list.appendChild(li);
    }
  }

  async function downloadTransfer(id, filename) {
    try {
      const res = await fetch("/api/files/" + id + "/download", {
        headers: { Authorization: "Bearer " + store.token },
      });
      if (!res.ok) throw new Error("download failed");
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = filename;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
    } catch (err) {
      alert("Could not download file: " + err.message);
    }
  }

  // ---------------- Utilities ----------------

  function escapeHtml(s) {
    const div = document.createElement("div");
    div.textContent = s;
    return div.innerHTML;
  }

  function truncate(s, n) {
    return s.length > n ? s.slice(0, n) + "…" : s;
  }

  function formatBytes(n) {
    if (n === 0 || n == null) return "0 B";
    const units = ["B", "KB", "MB", "GB"];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
    return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function formatTime(iso) {
    try {
      const d = new Date(iso);
      const now = new Date();
      const sameDay = d.toDateString() === now.toDateString();
      return sameDay
        ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        : d.toLocaleDateString([], { month: "short", day: "numeric" }) + " " + d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    } catch (_) {
      return iso;
    }
  }

  renderRoute();
})();

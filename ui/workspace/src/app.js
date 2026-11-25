(() => {
  function sendIpc(payload) {
    if (window.ipc && typeof window.ipc.postMessage === "function") {
      window.ipc.postMessage(JSON.stringify(payload));
    }
  }

  function el(id) {
    return document.getElementById(id);
  }

  function renderState(snapshot) {
    if (!snapshot || !snapshot.strings) {
      return;
    }

    const s = snapshot.strings;
    document.documentElement.lang = snapshot.locale === "vi" ? "vi" : "en";
    document.body.setAttribute("data-theme", snapshot.theme);

    el("app-title").textContent = s.app.name;
    el("app-greeting").textContent = s.workspace.greeting;
    el("status-line").textContent = s.workspace.greeting;
    el("btn-quick-capture").textContent = s.workspace.trigger_capture;
    el("btn-test-nudge").textContent = s.workspace.test_nudge;

    el("heading-now").textContent = s.workspace.blocks.now;
    el("heading-today").textContent = s.workspace.blocks.today;
    el("heading-inbox").textContent = s.workspace.blocks.inbox;
    el("heading-parked").textContent = s.workspace.blocks.parked;

    el("degraded-note").textContent = s.workspace.degraded_local_only;
    el("disclaimer-note").textContent = s.workspace.disclaimer;

    const wakeText =
      snapshot.last_wake_latency_ms !== null && snapshot.last_wake_latency_ms !== undefined
        ? ` (${snapshot.last_wake_latency_ms} ms)`
        : "";
    el("capture-summary").textContent =
      snapshot.last_capture_preview
        ? `${s.capture.saved_minimal} #${snapshot.stub_capture_count}${wakeText}: ${snapshot.last_capture_preview}`
        : `${snapshot.stub_capture_count}${wakeText}`;
    el("inbox-stub-note").textContent = `${s.workspace.blocks.inbox}: ${snapshot.stub_capture_count}`;

    const banner = el("hotkey-conflict-banner");
    if (snapshot.hotkey_conflicts && snapshot.hotkey_conflicts.length > 0) {
      const messages = snapshot.hotkey_conflicts.map((c) =>
        s.hotkey.conflict_banner.replace("{binding}", c.binding)
      );
      banner.textContent = messages.join(" ");
      banner.hidden = false;
    } else {
      banner.textContent = "";
      banner.hidden = true;
    }

    const statusBanner = el("status-announcement");
    if (snapshot.status_message) {
      statusBanner.textContent = snapshot.status_message;
      statusBanner.hidden = false;
    } else {
      statusBanner.textContent = "";
      statusBanner.hidden = true;
    }

    el("heading-settings").textContent = s.workspace.settings_title;

    let activityMsg = s.workspace.activity_status_running;
    if (snapshot.session_locked) {
      activityMsg = s.workspace.activity_status_locked;
    } else if (snapshot.activity_paused) {
      activityMsg = s.workspace.activity_status_paused;
    }
    el("activity-status-text").textContent = activityMsg;

    el("btn-toggle-activity").textContent = snapshot.activity_paused
      ? s.tray.resume_activity
      : s.tray.pause_activity;

    el("recent-activity-label").textContent = `${s.workspace.recent_activity_label}: ${snapshot.recent_samples.length}`;
    const list = el("recent-activity-list");
    list.innerHTML = "";
    for (const sample of snapshot.recent_samples) {
      const li = document.createElement("li");
      li.textContent = `${sample.process_name} (idle ${sample.idle_sec}s)`;
      list.appendChild(li);
    }

    const chkStartup = el("chk-launch-at-login");
    chkStartup.checked = snapshot.launch_at_login === "enabled";
    el("label-launch-at-login").textContent = s.workspace.launch_at_login_label;
    el("note-launch-at-login").textContent = s.workspace.launch_at_login_note;

    el("label-theme").textContent = s.workspace.theme_label;
    el("select-theme").value = snapshot.theme;

    el("label-locale").textContent = s.workspace.locale_label;
    el("select-locale").value = snapshot.locale;

    el("label-capture-hotkey").textContent = s.hotkey.capture_label;
    el("input-capture-hotkey").value = snapshot.capture_hotkey;
    el("btn-rebind-capture").textContent = s.hotkey.rebind_action;

    el("label-nudge-hotkey").textContent = s.hotkey.nudge_label;
    el("input-nudge-hotkey").value = snapshot.nudge_hotkey;
    el("btn-rebind-nudge").textContent = s.hotkey.rebind_action;

    el("note-countdowns").textContent = s.settings.countdowns_note;

    if (snapshot.route === "settings") {
      el("settings-panel").scrollIntoView({ behavior: "auto" });
    }
  }

  window.__vunOnIpcMessage = function (message) {
    if (!message) {
      return;
    }
    if (message.type === "state" && message.snapshot) {
      renderState(message.snapshot);
    } else if (message.type === "error" && message.message) {
      const statusBanner = el("status-announcement");
      statusBanner.textContent = message.message;
      statusBanner.hidden = false;
    }
  };

  document.addEventListener("DOMContentLoaded", () => {
    el("btn-quick-capture").addEventListener("click", () => {
      sendIpc({ type: "trigger_capture" });
    });

    el("btn-test-nudge").addEventListener("click", () => {
      sendIpc({ type: "show_test_nudge" });
    });

    el("btn-toggle-activity").addEventListener("click", () => {
      sendIpc({ type: "toggle_activity_monitoring" });
    });

    el("chk-launch-at-login").addEventListener("change", (ev) => {
      sendIpc({
        type: "set_launch_at_login",
        enabled: Boolean(ev.target.checked),
      });
    });

    el("select-theme").addEventListener("change", (ev) => {
      sendIpc({ type: "set_theme", theme: ev.target.value });
    });

    el("select-locale").addEventListener("change", (ev) => {
      sendIpc({ type: "set_locale", locale: ev.target.value });
    });

    el("form-capture-hotkey").addEventListener("submit", (ev) => {
      ev.preventDefault();
      sendIpc({
        type: "rebind_hotkey",
        kind: "capture",
        binding: el("input-capture-hotkey").value,
      });
    });

    el("form-nudge-hotkey").addEventListener("submit", (ev) => {
      ev.preventDefault();
      sendIpc({
        type: "rebind_hotkey",
        kind: "focus_nudge",
        binding: el("input-nudge-hotkey").value,
      });
    });

    sendIpc({ type: "get_shell_state" });
  });
})();

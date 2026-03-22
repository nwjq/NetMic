export const bindConfigInputs = ({ root, getState, setState, adapter }) => {
  if (!root) return;
  root.querySelectorAll("[data-field]").forEach((input) => {
    input.addEventListener("change", async (event) => {
      const state = getState();
      const target = event.target;
      const field = target.dataset.field;
      let value = target.value;
      if (target.type === "checkbox") {
        value = target.checked;
      }
      if (target.type === "number") {
        value = Number(value);
      }

      const isClient = state.mode === "client";
      const baseConfig = isClient ? state.client_config || {} : state.server_config || {};
      const nextConfig = { ...baseConfig, [field]: value };
      const snapshot = isClient
        ? await adapter.setClientConfig(nextConfig)
        : await adapter.setServerConfig(nextConfig);
      setState(snapshot);

      if (!isClient && field === "virtual_mic_enabled") {
        const nextSnapshot = value
          ? await adapter.createVirtualMic()
          : await adapter.removeVirtualMic();
        setState(nextSnapshot);
      }
    });
  });

  root.querySelectorAll("[data-app-field]").forEach((input) => {
    input.addEventListener("change", async (event) => {
      const target = event.target;
      const field = target.dataset.appField;
      if (field !== "launch_at_login") return;
      const snapshot = await adapter.setLaunchAtLogin(Boolean(target.checked));
      setState(snapshot);
    });
  });

  const forceButton = root.getElementById ? root.getElementById("force-disconnect") : null;
  if (forceButton) {
    forceButton.addEventListener("click", async () => {
      const snapshot = await adapter.forceDisconnect();
      setState(snapshot);
    });
  }
};

export const bindActions = ({
  elements,
  getState,
  setState,
  adapter,
  root,
  setActiveTab,
  isBusy,
  renderLogs,
}) => {
  const handleHideToTray = async () => {
    await adapter.hideToTray();
  };

  elements.modeButtons.forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (isBusy(getState())) return;
      const snapshot = await adapter.setMode(btn.dataset.mode);
      setState(snapshot);
    });
  });

  elements.navButtons.forEach((btn) => {
    btn.addEventListener("click", () => setActiveTab(btn.dataset.tab));
  });

  elements.primaryAction.addEventListener("click", async () => {
    const snapshot = isBusy(getState()) ? await adapter.stop() : await adapter.start();
    setState(snapshot);
  });

  elements.resetDefaults.addEventListener("click", async () => {
    const snapshot = await adapter.resetDefaults();
    setState(snapshot);
  });

  elements.logFilter.addEventListener("change", renderLogs);
  elements.logClear.addEventListener("click", async () => {
    const snapshot = await adapter.clearLogs();
    setState(snapshot);
  });
  elements.logExport.addEventListener("click", async () => {
    await adapter.exportLogs();
  });

  const eventRoot = root && typeof root.addEventListener === "function" ? root : null;
  if (!eventRoot) return;
  eventRoot.addEventListener("keydown", async (event) => {
    const key = String(event.key || "").toLowerCase();
    const primaryMod = event.metaKey || event.ctrlKey;
    const hideToTray =
      primaryMod && !event.shiftKey && !event.altKey && !event.repeat && key === "w";
    if (!hideToTray) return;
    if (typeof event.preventDefault === "function") {
      event.preventDefault();
    }
    await handleHideToTray();
  });
};

export const bindConfigInputs = ({ root, state, setState, adapter }) => {
  if (!root) return;
  root.querySelectorAll("[data-field]").forEach((input) => {
    input.addEventListener("change", async (event) => {
      const target = event.target;
      const field = target.dataset.field;
      let value = target.value;
      if (target.type === "checkbox") {
        value = target.checked;
      }
      if (target.type === "number") {
        value = Number(value);
      }

      const nextConfig = { ...state.config, [field]: value };
      const snapshot = await adapter.setConfig(nextConfig);
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
  state,
  setState,
  adapter,
  setActiveTab,
  isBusy,
  renderLogs,
}) => {
  elements.modeButtons.forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (isBusy(state)) return;
      const snapshot = await adapter.setMode(btn.dataset.mode);
      setState(snapshot);
    });
  });

  elements.navButtons.forEach((btn) => {
    btn.addEventListener("click", () => setActiveTab(btn.dataset.tab));
  });

  elements.primaryAction.addEventListener("click", async () => {
    const snapshot = isBusy(state) ? await adapter.stop() : await adapter.start();
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
};

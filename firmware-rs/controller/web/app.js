const state = {
  schedule: { enabled: false, entries: [] },
  status: null,
  irConfig: null,
  ota: null,
};

const els = {
  connection: document.getElementById('connection-status'),
  currentTemp: document.getElementById('current-temp'),
  currentHumidity: document.getElementById('current-humidity'),
  sensorStatus: document.getElementById('sensor-status'),
  thermostatState: document.getElementById('thermostat-state'),
  fireplaceStatus: document.getElementById('fireplace-status'),
  targetTemp: document.getElementById('target-temp'),
  fireplaceTemp: document.getElementById('fireplace-temp'),
  lightLevel: document.getElementById('light-level'),
  timerState: document.getElementById('timer-state'),
  holdRemaining: document.getElementById('hold-remaining'),
  cooldownRemaining: document.getElementById('cooldown-remaining'),
  runtimeMin: document.getElementById('runtime-min'),
  hysteresis: document.getElementById('hysteresis'),
  offset: document.getElementById('offset'),
  scheduleEnabled: document.getElementById('schedule-enabled'),
  scheduleDay: document.getElementById('schedule-day'),
  scheduleTime: document.getElementById('schedule-time'),
  scheduleMode: document.getElementById('schedule-mode'),
  scheduleTarget: document.getElementById('schedule-target'),
  scheduleList: document.getElementById('schedule-list'),
  nextEvent: document.getElementById('next-event'),
  timezone: document.getElementById('timezone'),
  timeSynced: document.getElementById('time-synced'),
  irTxPin: document.getElementById('ir-tx-pin'),
  irRmtChannel: document.getElementById('ir-rmt-channel'),
  irCarrierKhz: document.getElementById('ir-carrier-khz'),
  irConfigStatus: document.getElementById('ir-config-status'),
  irEnabled: document.getElementById('ir-enabled'),
  irSentFrames: document.getElementById('ir-sent-frames'),
  irFailedActions: document.getElementById('ir-failed-actions'),
  irLastError: document.getElementById('ir-last-error'),
  otaUrl: document.getElementById('ota-url'),
  otaSha256: document.getElementById('ota-sha256'),
  otaPassword: document.getElementById('ota-password'),
  otaReboot: document.getElementById('ota-reboot'),
  otaStatus: document.getElementById('ota-status'),
  otaSupported: document.getElementById('ota-supported'),
  otaInProgress: document.getElementById('ota-in-progress'),
  otaBytes: document.getElementById('ota-bytes'),
  otaProgress: document.getElementById('ota-progress'),
  otaLastError: document.getElementById('ota-last-error'),
};

async function api(path, options = {}) {
  const response = await fetch(path, options);
  if (!response.ok) {
    let message = `Request failed: ${response.status}`;
    try {
      const body = await response.json();
      message = body.error || message;
    } catch (_) {}
    throw new Error(message);
  }
  return response.json();
}

function setConnected(connected) {
  els.connection.textContent = connected ? 'Connected' : 'Disconnected';
  els.connection.className = connected ? 'pill connected' : 'pill disconnected';
}

function minutesToHHMM(minutes) {
  const h = String(Math.floor(minutes / 60)).padStart(2, '0');
  const m = String(minutes % 60).padStart(2, '0');
  return `${h}:${m}`;
}

function hhmmToMinutes(value) {
  const [h, m] = value.split(':').map(Number);
  return h * 60 + m;
}

function formatOtaBytes(bytesWritten, totalBytes) {
  const written = Number(bytesWritten || 0);
  if (totalBytes == null) return `${written}`;
  return `${written} / ${Number(totalBytes)}`;
}

function renderSchedule() {
  els.scheduleList.innerHTML = '';
  const entries = [...state.schedule.entries].sort((a, b) => {
    if (a.day === b.day) return a.startMinutes - b.startMinutes;
    return a.day.localeCompare(b.day);
  });

  for (const [index, entry] of entries.entries()) {
    const li = document.createElement('li');
    li.className = 'schedule-item';
    li.innerHTML = `
      <span>${entry.day} ${minutesToHHMM(entry.startMinutes)} \u2192 ${entry.mode} ${entry.targetTemp}\u00b0F</span>
      <button class="btn ghost" data-remove="${index}">Remove</button>
    `;
    els.scheduleList.appendChild(li);
  }

  els.scheduleList.querySelectorAll('button[data-remove]').forEach((button) => {
    button.addEventListener('click', () => {
      const idx = Number(button.dataset.remove);
      state.schedule.entries.splice(idx, 1);
      renderSchedule();
    });
  });
}

function updateStatus(status) {
  state.status = status;
  els.currentTemp.textContent = Number(status.currentTemp || 0).toFixed(1);
  els.currentHumidity.textContent = `${Number(status.currentHumidity || 0).toFixed(0)}%`;
  els.sensorStatus.textContent = status.sensorValid ? 'Sensor OK' : 'Sensor stale';
  els.thermostatState.textContent = status.state;
  els.fireplaceStatus.textContent = status.fireplaceOn ? 'On' : 'Off';
  els.targetTemp.textContent = Number(status.targetTemp || 0).toFixed(0);
  els.fireplaceTemp.textContent = status.fireplaceTemp ?? '--';
  els.lightLevel.textContent = status.lightLevel ?? '--';
  els.timerState.textContent = status.timerString ?? '--';
  els.holdRemaining.textContent = status.holdRemainingMin ?? 0;
  els.cooldownRemaining.textContent = status.cooldownRemainingMin ?? 0;
  els.runtimeMin.textContent = status.runtimeMin ?? 0;
  els.hysteresis.value = status.hysteresis;
  els.offset.value = status.fireplaceOffset;
  els.scheduleEnabled.checked = Boolean(status.scheduleEnabled);
  els.nextEvent.textContent = status.nextScheduleEventEpoch
    ? new Date(status.nextScheduleEventEpoch * 1000).toLocaleString()
    : '--';
  els.timeSynced.textContent = String(Boolean(status.timeSynced));
  els.timezone.value = status.timezone || '';
}

async function refreshStatus() {
  try {
    const status = await api('/api/status');
    updateStatus(status);
    setConnected(true);
  } catch (error) {
    setConnected(false);
    console.error(error);
  }
}

async function refreshSchedule() {
  try {
    state.schedule = await api('/api/schedule');
    els.scheduleEnabled.checked = Boolean(state.schedule.enabled);
    renderSchedule();
  } catch (error) {
    console.error(error);
  }
}

function updateIrConfig(config) {
  state.irConfig = config;
  els.irTxPin.value = config.txPin ?? 4;
  els.irRmtChannel.value = config.rmtChannel ?? 0;
  els.irCarrierKhz.value = config.carrierKHz ?? 36;
  els.irConfigStatus.textContent = 'IR config loaded.';
}

function updateIrDiagnostics(diagnostics) {
  els.irEnabled.textContent = String(Boolean(diagnostics.enabled));
  els.irSentFrames.textContent = String(diagnostics.sentFrames ?? 0);
  els.irFailedActions.textContent = String(diagnostics.failedActions ?? 0);
  els.irLastError.textContent = diagnostics.lastError || '--';
}

async function refreshIrConfig() {
  try {
    const config = await api('/api/ir/config');
    updateIrConfig(config);
  } catch (error) {
    els.irConfigStatus.textContent = `IR config unavailable: ${error.message}`;
    console.error(error);
  }
}

async function refreshIrDiagnostics() {
  try {
    const diagnostics = await api('/api/ir/diagnostics');
    updateIrDiagnostics(diagnostics);
  } catch (error) {
    els.irLastError.textContent = error.message;
    console.error(error);
  }
}

function updateOtaStatus(status) {
  state.ota = status;
  els.otaSupported.textContent = String(Boolean(status.supported));
  els.otaInProgress.textContent = String(Boolean(status.inProgress));
  els.otaBytes.textContent = formatOtaBytes(status.bytesWritten, status.totalBytes);
  els.otaProgress.textContent = status.progressPct == null ? '--' : `${status.progressPct}%`;
  els.otaLastError.textContent = status.lastError || '--';

  if (status.inProgress) {
    els.otaStatus.textContent = 'OTA update in progress...';
  } else if (status.lastError) {
    els.otaStatus.textContent = `Last OTA result: ${status.lastError}`;
  } else {
    els.otaStatus.textContent = 'OTA idle.';
  }
}

async function refreshOtaStatus() {
  try {
    const status = await api('/api/ota/status');
    updateOtaStatus(status);
  } catch (error) {
    els.otaStatus.textContent = `OTA status unavailable: ${error.message}`;
    console.error(error);
  }
}

function bindControls() {
  document.getElementById('target-up').addEventListener('click', () => api(`/api/target?value=${(state.status?.targetTemp || 70) + 1}`, { method: 'POST' }).then(updateStatus));
  document.getElementById('target-down').addEventListener('click', () => api(`/api/target?value=${(state.status?.targetTemp || 70) - 1}`, { method: 'POST' }).then(updateStatus));

  document.getElementById('mode-off').addEventListener('click', () => api('/api/mode?value=OFF', { method: 'POST' }).then(updateStatus));
  document.getElementById('mode-heat').addEventListener('click', () => api('/api/mode?value=HEAT', { method: 'POST' }).then(updateStatus));

  const commands = [
    ['ir-on', '/api/ir/on'],
    ['ir-off', '/api/ir/off'],
    ['heat-on', '/api/ir/heat/on'],
    ['heat-off', '/api/ir/heat/off'],
    ['heat-up', '/api/ir/heat/up'],
    ['heat-down', '/api/ir/heat/down'],
    ['light-toggle', '/api/ir/light/toggle'],
    ['timer-toggle', '/api/ir/timer/toggle'],
    ['hold-enter', '/api/hold/enter?minutes=30'],
    ['hold-exit', '/api/hold/exit'],
    ['safety-reset', '/api/safety/reset'],
  ];

  for (const [id, endpoint] of commands) {
    document.getElementById(id).addEventListener('click', () => api(endpoint, { method: 'POST' }).then(updateStatus));
  }

  document.getElementById('save-hysteresis').addEventListener('click', () => {
    const value = Number(els.hysteresis.value);
    api(`/api/hysteresis?value=${value}`, { method: 'POST' }).then(updateStatus).catch(console.error);
  });

  document.getElementById('save-offset').addEventListener('click', () => {
    const value = Number(els.offset.value);
    api(`/api/offset?value=${value}`, { method: 'POST' }).then(updateStatus).catch(console.error);
  });

  document.getElementById('schedule-form').addEventListener('submit', (event) => {
    event.preventDefault();
    state.schedule.entries.push({
      day: els.scheduleDay.value,
      startMinutes: hhmmToMinutes(els.scheduleTime.value),
      mode: els.scheduleMode.value,
      targetTemp: Number(els.scheduleTarget.value || 70),
    });
    renderSchedule();
  });

  document.getElementById('schedule-clear').addEventListener('click', () => {
    state.schedule.entries = [];
    renderSchedule();
  });

  document.getElementById('schedule-save').addEventListener('click', async () => {
    state.schedule.enabled = els.scheduleEnabled.checked;
    await api('/api/schedule', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(state.schedule),
    });
    await refreshSchedule();
    await refreshStatus();
  });

  document.getElementById('save-timezone').addEventListener('click', async () => {
    await api('/api/timezone', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ timezone: els.timezone.value }),
    });
    await refreshStatus();
  });

  document.getElementById('ir-save-config').addEventListener('click', async () => {
    els.irConfigStatus.textContent = 'Saving IR config...';
    try {
      const payload = {
        txPin: Number(els.irTxPin.value || 4),
        rmtChannel: Number(els.irRmtChannel.value || 0),
        carrierKHz: Number(els.irCarrierKhz.value || 36),
      };
      const response = await api('/api/ir/config', {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(payload),
      });
      updateIrConfig(response.ir || payload);
      els.irConfigStatus.textContent = `Saved. restartRequired=${String(Boolean(response.restartRequired))}`;
      await refreshIrDiagnostics();
    } catch (error) {
      els.irConfigStatus.textContent = error.message;
      console.error(error);
    }
  });

  document.getElementById('ir-refresh-diagnostics').addEventListener('click', refreshIrDiagnostics);

  document.getElementById('ota-refresh-status').addEventListener('click', refreshOtaStatus);
  document.getElementById('ota-apply').addEventListener('click', async () => {
    const url = (els.otaUrl.value || '').trim();
    if (!url) {
      els.otaStatus.textContent = 'Firmware URL is required.';
      return;
    }

    els.otaStatus.textContent = 'Starting OTA apply...';
    try {
      const payload = {
        url,
        reboot: Boolean(els.otaReboot.checked),
      };
      const sha = (els.otaSha256.value || '').trim();
      if (sha) payload.sha256 = sha;
      const password = (els.otaPassword.value || '').trim();
      if (password) payload.password = password;

      await api('/api/ota/apply', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(payload),
      });
      els.otaStatus.textContent = 'OTA apply started.';
      await refreshOtaStatus();
    } catch (error) {
      els.otaStatus.textContent = error.message;
      console.error(error);
    }
  });
}

async function init() {
  bindControls();
  await Promise.all([
    refreshSchedule(),
    refreshStatus(),
    refreshIrConfig(),
    refreshIrDiagnostics(),
    refreshOtaStatus(),
  ]);
  setInterval(() => {
    refreshStatus();
    refreshIrDiagnostics();
    refreshOtaStatus();
  }, 5000);
}

init().catch((error) => {
  console.error(error);
  setConnected(false);
});

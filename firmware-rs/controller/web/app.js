/* ── Smart Thermostat · app.js ── */

const CIRCUMFERENCE = 2 * Math.PI * 88; // SVG circle r=88
const TEMP_MIN = 50;
const TEMP_MAX = 90;

const state = {
  schedule: { enabled: false, entries: [] },
  status: null,
  irConfig: null,
  ota: null,
  network: null,
};

/* ── Theme (auto light/dark by time of day) ── */

function applyTheme() {
  var h = new Date().getHours();
  var isDay = h >= 7 && h < 19;
  document.documentElement.classList.toggle('light', isDay);
  var meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.setAttribute('content', isDay ? '#f4f3f1' : '#111118');
}
applyTheme();
setInterval(applyTheme, 60000);

/* ── Helpers ── */

function $(id) { return document.getElementById(id); }

var _pending = {};
function api(path, options) {
  var key = (options && options.method || 'GET') + ' ' + path;
  if (_pending[key]) return _pending[key];
  var p = fetch(path, options || {}).then(function (r) {
    if (!r.ok) return r.json().catch(function () { return {}; }).then(function (b) {
      throw new Error(b.error || 'Request failed: ' + r.status);
    });
    return r.json();
  }).finally(function () { delete _pending[key]; });
  _pending[key] = p;
  return p;
}

/* Debounce button taps (500ms lockout) */
function guardBtn(el, fn) {
  var locked = false;
  el.addEventListener('click', function (e) {
    if (locked) { e.preventDefault(); return; }
    locked = true;
    setTimeout(function () { locked = false; }, 500);
    fn(e);
  });
}

function parseIp(str) {
  var parts = (str || '').split('.').map(Number);
  return parts.length === 4 ? parts : [0, 0, 0, 0];
}

function formatIp(arr) {
  return Array.isArray(arr) ? arr.join('.') : (arr || '');
}

function minutesToHHMM(m) {
  return String(Math.floor(m / 60)).padStart(2, '0') + ':' + String(m % 60).padStart(2, '0');
}

function hhmmToMinutes(v) {
  var p = v.split(':').map(Number);
  return p[0] * 60 + p[1];
}

function tempFraction(t) {
  return Math.max(0, Math.min(1, (t - TEMP_MIN) / (TEMP_MAX - TEMP_MIN)));
}

/* ── Dial ring update ── */

function updateRings() {
  var s = state.status;
  if (!s) return;
  var current = Number(s.currentTemp || 0);
  var target = Number(s.targetTemp || 70);
  var ringCurrent = $('ring-current');
  var ringTarget = $('ring-target');
  if (ringTarget) ringTarget.style.strokeDashoffset = CIRCUMFERENCE * (1 - tempFraction(target));
  if (ringCurrent) ringCurrent.style.strokeDashoffset = CIRCUMFERENCE * (1 - tempFraction(current));
}

/* ── Mode styling ── */

function applyMode(mode) {
  document.body.setAttribute('data-mode', mode || 'OFF');
  var offBtn = $('mode-off');
  var heatBtn = $('mode-heat');
  if (offBtn) offBtn.className = mode === 'OFF' ? 'mode-btn active' : 'mode-btn';
  if (heatBtn) heatBtn.className = mode !== 'OFF' ? 'mode-btn active' : 'mode-btn';
}

/* ── Status update ── */

function updateStatus(s) {
  state.status = s;
  var current = Number(s.currentTemp || 0);
  var target = Number(s.targetTemp || 70);
  var mode = s.mode || 'OFF';

  $('current-temp').textContent = current.toFixed(1);
  $('target-temp').textContent = target.toFixed(0);
  $('current-humidity').textContent = Number(s.currentHumidity || 0).toFixed(0) + '%';
  $('sensor-status').textContent = s.sensorValid ? 'Sensor OK' : 'Sensor stale';
  $('thermostat-state').textContent = s.state || 'IDLE';
  $('fireplace-status').textContent = s.fireplaceOn ? 'On' : 'Off';
  var fpDot = $('fp-dot');
  if (fpDot) { if (s.fireplaceOn) fpDot.classList.add('on'); else fpDot.classList.remove('on'); }
  $('fireplace-temp').textContent = s.fireplaceTemp != null ? s.fireplaceTemp : '--';
  $('light-level').textContent = s.lightLevel != null ? s.lightLevel : '--';
  $('timer-state').textContent = s.timerString || '--';
  $('hold-remaining').textContent = s.holdRemainingMin || 0;
  $('cooldown-remaining').textContent = s.cooldownRemainingMin || 0;
  $('runtime-min').textContent = s.runtimeMin || 0;

  /* Show hold bar only when hold or cooldown is active */
  var holdBar = $('hold-bar');
  var holdActive = (s.holdRemainingMin || 0) > 0 || (s.cooldownRemainingMin || 0) > 0;
  if (holdBar) holdBar.style.display = holdActive ? '' : 'none';
  $('schedule-enabled').checked = Boolean(s.scheduleEnabled);
  $('next-event').textContent = s.nextScheduleEventEpoch
    ? new Date(s.nextScheduleEventEpoch * 1000).toLocaleString()
    : '--';
  $('time-synced').textContent = String(Boolean(s.timeSynced));

  /* connection chip */
  var chip = $('chip-connection');
  var dot = chip.querySelector('.dot');
  var txt = chip.querySelector('.status-text');
  if (dot) dot.classList.add('ok');
  if (txt) txt.textContent = 'Connected';

  /* sensor chip dot */
  var sensorDot = $('chip-sensor').querySelector('.chip-icon');
  if (sensorDot) sensorDot.style.opacity = s.sensorValid ? 1 : 0.4;

  applyMode(mode === 'OFF' ? 'OFF' : 'HEATING');
  updateRings();
}

function setDisconnected() {
  var chip = $('chip-connection');
  var dot = chip.querySelector('.dot');
  var txt = chip.querySelector('.status-text');
  if (dot) dot.classList.remove('ok');
  if (txt) txt.textContent = 'Disconnected';
}

/* ── Schedule rendering (safe DOM, no innerHTML) ── */

function renderSchedule() {
  var list = $('schedule-list');
  while (list.firstChild) list.removeChild(list.firstChild);

  var entries = state.schedule.entries.slice().sort(function (a, b) {
    if (a.day === b.day) return a.startMinutes - b.startMinutes;
    return a.day.localeCompare(b.day);
  });

  entries.forEach(function (entry, index) {
    var li = document.createElement('li');
    li.className = 'sched-item';

    var span = document.createElement('span');
    span.textContent = entry.day + ' ' + minutesToHHMM(entry.startMinutes) +
      ' \u2192 ' + entry.mode + ' ' + entry.targetTemp + '\u00b0F';
    li.appendChild(span);

    var btn = document.createElement('button');
    btn.textContent = '\u2715';
    btn.setAttribute('data-rm', String(index));
    btn.addEventListener('click', function () {
      state.schedule.entries.splice(index, 1);
      renderSchedule();
    });
    li.appendChild(btn);

    list.appendChild(li);
  });
}

/* ── Network config ── */

function updateNetworkUI(n) {
  state.network = n;
  $('net-ssid').value = n.wifiSsid || '';
  $('net-mqtt-host').value = n.mqttHost || '';
  $('net-mqtt-port').value = n.mqttPort || '';
  $('net-mqtt-user').value = n.mqttUser || '';
  var staticIp = Boolean(n.useStaticIp);
  $('net-static-ip').checked = staticIp;
  $('static-ip-fields').style.display = staticIp ? '' : 'none';
  if (n.staticIp) $('net-ip').value = formatIp(n.staticIp);
  if (n.gateway) $('net-gw').value = formatIp(n.gateway);
  if (n.subnet) $('net-subnet').value = formatIp(n.subnet);
  if (n.dns) $('net-dns').value = formatIp(n.dns);
}

async function refreshNetwork() {
  try {
    var n = await api('/api/network');
    updateNetworkUI(n);
  } catch (e) {
    $('net-status').textContent = 'Network config unavailable';
    console.error(e);
  }
}

/* ── IR config/diagnostics ── */

function updateIrConfig(c) {
  state.irConfig = c;
  $('ir-tx-pin').value = c.txPin != null ? c.txPin : 4;
  $('ir-rmt-channel').value = c.rmtChannel != null ? c.rmtChannel : 0;
  $('ir-carrier-khz').value = c.carrierKHz != null ? c.carrierKHz : 36;
  $('ir-config-status').textContent = 'IR config loaded.';
}

function updateIrDiagnostics(d) {
  $('ir-enabled').textContent = String(Boolean(d.enabled));
  $('ir-sent-frames').textContent = String(d.sentFrames || 0);
  $('ir-failed-actions').textContent = String(d.failedActions || 0);
  $('ir-last-error').textContent = d.lastError || '--';
}

/* ── OTA ── */

function updateOtaStatus(s) {
  state.ota = s;
  $('ota-supported').textContent = String(Boolean(s.supported));
  $('ota-in-progress').textContent = String(Boolean(s.inProgress));
  var written = Number(s.bytesWritten || 0);
  $('ota-bytes').textContent = s.totalBytes != null ? written + ' / ' + Number(s.totalBytes) : String(written);
  $('ota-progress').textContent = s.progressPct != null ? s.progressPct + '%' : '--';
  $('ota-last-error').textContent = s.lastError || '--';
  $('ota-status').textContent = s.inProgress ? 'OTA update in progress...'
    : s.lastError ? 'Last OTA: ' + s.lastError : 'OTA idle.';
}

/* Populate settings inputs (only on load or after save, not during poll) */
function updateSettingsInputs(s) {
  var active = document.activeElement;
  if (active !== $('hysteresis')) $('hysteresis').value = s.hysteresis;
  if (active !== $('offset')) $('offset').value = s.fireplaceOffset;
  if (active !== $('timezone')) $('timezone').value = s.timezone || '';
}

/* ── Refresh functions ── */

async function refreshStatus() {
  try {
    updateStatus(await api('/api/status'));
  } catch (e) {
    setDisconnected();
    console.error(e);
  }
}

async function refreshSchedule() {
  try {
    state.schedule = await api('/api/schedule');
    $('schedule-enabled').checked = Boolean(state.schedule.enabled);
    renderSchedule();
  } catch (e) { console.error(e); }
}

async function refreshIrConfig() {
  try { updateIrConfig(await api('/api/ir/config')); }
  catch (e) { $('ir-config-status').textContent = 'IR config unavailable'; console.error(e); }
}

async function refreshIrDiagnostics() {
  try { updateIrDiagnostics(await api('/api/ir/diagnostics')); }
  catch (e) { console.error(e); }
}

async function refreshOtaStatus() {
  try { updateOtaStatus(await api('/api/ota/status')); }
  catch (e) { $('ota-status').textContent = 'OTA unavailable'; console.error(e); }
}

/* ── Panel toggle ── */

function initPanels() {
  document.querySelectorAll('.panel-header').forEach(function (header) {
    header.addEventListener('click', function () {
      header.closest('.panel').classList.toggle('open');
    });
  });
}

/* ── Static IP toggle ── */

function initStaticIpToggle() {
  $('net-static-ip').addEventListener('change', function () {
    $('static-ip-fields').style.display = this.checked ? '' : 'none';
  });
}

/* ── Bind all controls ── */

function bindControls() {
  /* Target temp +/- */
  guardBtn($('target-up'), function () {
    var t = (state.status ? state.status.targetTemp : 70) + 1;
    api('/api/target?value=' + t, { method: 'POST' }).then(updateStatus);
  });
  guardBtn($('target-down'), function () {
    var t = (state.status ? state.status.targetTemp : 70) - 1;
    api('/api/target?value=' + t, { method: 'POST' }).then(updateStatus);
  });

  /* Mode */
  guardBtn($('mode-off'), function () {
    api('/api/mode?value=OFF', { method: 'POST' }).then(updateStatus);
  });
  guardBtn($('mode-heat'), function () {
    api('/api/mode?value=HEAT', { method: 'POST' }).then(updateStatus);
  });

  /* IR + hold + safety commands */
  var cmds = [
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
  cmds.forEach(function (pair) {
    guardBtn($(pair[0]), function () {
      api(pair[1], { method: 'POST' }).then(updateStatus);
    });
  });

  /* Hysteresis */
  guardBtn($('save-hysteresis'), function () {
    api('/api/hysteresis?value=' + Number($('hysteresis').value), { method: 'POST' }).then(function (s) {
      updateStatus(s); updateSettingsInputs(s);
    }).catch(console.error);
  });

  /* Offset */
  guardBtn($('save-offset'), function () {
    api('/api/offset?value=' + Number($('offset').value), { method: 'POST' }).then(function (s) {
      updateStatus(s); updateSettingsInputs(s);
    }).catch(console.error);
  });

  /* Schedule form */
  $('schedule-form').addEventListener('submit', function (e) {
    e.preventDefault();
    state.schedule.entries.push({
      day: $('schedule-day').value,
      startMinutes: hhmmToMinutes($('schedule-time').value),
      mode: $('schedule-mode').value,
      targetTemp: Number($('schedule-target').value || 70),
    });
    renderSchedule();
  });

  guardBtn($('schedule-clear'), function () {
    state.schedule.entries = [];
    renderSchedule();
  });

  guardBtn($('schedule-save'), async function () {
    state.schedule.enabled = $('schedule-enabled').checked;
    await api('/api/schedule', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(state.schedule),
    });
    await refreshSchedule();
    await refreshStatus();
  });

  /* Timezone */
  guardBtn($('save-timezone'), async function () {
    await api('/api/timezone', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ timezone: $('timezone').value }),
    });
    await refreshStatus();
    if (state.status) updateSettingsInputs(state.status);
  });

  /* IR config */
  guardBtn($('ir-save-config'), async function () {
    $('ir-config-status').textContent = 'Saving...';
    try {
      var payload = {
        txPin: Number($('ir-tx-pin').value || 4),
        rmtChannel: Number($('ir-rmt-channel').value || 0),
        carrierKHz: Number($('ir-carrier-khz').value || 36),
      };
      var resp = await api('/api/ir/config', {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(payload),
      });
      updateIrConfig(resp.ir || payload);
      $('ir-config-status').textContent = 'Saved. restartRequired=' + String(Boolean(resp.restartRequired));
      await refreshIrDiagnostics();
    } catch (e) {
      $('ir-config-status').textContent = e.message;
    }
  });

  guardBtn($('ir-refresh-diagnostics'), refreshIrDiagnostics);

  /* OTA */
  guardBtn($('ota-refresh-status'), refreshOtaStatus);
  guardBtn($('ota-apply'), async function () {
    var url = ($('ota-url').value || '').trim();
    if (!url) { $('ota-status').textContent = 'URL required.'; return; }
    $('ota-status').textContent = 'Starting OTA...';
    try {
      var payload = { url: url, reboot: Boolean($('ota-reboot').checked) };
      var sha = ($('ota-sha256').value || '').trim();
      if (sha) payload.sha256 = sha;
      var pw = ($('ota-password').value || '').trim();
      if (pw) payload.password = pw;
      await api('/api/ota/apply', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(payload),
      });
      $('ota-status').textContent = 'OTA apply started.';
      await refreshOtaStatus();
    } catch (e) {
      $('ota-status').textContent = e.message;
    }
  });

  /* Network save */
  guardBtn($('net-save'), async function () {
    $('net-status').textContent = 'Saving...';
    try {
      var payload = {
        wifiSsid: $('net-ssid').value,
        mqttHost: $('net-mqtt-host').value,
        mqttPort: Number($('net-mqtt-port').value || 1883),
        mqttUser: $('net-mqtt-user').value,
        useStaticIp: $('net-static-ip').checked,
      };
      var mqttPw = ($('net-mqtt-pass').value || '').trim();
      if (mqttPw) payload.mqttPass = mqttPw;
      if (payload.useStaticIp) {
        payload.staticIp = parseIp($('net-ip').value);
        payload.gateway = parseIp($('net-gw').value);
        payload.subnet = parseIp($('net-subnet').value);
        payload.dns = parseIp($('net-dns').value);
      }
      var resp = await api('/api/network', {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(payload),
      });
      $('net-status').textContent = 'Saved. restartRequired=' + String(Boolean(resp.restartRequired));
    } catch (e) {
      $('net-status').textContent = e.message;
    }
  });
}

/* ── Init ── */

async function init() {
  initPanels();
  initStaticIpToggle();
  bindControls();
  await Promise.all([
    refreshStatus().then(function () { if (state.status) updateSettingsInputs(state.status); }),
    refreshSchedule(),
    refreshNetwork(),
    refreshIrConfig(),
    refreshIrDiagnostics(),
    refreshOtaStatus(),
  ]);
  setInterval(function () {
    refreshStatus();
    refreshIrDiagnostics();
  }, 5000);
}

init().catch(function (e) {
  console.error(e);
  setDisconnected();
});

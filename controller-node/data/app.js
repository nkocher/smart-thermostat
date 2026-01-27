// Smart Thermostat Web UI

const API_BASE = '';
const POLL_INTERVAL = 5000;

// DOM Elements
const elements = {
    connectionStatus: document.getElementById('connection-status'),
    currentTemp: document.getElementById('current-temp'),
    currentHumidity: document.getElementById('current-humidity'),
    sensorStatus: document.getElementById('sensor-status'),
    targetTemp: document.getElementById('target-temp'),
    tempUp: document.getElementById('temp-up'),
    tempDown: document.getElementById('temp-down'),
    modeOff: document.getElementById('mode-off'),
    modeHeat: document.getElementById('mode-heat'),
    thermostatState: document.getElementById('thermostat-state'),
    fireplaceIndicator: document.getElementById('fireplace-indicator'),
    fireplaceStatusText: document.getElementById('fireplace-status-text'),
    fireplaceTemp: document.getElementById('fireplace-temp'),
    irOn: document.getElementById('ir-on'),
    irOff: document.getElementById('ir-off'),
    heatUp: document.getElementById('heat-up'),
    heatDown: document.getElementById('heat-down'),
    lightToggle: document.getElementById('light-toggle'),
    lightLevel: document.getElementById('light-level'),
    timerToggle: document.getElementById('timer-toggle'),
    timerValue: document.getElementById('timer-value'),
    hysteresis: document.getElementById('hysteresis'),
    saveHysteresis: document.getElementById('save-hysteresis'),
    fireplaceOffset: document.getElementById('fireplace-offset'),
    saveOffset: document.getElementById('save-offset'),
    settingsToggle: document.querySelector('.settings-toggle'),
    settingsCard: document.querySelector('.settings'),
    holdControl: document.getElementById('hold-control'),
    holdRemaining: document.getElementById('hold-remaining'),
    resumeAuto: document.getElementById('resume-auto')
};

// State
let currentState = {
    currentTemp: 0,
    currentHumidity: 0,
    targetTemp: 70,
    mode: 'OFF',
    state: 'IDLE',
    fireplaceOn: false,
    sensorValid: false,
    hysteresis: 2,
    fireplaceOffset: 4,
    lightLevel: 0,
    timerState: 0,
    timerString: 'OFF',
    holdActive: false,
    holdRemainingMin: 0
};

// API Functions
async function fetchStatus() {
    try {
        const response = await fetch(`${API_BASE}/api/status`);
        if (!response.ok) throw new Error('Network error');

        const data = await response.json();
        currentState = { ...currentState, ...data };
        updateUI();
        setConnectionStatus('connected');
    } catch (error) {
        console.error('Failed to fetch status:', error);
        setConnectionStatus('error');
    }
}

async function setTargetTemp(temp) {
    try {
        const response = await fetch(`${API_BASE}/api/target?value=${temp}`, {
            method: 'POST'
        });
        if (response.ok) {
            const data = await response.json();
            currentState = { ...currentState, ...data };
            updateUI();
        }
    } catch (error) {
        console.error('Failed to set target:', error);
    }
}

async function setMode(mode) {
    try {
        const response = await fetch(`${API_BASE}/api/mode?value=${mode}`, {
            method: 'POST'
        });
        if (response.ok) {
            const data = await response.json();
            currentState = { ...currentState, ...data };
            updateUI();
        }
    } catch (error) {
        console.error('Failed to set mode:', error);
    }
}

async function setHysteresis(value) {
    try {
        const response = await fetch(`${API_BASE}/api/hysteresis?value=${value}`, {
            method: 'POST'
        });
        if (response.ok) {
            const data = await response.json();
            currentState = { ...currentState, ...data };
            updateUI();
        }
    } catch (error) {
        console.error('Failed to set hysteresis:', error);
    }
}

async function setFireplaceOffset(value) {
    try {
        const response = await fetch(`${API_BASE}/api/offset?value=${value}`, {
            method: 'POST'
        });
        if (response.ok) {
            const data = await response.json();
            currentState = { ...currentState, ...data };
            updateUI();
        }
    } catch (error) {
        console.error('Failed to set offset:', error);
    }
}

async function sendIRCommand(command) {
    try {
        const response = await fetch(`${API_BASE}/api/ir/${command}`, {
            method: 'POST'
        });
        if (response.ok) {
            const data = await response.json();
            currentState = { ...currentState, ...data };
            updateUI();
        }
    } catch (error) {
        console.error('Failed to send IR command:', error);
    }
}

async function exitHold() {
    try {
        const response = await fetch(`${API_BASE}/api/hold/exit`, {
            method: 'POST'
        });
        if (response.ok) {
            const data = await response.json();
            currentState = { ...currentState, ...data };
            updateUI();
        }
    } catch (error) {
        console.error('Failed to exit hold:', error);
    }
}

// UI Functions
function setConnectionStatus(status) {
    elements.connectionStatus.className = 'status-indicator ' + status;
}

function updateUI() {
    // Current conditions
    elements.currentTemp.textContent = currentState.currentTemp.toFixed(1);
    elements.currentHumidity.textContent = currentState.currentHumidity.toFixed(0);

    // Sensor status
    if (currentState.sensorValid) {
        elements.sensorStatus.textContent = 'Sensor OK';
        elements.sensorStatus.className = 'sensor-status valid';
    } else {
        elements.sensorStatus.textContent = 'Sensor data stale';
        elements.sensorStatus.className = 'sensor-status';
    }

    // Target temperature
    elements.targetTemp.textContent = currentState.targetTemp.toFixed(0);

    // Mode buttons
    elements.modeOff.className = 'btn mode-btn' + (currentState.mode === 'OFF' ? ' active' : '');
    elements.modeHeat.className = 'btn mode-btn' + (currentState.mode === 'HEAT' ? ' active heat' : '');

    // Thermostat state
    elements.thermostatState.textContent = currentState.state;

    // Fireplace status
    elements.fireplaceIndicator.className = 'indicator' + (currentState.fireplaceOn ? ' on' : '');
    elements.fireplaceStatusText.textContent = currentState.fireplaceOn ? 'On' : 'Off';

    // Fireplace temperature
    if (elements.fireplaceTemp) {
        elements.fireplaceTemp.textContent = currentState.fireplaceTemp || 70;
    }

    // Hysteresis
    elements.hysteresis.value = currentState.hysteresis;

    // Fireplace offset
    if (elements.fireplaceOffset) {
        elements.fireplaceOffset.value = currentState.fireplaceOffset || 4;
    }

    // Light level display
    if (elements.lightLevel) {
        elements.lightLevel.textContent = currentState.lightLevel === 0 ? 'OFF' : currentState.lightLevel;
    }

    // Timer display
    if (elements.timerValue) {
        elements.timerValue.textContent = currentState.timerString || 'OFF';
    }

    // Hold control
    if (elements.holdControl) {
        if (currentState.holdActive) {
            elements.holdControl.style.display = 'flex';
            elements.holdRemaining.textContent = `Hold: ${currentState.holdRemainingMin} min`;
        } else {
            elements.holdControl.style.display = 'none';
        }
    }
}

// Event Handlers
function setupEventListeners() {
    // Temperature controls
    elements.tempUp.addEventListener('click', () => {
        const newTarget = currentState.targetTemp + 1;
        setTargetTemp(newTarget);
    });

    elements.tempDown.addEventListener('click', () => {
        const newTarget = currentState.targetTemp - 1;
        setTargetTemp(newTarget);
    });

    // Mode controls
    elements.modeOff.addEventListener('click', () => setMode('OFF'));
    elements.modeHeat.addEventListener('click', () => setMode('HEAT'));

    // Fireplace controls
    elements.irOn.addEventListener('click', () => sendIRCommand('on'));
    elements.irOff.addEventListener('click', () => sendIRCommand('off'));
    elements.heatUp.addEventListener('click', () => sendIRCommand('heat/up'));
    elements.heatDown.addEventListener('click', () => sendIRCommand('heat/down'));

    // Light toggle (cycles through levels)
    if (elements.lightToggle) {
        elements.lightToggle.addEventListener('click', () => sendIRCommand('light/toggle'));
    }

    // Timer toggle (cycles through timer values)
    if (elements.timerToggle) {
        elements.timerToggle.addEventListener('click', () => sendIRCommand('timer/toggle'));
    }

    // Settings
    elements.settingsToggle.addEventListener('click', () => {
        elements.settingsCard.classList.toggle('collapsed');
        elements.settingsToggle.textContent = elements.settingsCard.classList.contains('collapsed')
            ? 'Settings ▼'
            : 'Settings ▲';
    });

    elements.saveHysteresis.addEventListener('click', () => {
        const value = parseFloat(elements.hysteresis.value);
        if (value >= 0.5 && value <= 5) {
            setHysteresis(value);
        }
    });

    elements.saveOffset.addEventListener('click', () => {
        const value = parseInt(elements.fireplaceOffset.value);
        if (value >= 2 && value <= 10 && value % 2 === 0) {
            setFireplaceOffset(value);
        }
    });

    // Hold control
    if (elements.resumeAuto) {
        elements.resumeAuto.addEventListener('click', () => exitHold());
    }
}

// Initialize
function init() {
    setupEventListeners();
    fetchStatus();
    setInterval(fetchStatus, POLL_INTERVAL);
}

// Start when DOM is ready
if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
} else {
    init();
}

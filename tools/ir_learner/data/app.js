// IR Code Learner Web App

const statusBar = document.getElementById('statusBar');
const captureModal = document.getElementById('captureModal');
const exportModal = document.getElementById('exportModal');
const modalButtonName = document.getElementById('modalButtonName');
const exportCode = document.getElementById('exportCode');

let pollInterval = null;
let currentCapture = null;

// Initialize
document.addEventListener('DOMContentLoaded', () => {
    setupEventListeners();
    // Disable auto-load for debugging - click a button to manually load
    // loadCapturedCodes();
    console.log("Ready - click Capture to test");
});

function setupEventListeners() {
    // Capture buttons
    document.querySelectorAll('.btn-capture').forEach(btn => {
        btn.addEventListener('click', (e) => {
            const card = e.target.closest('.button-card');
            const buttonName = card.dataset.button;
            startCapture(buttonName);
        });
    });

    // Test buttons
    document.querySelectorAll('.btn-test').forEach(btn => {
        btn.addEventListener('click', (e) => {
            const card = e.target.closest('.button-card');
            const buttonName = card.dataset.button;
            testCode(buttonName);
        });
    });

    // Clear buttons
    document.querySelectorAll('.btn-clear').forEach(btn => {
        btn.addEventListener('click', (e) => {
            const card = e.target.closest('.button-card');
            const buttonName = card.dataset.button;
            clearCode(buttonName);
        });
    });

    // Cancel capture
    document.getElementById('cancelCapture').addEventListener('click', cancelCapture);

    // Export
    document.getElementById('exportBtn').addEventListener('click', exportConfig);

    // Copy to clipboard
    document.getElementById('copyBtn').addEventListener('click', copyToClipboard);

    // Close export modal
    document.getElementById('closeExport').addEventListener('click', () => {
        exportModal.classList.remove('active');
    });

    // Temperature sequence capture
    document.getElementById('captureTempBtn').addEventListener('click', captureTemperatureSequence);
}

async function loadCapturedCodes() {
    try {
        const response = await fetch('/api/codes');
        const data = await response.json();

        data.codes.forEach(code => {
            updateCardStatus(code.name, code.captured, code);
        });

        setStatus('Ready', 'ready');
    } catch (err) {
        console.error('Failed to load codes:', err);
        setStatus('Connection error', 'error');
    }
}

function updateCardStatus(buttonName, captured, codeInfo = null) {
    const card = document.querySelector(`[data-button="${buttonName}"]`);
    if (!card) return;

    const statusEl = card.querySelector('.button-status');
    const testBtn = card.querySelector('.btn-test');
    const clearBtn = card.querySelector('.btn-clear');

    if (captured) {
        card.classList.add('captured');
        statusEl.textContent = `Captured (${codeInfo?.protocol || 'RAW'}, ${codeInfo?.rawLen || '?'} values)`;
        testBtn.disabled = false;
        clearBtn.disabled = false;
    } else {
        card.classList.remove('captured');
        statusEl.textContent = 'Not captured';
        testBtn.disabled = true;
        clearBtn.disabled = true;
    }
}

async function startCapture(buttonName) {
    try {
        const formData = new FormData();
        formData.append('button', buttonName);

        const response = await fetch('/api/capture/start', {
            method: 'POST',
            body: formData
        });

        if (!response.ok) {
            throw new Error('Failed to start capture');
        }

        currentCapture = buttonName;

        // Update UI
        const card = document.querySelector(`[data-button="${buttonName}"]`);
        card.classList.add('capturing');

        // Show modal
        modalButtonName.textContent = formatButtonName(buttonName);
        captureModal.classList.add('active');

        setStatus(`Capturing: ${formatButtonName(buttonName)}`, 'capturing');

        // Start polling for capture completion
        pollInterval = setInterval(checkCaptureStatus, 200);
    } catch (err) {
        console.error('Capture error:', err);
        setStatus('Capture failed', 'error');
    }
}

async function checkCaptureStatus() {
    try {
        const response = await fetch('/api/status');
        const data = await response.json();

        if (data.newCode && !data.capturing) {
            // Capture complete!
            stopPolling();
            captureModal.classList.remove('active');

            // Reload codes to get updated info
            await loadCapturedCodes();

            const card = document.querySelector(`[data-button="${currentCapture}"]`);
            if (card) {
                card.classList.remove('capturing');
            }

            setStatus('Capture successful!', 'ready');
            currentCapture = null;
        }
    } catch (err) {
        console.error('Poll error:', err);
    }
}

async function cancelCapture() {
    try {
        await fetch('/api/capture/stop', { method: 'POST' });
    } catch (err) {
        console.error('Cancel error:', err);
    }

    stopPolling();
    captureModal.classList.remove('active');

    if (currentCapture) {
        const card = document.querySelector(`[data-button="${currentCapture}"]`);
        if (card) {
            card.classList.remove('capturing');
        }
    }

    setStatus('Capture cancelled', 'ready');
    currentCapture = null;
}

function stopPolling() {
    if (pollInterval) {
        clearInterval(pollInterval);
        pollInterval = null;
    }
}

async function testCode(buttonName) {
    try {
        const formData = new FormData();
        formData.append('button', buttonName);

        setStatus(`Testing: ${formatButtonName(buttonName)}`, 'capturing');

        const response = await fetch('/api/test', {
            method: 'POST',
            body: formData
        });

        if (!response.ok) {
            throw new Error('Test failed');
        }

        setStatus('IR signal sent!', 'ready');
    } catch (err) {
        console.error('Test error:', err);
        setStatus('Test failed', 'error');
    }
}

async function clearCode(buttonName) {
    if (!confirm(`Clear captured code for ${formatButtonName(buttonName)}?`)) {
        return;
    }

    try {
        const formData = new FormData();
        formData.append('button', buttonName);

        const response = await fetch('/api/codes/clear', {
            method: 'POST',
            body: formData
        });

        if (!response.ok) {
            throw new Error('Clear failed');
        }

        updateCardStatus(buttonName, false);
        setStatus('Code cleared', 'ready');
    } catch (err) {
        console.error('Clear error:', err);
        setStatus('Clear failed', 'error');
    }
}

async function exportConfig() {
    try {
        const response = await fetch('/api/export');
        const code = await response.text();

        exportCode.textContent = code;
        exportModal.classList.add('active');
    } catch (err) {
        console.error('Export error:', err);
        setStatus('Export failed', 'error');
    }
}

async function copyToClipboard() {
    try {
        await navigator.clipboard.writeText(exportCode.textContent);
        document.getElementById('copyBtn').textContent = 'Copied!';
        setTimeout(() => {
            document.getElementById('copyBtn').textContent = 'Copy to Clipboard';
        }, 2000);
    } catch (err) {
        console.error('Copy error:', err);
        // Fallback for older browsers
        const textarea = document.createElement('textarea');
        textarea.value = exportCode.textContent;
        document.body.appendChild(textarea);
        textarea.select();
        document.execCommand('copy');
        document.body.removeChild(textarea);
        document.getElementById('copyBtn').textContent = 'Copied!';
        setTimeout(() => {
            document.getElementById('copyBtn').textContent = 'Copy to Clipboard';
        }, 2000);
    }
}

function setStatus(text, state) {
    statusBar.querySelector('.status-text').textContent = text;
    statusBar.className = 'status-bar';
    if (state) {
        statusBar.classList.add(state);
    }
}

function formatButtonName(name) {
    return name.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
}

// Temperature sequence capture
async function captureTemperatureSequence() {
    const capturedCodes = [];
    const progressDiv = document.getElementById('tempProgress');
    const outputDiv = document.getElementById('tempOutput');
    const captureBtn = document.getElementById('captureTempBtn');

    // Disable button during capture
    captureBtn.disabled = true;
    outputDiv.classList.remove('active');
    outputDiv.textContent = '';

    try {
        // Capture UP transitions (60→62, 62→64, ... 78→80)
        progressDiv.textContent = 'Capturing TEMP UP transitions...';
        for (let temp = 60; temp < 80; temp += 2) {
            const nextTemp = temp + 2;
            const buttonName = `temp_up_from_${temp}`;

            progressDiv.textContent = `Set fireplace to ${temp}°F, then press TEMP UP (${temp}→${nextTemp})`;
            setStatus(`Temperature UP: ${temp}→${nextTemp}`, 'capturing');

            // Show modal with detailed instructions
            modalButtonName.textContent = `TEMP UP at ${temp}°F (goes to ${nextTemp}°F)`;
            captureModal.classList.add('active');

            const code = await captureTemperatureCode(buttonName);

            if (code) {
                capturedCodes.push({
                    type: 'up',
                    from: temp,
                    to: nextTemp,
                    raw: code.rawData,
                    rawLen: code.rawLen
                });
                progressDiv.textContent = `Captured UP ${temp}→${nextTemp} ✓`;
            } else {
                throw new Error(`Failed to capture UP ${temp}→${nextTemp}`);
            }

            captureModal.classList.remove('active');
            await sleep(500);
        }

        // Capture DOWN transitions (80→78, 78→76, ... 62→60)
        progressDiv.textContent = 'Capturing TEMP DOWN transitions...';
        for (let temp = 80; temp > 60; temp -= 2) {
            const nextTemp = temp - 2;
            const buttonName = `temp_down_from_${temp}`;

            progressDiv.textContent = `Set fireplace to ${temp}°F, then press TEMP DOWN (${temp}→${nextTemp})`;
            setStatus(`Temperature DOWN: ${temp}→${nextTemp}`, 'capturing');

            modalButtonName.textContent = `TEMP DOWN at ${temp}°F (goes to ${nextTemp}°F)`;
            captureModal.classList.add('active');

            const code = await captureTemperatureCode(buttonName);

            if (code) {
                capturedCodes.push({
                    type: 'down',
                    from: temp,
                    to: nextTemp,
                    raw: code.rawData,
                    rawLen: code.rawLen
                });
                progressDiv.textContent = `Captured DOWN ${temp}→${nextTemp} ✓`;
            } else {
                throw new Error(`Failed to capture DOWN ${temp}→${nextTemp}`);
            }

            captureModal.classList.remove('active');
            await sleep(500);
        }

        // All captured! Generate config.h code
        progressDiv.textContent = 'All temperature transitions captured! ✓';
        setStatus('Temperature capture complete', 'ready');

        const configCode = generateTempConfigCode(capturedCodes);
        outputDiv.textContent = configCode;
        outputDiv.classList.add('active');

    } catch (err) {
        console.error('Temperature capture error:', err);
        progressDiv.textContent = 'Capture failed: ' + err.message;
        setStatus('Temperature capture failed', 'error');
        captureModal.classList.remove('active');
    } finally {
        captureBtn.disabled = false;
    }
}

async function captureTemperatureCode(buttonName) {
    return new Promise((resolve, reject) => {
        let pollInterval = null;
        let timeout = null;

        const cleanup = () => {
            if (pollInterval) clearInterval(pollInterval);
            if (timeout) clearTimeout(timeout);
        };

        // Start capture on server
        fetch('/api/capture/start', {
            method: 'POST',
            body: new URLSearchParams({ button: buttonName })
        })
        .then(response => {
            if (!response.ok) throw new Error('Failed to start capture');

            // Poll for completion
            pollInterval = setInterval(async () => {
                try {
                    const statusResp = await fetch('/api/status');
                    const data = await statusResp.json();

                    if (data.newCode && !data.capturing) {
                        cleanup();

                        // Get the raw code data
                        const rawResp = await fetch(`/api/codes/raw?button=${buttonName}`);
                        const rawData = await rawResp.json();

                        resolve(rawData);
                    }
                } catch (err) {
                    cleanup();
                    reject(err);
                }
            }, 200);

            // 10 second timeout per button
            timeout = setTimeout(() => {
                cleanup();
                fetch('/api/capture/stop', { method: 'POST' });
                reject(new Error('Timeout waiting for IR signal'));
            }, 10000);
        })
        .catch(err => {
            cleanup();
            reject(err);
        });
    });
}

function generateTempConfigCode(capturedCodes) {
    let output = '// Temperature IR codes (state-dependent, like light codes)\n';
    output += '// UP transitions: 60→62, 62→64, ... 78→80\n';
    output += '// DOWN transitions: 80→78, 78→76, ... 62→60\n';
    output += '// Paste these into controller-node/src/config.h after line 174\n\n';

    // Sort codes: all UP transitions first, then all DOWN transitions
    const upCodes = capturedCodes.filter(c => c.type === 'up').sort((a, b) => a.from - b.from);
    const downCodes = capturedCodes.filter(c => c.type === 'down').sort((a, b) => b.from - a.from);

    // Generate UP transition codes
    output += '// TEMP UP transitions\n';
    upCodes.forEach(({ from, to, raw, rawLen }) => {
        output += `// ${from}°F → ${to}°F (press UP at ${from}°F)\n`;
        output += `const uint16_t IR_RAW_TEMP_UP_FROM_${from}[] = {\n    `;

        for (let i = 0; i < rawLen; i++) {
            output += raw[i];
            if (i < rawLen - 1) {
                output += ', ';
            }
            if ((i + 1) % 10 === 0 && i < rawLen - 1) {
                output += '\n    ';
            }
        }

        output += `\n};\n`;
        output += `const uint16_t IR_RAW_TEMP_UP_FROM_${from}_LEN = ${rawLen};\n\n`;
    });

    // Generate DOWN transition codes
    output += '// TEMP DOWN transitions\n';
    downCodes.forEach(({ from, to, raw, rawLen }) => {
        output += `// ${from}°F → ${to}°F (press DOWN at ${from}°F)\n`;
        output += `const uint16_t IR_RAW_TEMP_DOWN_FROM_${from}[] = {\n    `;

        for (let i = 0; i < rawLen; i++) {
            output += raw[i];
            if (i < rawLen - 1) {
                output += ', ';
            }
            if ((i + 1) % 10 === 0 && i < rawLen - 1) {
                output += '\n    ';
            }
        }

        output += `\n};\n`;
        output += `const uint16_t IR_RAW_TEMP_DOWN_FROM_${from}_LEN = ${rawLen};\n\n`;
    });

    return output;
}

function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

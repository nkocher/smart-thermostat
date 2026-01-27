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

import { CACHE_BYTES, MAX_CACHED_PAGES, MAX_ZOOM, MIN_ZOOM, WORKER_COUNT } from './config.js';
import { RenderCache } from './cache.js';
import { attachGestures } from './input.js';
import { RendererPool } from './renderer.js';
import { Scheduler } from './scheduler.js';
import { Viewer } from './viewer.js';

const elements = {
    fileSelector: document.getElementById('file-selector'),
    dropZone: document.getElementById('drop-zone'),
    fileInput: document.getElementById('file-input'),
    viewer: document.getElementById('viewer'),
    viewerControls: document.getElementById('viewer-controls'),
    homeButton: document.getElementById('home-button'),
    settingsButton: document.getElementById('settings-button'),
    settingsPopover: document.getElementById('settings-popover'),
    pagedMode: document.getElementById('paged-mode'),
    continuousMode: document.getElementById('continuous-mode'),
    prevPage: document.getElementById('prev-page'),
    nextPage: document.getElementById('next-page'),
    pageInput: document.getElementById('page-input'),
    pageTotal: document.getElementById('page-total'),
    zoomOut: document.getElementById('zoom-out'),
    zoomIn: document.getElementById('zoom-in'),
    zoomValue: document.getElementById('zoom-value'),
    viewport: document.getElementById('document-viewport'),
    stage: document.getElementById('document-stage'),
    themeOptions: document.querySelectorAll('[data-theme-choice]'),
    themeColor: document.querySelector('meta[name="theme-color"]'),
    dropOverlay: document.getElementById('drop-overlay'),
    loadingOverlay: document.getElementById('loading-overlay'),
    loadingText: document.getElementById('loading-text'),
    toast: document.getElementById('toast'),
};

const systemTheme = matchMedia('(prefers-color-scheme: dark)');
let themePreference = localStorage.getItem('hayro-theme') || 'system';
let toastTimer = null;
let fileName = '';
let openCount = 0;

function showToast(message) {
    clearTimeout(toastTimer);
    elements.toast.textContent = message;
    elements.toast.hidden = false;
    toastTimer = setTimeout(() => {
        elements.toast.hidden = true;
    }, 4500);
}

function showLoading(text) {
    elements.loadingText.textContent = text;
    elements.loadingOverlay.hidden = false;
}

function hideLoading() {
    elements.loadingOverlay.hidden = true;
}

const cache = new RenderCache(CACHE_BYTES, MAX_CACHED_PAGES);
const pool = new RendererPool(WORKER_COUNT, {
    onIdle: () => scheduler.pump(),
    onCrash: () => {
        hideLoading();
        showToast('The renderer stopped unexpectedly. Please reopen the PDF.');
    },
});
const scheduler = new Scheduler(pool, cache, {
    onRendered: page => viewer.handleRendered(page),
    onFailed: page => viewer.handleRendered(page),
});
const viewer = new Viewer(elements, cache, scheduler, updateControls);

function updateControls() {
    const pageCount = viewer.pageCount || 1;
    elements.pageInput.value = String(viewer.page);
    elements.pageInput.max = String(pageCount);
    elements.pageTotal.textContent = `/ ${pageCount}`;
    elements.prevPage.disabled = viewer.page <= 1;
    elements.nextPage.disabled = viewer.page >= viewer.pageCount;
    elements.zoomValue.textContent = `${Math.round(viewer.zoom * 100)}%`;
    elements.zoomOut.disabled = viewer.targetZoom <= MIN_ZOOM;
    elements.zoomIn.disabled = viewer.targetZoom >= MAX_ZOOM;

    const paged = viewer.mode === 'paged';
    elements.pagedMode.classList.toggle('active', paged);
    elements.continuousMode.classList.toggle('active', !paged);
    elements.pagedMode.setAttribute('aria-pressed', String(paged));
    elements.continuousMode.setAttribute('aria-pressed', String(!paged));
}

function applyTheme(preference) {
    themePreference = ['light', 'dark', 'system'].includes(preference) ? preference : 'system';
    const resolved = themePreference === 'system' ? (systemTheme.matches ? 'dark' : 'light') : themePreference;

    document.documentElement.dataset.theme = resolved;
    localStorage.setItem('hayro-theme', themePreference);
    elements.themeOptions.forEach(option => {
        const active = option.dataset.themeChoice === themePreference;
        option.classList.toggle('active', active);
        option.setAttribute('aria-pressed', String(active));
    });
    elements.themeColor.content = resolved === 'dark' ? '#181818' : '#f4f4f4';
}

function setSettingsOpen(open) {
    elements.settingsPopover.hidden = !open;
    elements.settingsButton.setAttribute('aria-expanded', String(open));
}

function setMode(mode) {
    localStorage.setItem('hayro-view-mode', mode);
    viewer.setMode(mode);
}

function showViewer() {
    elements.fileSelector.hidden = true;
    elements.viewer.hidden = false;
    elements.viewerControls.hidden = false;
    document.title = `${fileName} — Hayro`;
}

function showFileSelector() {
    viewer.close();
    scheduler.reset();
    elements.viewer.hidden = true;
    elements.viewerControls.hidden = true;
    elements.fileSelector.hidden = false;
    document.title = 'Hayro demo';
}

function isPdf(file) {
    return file?.type === 'application/pdf' || file?.name.toLowerCase().endsWith('.pdf');
}

async function openFile(file) {
    if (!isPdf(file)) {
        showToast('Please choose a PDF file.');
        return;
    }

    const current = ++openCount;
    elements.fileInput.value = '';

    try {
        showLoading('Uploading PDF');
        const buffer = await file.arrayBuffer();
        if (current !== openCount) return;

        showLoading('Loading PDF');
        viewer.close();
        scheduler.reset();
        const loaded = await pool.load(buffer);
        if (current !== openCount) return;

        fileName = file.name;
        document.documentElement.dataset.wasmVariant = loaded.variant;
        showViewer();
        viewer.open(loaded);
        hideLoading();
    } catch (error) {
        if (current !== openCount) return;
        hideLoading();
        if (error instanceof DOMException) {
            showToast(`Could not read ${file.name}.`);
        } else {
            showFileSelector();
            showToast('This PDF could not be opened. It may be encrypted or damaged.');
        }
        console.error(error);
    }
}

function preventDefaults(event) {
    event.preventDefault();
    event.stopPropagation();
}

function attachFileHandling() {
    elements.dropZone.addEventListener('click', () => elements.fileInput.click());
    elements.fileInput.addEventListener('change', () => {
        if (elements.fileInput.files.length > 0) openFile(elements.fileInput.files[0]);
    });

    document.addEventListener('dragenter', event => {
        preventDefaults(event);
        if (viewer.loaded) {
            elements.dropOverlay.hidden = false;
        } else {
            elements.dropZone.classList.add('dragover');
        }
    });
    document.addEventListener('dragover', preventDefaults);
    document.addEventListener('dragleave', event => {
        preventDefaults(event);
        if (!event.relatedTarget) {
            elements.dropOverlay.hidden = true;
            elements.dropZone.classList.remove('dragover');
        }
    });
    document.addEventListener('drop', event => {
        preventDefaults(event);
        elements.dropOverlay.hidden = true;
        elements.dropZone.classList.remove('dragover');
        if (event.dataTransfer.files.length > 0) openFile(event.dataTransfer.files[0]);
    });
}

function attachControls() {
    elements.settingsButton.addEventListener('click', () => setSettingsOpen(elements.settingsPopover.hidden));
    document.addEventListener('pointerdown', event => {
        if (elements.settingsPopover.hidden) return;
        if (elements.settingsPopover.contains(event.target) || event.target === elements.settingsButton) return;
        setSettingsOpen(false);
    });
    elements.themeOptions.forEach(option => {
        option.addEventListener('click', () => applyTheme(option.dataset.themeChoice));
    });
    systemTheme.addEventListener('change', () => {
        if (themePreference === 'system') applyTheme('system');
    });

    elements.pagedMode.addEventListener('click', () => setMode('paged'));
    elements.continuousMode.addEventListener('click', () => setMode('continuous'));
    elements.prevPage.addEventListener('click', () => viewer.setPage(viewer.page - 1));
    elements.nextPage.addEventListener('click', () => viewer.setPage(viewer.page + 1));
    elements.zoomOut.addEventListener('click', () => viewer.zoomStep(-1));
    elements.zoomIn.addEventListener('click', () => viewer.zoomStep(1));
    elements.zoomValue.addEventListener('click', () => viewer.setZoom(1));
    elements.homeButton.addEventListener('click', () => {
        if (viewer.loaded) showFileSelector();
    });

    elements.pageInput.addEventListener('change', () => {
        const page = Number(elements.pageInput.value);
        if (Number.isFinite(page)) viewer.setPage(page);
        updateControls();
    });
    elements.pageInput.addEventListener('keydown', event => {
        if (event.key === 'Enter') event.currentTarget.blur();
    });

    document.addEventListener('keydown', event => {
        if (event.key === 'Escape' && !elements.settingsPopover.hidden) {
            setSettingsOpen(false);
            elements.settingsButton.focus();
            return;
        }
        if (!viewer.loaded || event.target instanceof HTMLInputElement) return;

        const modifier = event.ctrlKey || event.metaKey;
        if (viewer.mode === 'paged' && event.key === 'ArrowLeft') {
            viewer.setPage(viewer.page - 1);
        } else if (viewer.mode === 'paged' && event.key === 'ArrowRight') {
            viewer.setPage(viewer.page + 1);
        } else if (modifier && (event.key === '+' || event.key === '=')) {
            viewer.zoomStep(1);
        } else if (modifier && event.key === '-') {
            viewer.zoomStep(-1);
        } else if (modifier && event.key === '0') {
            viewer.setZoom(1);
        } else {
            return;
        }
        event.preventDefault();
    });

    window.addEventListener('resize', () => viewer.handleResize());
}

applyTheme(themePreference);
viewer.setMode(localStorage.getItem('hayro-view-mode') === 'paged' ? 'paged' : 'continuous');
attachFileHandling();
attachControls();
attachGestures(elements.viewport, viewer);

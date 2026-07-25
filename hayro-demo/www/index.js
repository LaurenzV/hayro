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
    pagedDocument: document.getElementById('paged-document'),
    pagedShell: document.querySelector('.paged-shell'),
    pagedCanvas: document.getElementById('pdf-canvas'),
    continuousDocument: document.getElementById('continuous-document'),
    themeOptions: document.querySelectorAll('[data-theme-choice]'),
    dropOverlay: document.getElementById('drop-overlay'),
    loadingOverlay: document.getElementById('loading-overlay'),
    toast: document.getElementById('toast'),
};

const systemThemeQuery = matchMedia('(prefers-color-scheme: dark)');

const state = {
    worker: null,
    generation: 0,
    requestId: 0,
    loaded: false,
    fileName: '',
    mode: localStorage.getItem('hayro-view-mode') === 'paged' ? 'paged' : 'continuous',
    page: 1,
    totalPages: 0,
    dimensions: [],
    zoom: 1,
    committedZoom: 1,
    targetZoom: 1,
    zoomFrame: null,
    zoomAnchor: null,
    rendered: new Map(),
    cacheBytes: 0,
    cacheClock: 0,
    pending: new Map(),
    renderFailures: new Map(),
    renderTimer: null,
    resizeTimer: null,
    scrollFrame: null,
    scrollEndTimer: null,
    toastTimer: null,
    observer: null,
    visiblePages: new Set(),
    scrolling: false,
    visiblePage: 1,
    drag: null,
    pinch: null,
    swipe: null,
    canPanX: false,
    canPanY: false,
    layoutViewportWidth: window.innerWidth,
    layoutPixelRatio: Math.max(window.devicePixelRatio || 1, 1),
    pagedBaseWidth: 0,
    pagedBaseHeight: 0,
    continuousBaseWidth: 0,
    continuousBaseHeight: 0,
    themePreference: localStorage.getItem('hayro-theme') || 'system',
};

const CONSTRAINED_DEVICE = matchMedia('(pointer: coarse)').matches
    || (navigator.deviceMemory && navigator.deviceMemory <= 4)
    || navigator.hardwareConcurrency <= 4;
const MAX_RENDER_CACHE_BYTES = (CONSTRAINED_DEVICE ? 96 : 192) * 1024 * 1024;
const MAX_CACHED_PAGES = CONSTRAINED_DEVICE ? 8 : 24;
const MAX_BUFFER_PAGES = CONSTRAINED_DEVICE ? 1 : 3;
const ZOOM_STEPS = [0.1, 0.25, 0.5, 0.67, 0.8, 1, 1.25, 1.5, 2, 2.5, 3, 4];
const SIMD_PROBE = new Uint8Array([
    0, 97, 115, 109, 1, 0, 0, 0,
    1, 4, 1, 96, 0, 0,
    3, 2, 1, 0,
    10, 9, 1, 7, 0, 65, 0, 253, 15, 26, 11,
]);

function supportsWasmSimd() {
    try {
        return WebAssembly.validate(SIMD_PROBE);
    } catch {
        return false;
    }
}

function createWorker() {
    const variant = supportsWasmSimd() ? 'simd' : 'nosimd';
    const workerUrl = new URL('./renderer.worker.js', import.meta.url);
    workerUrl.searchParams.set('variant', variant);
    document.documentElement.dataset.wasmVariant = variant;
    const worker = new Worker(workerUrl, { type: 'module' });
    worker.addEventListener('message', handleWorkerMessage);
    worker.addEventListener('error', () => {
        hideLoading();
        showToast('The renderer stopped unexpectedly. Please reopen the PDF.');
    });
    return worker;
}

function showLoading() {
    elements.loadingOverlay.hidden = false;
}

function hideLoading() {
    elements.loadingOverlay.hidden = true;
}

function showToast(message) {
    clearTimeout(state.toastTimer);
    elements.toast.textContent = message;
    elements.toast.hidden = false;
    state.toastTimer = setTimeout(() => {
        elements.toast.hidden = true;
    }, 4500);
}

function applyTheme(preference) {
    const selected = ['light', 'dark', 'system'].includes(preference) ? preference : 'system';
    const resolved = selected === 'system'
        ? (systemThemeQuery.matches ? 'dark' : 'light')
        : selected;

    state.themePreference = selected;
    document.documentElement.dataset.theme = resolved;
    localStorage.setItem('hayro-theme', selected);
    elements.themeOptions.forEach(option => {
        const active = option.dataset.themeChoice === selected;
        option.classList.toggle('active', active);
        option.setAttribute('aria-pressed', String(active));
    });
    document.querySelector('meta[name="theme-color"]').content = resolved === 'dark'
        ? '#181818'
        : '#f4f4f4';
}

function setSettingsOpen(open) {
    elements.settingsPopover.hidden = !open;
    elements.settingsButton.setAttribute('aria-expanded', String(open));
}

function updateModeControls() {
    const paged = state.mode === 'paged';
    elements.pagedMode.classList.toggle('active', paged);
    elements.continuousMode.classList.toggle('active', !paged);
    elements.pagedMode.setAttribute('aria-pressed', String(paged));
    elements.continuousMode.setAttribute('aria-pressed', String(!paged));
}

function resetPagePlaceholder(shell) {
    const placeholder = shell.querySelector('.page-placeholder');
    const spinner = document.createElement('span');
    spinner.className = 'spinner page-spinner';
    spinner.setAttribute('aria-hidden', 'true');
    placeholder.replaceChildren(spinner);
    placeholder.setAttribute('role', 'status');
    placeholder.setAttribute('aria-label', 'Rendering page');
}

function isPdf(file) {
    return file?.type === 'application/pdf' || file?.name.toLowerCase().endsWith('.pdf');
}

function clearContinuousCanvas(page, cacheKey) {
    const shell = elements.continuousDocument.querySelector(`[data-page="${page}"]`);
    const canvas = shell?.querySelector('canvas');
    if (!canvas || canvas.dataset.cacheKey !== String(cacheKey)) return;

    canvas.width = 1;
    canvas.height = 1;
    delete canvas.dataset.cacheKey;
    shell.classList.remove('rendered', 'error');
    resetPagePlaceholder(shell);
}

function removeCachedPage(page) {
    const rendered = state.rendered.get(page);
    if (!rendered) return;

    rendered.bitmap?.close();
    if (rendered.mode === 'continuous') {
        clearContinuousCanvas(page, rendered.cacheKey);
    }
    state.cacheBytes = Math.max(0, state.cacheBytes - rendered.bytes);
    state.rendered.delete(page);
}

function clearRenderedPages() {
    for (const page of [...state.rendered.keys()]) {
        removeCachedPage(page);
    }
    state.cacheBytes = 0;
}

function touchCachedPage(rendered) {
    state.cacheClock += 1;
    rendered.lastUsed = state.cacheClock;
}

function getRenderPixelRatio() {
    return Math.max(window.devicePixelRatio || 1, 1);
}

function estimatePageCacheBytes(page = state.page) {
    if (!state.loaded) return 1;
    const size = getBasePageSize(page);
    const pixelRatio = getRenderPixelRatio();
    const width = Math.max(1, Math.ceil(size.width * state.zoom * pixelRatio));
    const height = Math.max(1, Math.ceil(size.height * state.zoom * pixelRatio));
    return width * height * 4 * 2;
}

function getCachePageLimit() {
    const memoryLimitedPages = Math.floor(
        MAX_RENDER_CACHE_BYTES / estimatePageCacheBytes(),
    );
    return Math.max(1, Math.min(MAX_CACHED_PAGES, memoryLimitedPages));
}

function getBufferPageCount() {
    return Math.min(MAX_BUFFER_PAGES, Math.floor((getCachePageLimit() - 1) / 2));
}

function enforceCacheLimit() {
    const pageLimit = getCachePageLimit();

    while (
        state.rendered.size > pageLimit
        || state.cacheBytes > MAX_RENDER_CACHE_BYTES
    ) {
        const candidate = [...state.rendered.entries()]
            .filter(([page]) => page !== state.page && !state.visiblePages.has(page))
            .sort((left, right) => left[1].lastUsed - right[1].lastUsed)[0];
        if (!candidate) break;
        removeCachedPage(candidate[0]);
    }
}

async function handleFile(file) {
    if (!isPdf(file)) {
        showToast('Please choose a PDF file.');
        return;
    }

    try {
        showLoading();
        const buffer = await file.arrayBuffer();
        state.generation += 1;
        state.loaded = false;
        state.fileName = file.name;
        clearRenderedPages();
        state.pending.clear();
        state.renderFailures.clear();
        state.worker.postMessage({
            type: 'load',
            generation: state.generation,
            buffer,
        }, [buffer]);
    } catch (error) {
        hideLoading();
        showToast(`Could not read ${file.name}.`);
        console.error(error);
    } finally {
        elements.fileInput.value = '';
    }
}

function handleWorkerMessage({ data }) {
    if (data.generation !== state.generation) {
        data.bitmap?.close();
        return;
    }

    if (data.type === 'loaded') {
        document.documentElement.dataset.wasmVariant = data.variant;
        state.loaded = true;
        state.totalPages = data.totalPages;
        state.dimensions = [];
        for (let index = 0; index < data.dimensions.length; index += 2) {
            state.dimensions.push({
                width: data.dimensions[index],
                height: data.dimensions[index + 1],
            });
        }
        state.page = 1;
        state.visiblePage = 1;
        state.zoom = 1;
        state.committedZoom = 1;
        state.targetZoom = 1;
        buildContinuousPages();
        showViewer();
        switchMode(state.mode, false);
        hideLoading();
        return;
    }

    if (data.type === 'load-error') {
        hideLoading();
        showToast('This PDF could not be opened. It may be encrypted or damaged.');
        console.error(data.message);
        return;
    }

    if (data.type === 'rendered') {
        const pending = state.pending.get(data.page);
        if (!pending || pending.requestId !== data.requestId) {
            data.bitmap?.close();
            refillContinuousRenderQueue();
            return;
        }
        state.pending.delete(data.page);

        if (Math.abs(data.zoom - state.zoom) > 0.001) {
            data.bitmap?.close();
            refillContinuousRenderQueue();
            return;
        }

        state.renderFailures.delete(data.page);
        const imageData = data.buffer
            ? new ImageData(new Uint8ClampedArray(data.buffer), data.width, data.height)
            : null;
        removeCachedPage(data.page);
        const rendered = {
            imageData,
            bitmap: data.bitmap,
            width: data.width,
            height: data.height,
            zoom: data.zoom,
            mode: state.mode,
            bytes: data.width * data.height * 4 * 2,
            cacheKey: data.requestId,
            lastUsed: 0,
        };
        touchCachedPage(rendered);
        state.rendered.set(data.page, rendered);
        state.cacheBytes += rendered.bytes;
        if (data.page === state.page) commitRenderedZoom(data.zoom);
        paintPage(data.page);
        enforceCacheLimit();
        refillContinuousRenderQueue();
        return;
    }

    if (data.type === 'render-error') {
        const pending = state.pending.get(data.page);
        if (!pending || pending.requestId !== data.requestId) {
            refillContinuousRenderQueue();
            return;
        }
        state.pending.delete(data.page);
        state.renderFailures.set(data.page, {
            zoom: pending.zoom,
            mode: pending.mode,
        });
        markPageError(data.page);
        console.error(data.message);
        refillContinuousRenderQueue();
    }
}

function showViewer() {
    elements.fileSelector.hidden = true;
    elements.viewer.hidden = false;
    elements.viewerControls.hidden = false;
    document.title = `${state.fileName} — Hayro`;
    updateControls();
}

function showFileSelector() {
    elements.viewer.hidden = true;
    elements.viewerControls.hidden = true;
    elements.fileSelector.hidden = false;
    document.title = 'Hayro demo';
}

function switchMode(mode, persist = true) {
    state.mode = mode === 'continuous' ? 'continuous' : 'paged';
    if (persist) localStorage.setItem('hayro-view-mode', state.mode);
    updateModeControls();
    if (!state.loaded) return;

    if (state.zoomFrame !== null) {
        cancelAnimationFrame(state.zoomFrame);
        state.zoomFrame = null;
        state.targetZoom = state.zoom;
    }

    if (state.pending.size > 0) {
        state.worker.postMessage({ type: 'cancel' });
        state.pending.clear();
    }
    clearRenderedPages();
    state.renderFailures.clear();
    state.observer?.disconnect();
    state.visiblePages.clear();
    state.scrolling = false;
    clearTimeout(state.scrollEndTimer);
    elements.stage.classList.toggle('paged', state.mode === 'paged');
    elements.stage.classList.toggle('continuous', state.mode === 'continuous');
    elements.pagedDocument.hidden = state.mode !== 'paged';
    elements.continuousDocument.hidden = state.mode !== 'continuous';
    elements.stage.style.width = '';
    elements.stage.style.height = '';
    elements.viewport.scrollLeft = 0;
    elements.viewport.scrollTop = 0;

    updatePageLayouts();
    if (state.mode === 'paged') {
        renderPagedPage();
    } else {
        observeContinuousPages();
        requestVisiblePages();
        requestAnimationFrame(() => scrollToPage(state.page, false));
    }
    updateControls();
}

function getViewportMetrics() {
    const horizontalGutter = window.innerWidth <= 680 ? 24 : 48;
    const verticalGutter = window.innerWidth <= 680 ? 72 : 88;
    return {
        width: Math.max(200, Math.min(elements.viewport.clientWidth - horizontalGutter, 1240)),
        height: Math.max(200, elements.viewport.clientHeight - verticalGutter),
    };
}

function getBasePageSize(page) {
    const dimensions = state.dimensions[page - 1] || { width: 612, height: 792 };
    const viewport = getViewportMetrics();

    if (state.mode === 'continuous') {
        const width = viewport.width;
        return {
            width,
            height: width * dimensions.height / dimensions.width,
        };
    }

    const scale = Math.min(
        viewport.width / dimensions.width,
        viewport.height / dimensions.height,
    );
    return {
        width: dimensions.width * scale,
        height: dimensions.height * scale,
    };
}

function updatePageLayouts() {
    if (!state.loaded) return;

    if (state.mode === 'paged') {
        const size = getBasePageSize(state.page);
        state.pagedBaseWidth = size.width;
        state.pagedBaseHeight = size.height;
        const layoutSize = {
            width: size.width * state.committedZoom,
            height: size.height * state.committedZoom,
        };
        elements.pagedShell.style.width = `${layoutSize.width}px`;
        elements.pagedShell.style.height = `${layoutSize.height}px`;
        scaleExistingCanvas(elements.pagedCanvas, state.page, layoutSize);
        updatePagedTransform();
        return;
    }

    elements.continuousDocument.querySelectorAll('.page-shell').forEach(shell => {
        const page = Number(shell.dataset.page);
        const size = getBasePageSize(page);
        const layoutSize = {
            width: size.width * state.committedZoom,
            height: size.height * state.committedZoom,
        };
        shell.style.width = `${layoutSize.width}px`;
        shell.style.height = `${layoutSize.height}px`;
        scaleExistingCanvas(shell.querySelector('canvas'), page, layoutSize);
    });
    const gap = window.innerWidth <= 680 ? 16 : 24;
    elements.continuousDocument.style.gap = `${gap * state.committedZoom}px`;
    state.continuousBaseWidth = elements.continuousDocument.offsetWidth;
    state.continuousBaseHeight = elements.continuousDocument.offsetHeight;
    updateContinuousTransform();
}

function scaleExistingCanvas(canvas, page, size) {
    const rendered = state.rendered.get(page);
    canvas.style.width = `${size.width}px`;
    canvas.style.height = `${size.height}px`;
    canvas.classList.toggle('soft', Boolean(rendered && Math.abs(rendered.zoom - state.zoom) > 0.001));
}

function updateViewportOverflow() {
    if (state.mode === 'continuous') {
        updateContinuousTransform();
        return;
    }
    updatePagedTransform();
}

function updatePagedTransform() {
    if (state.mode !== 'paged' || state.pagedBaseWidth === 0) return;

    const gutter = window.innerWidth <= 680 ? 24 : 48;
    const scaledWidth = state.pagedBaseWidth * state.zoom;
    const scaledHeight = state.pagedBaseHeight * state.zoom;
    const layoutWidth = state.pagedBaseWidth * state.committedZoom;
    const layoutHeight = state.pagedBaseHeight * state.committedZoom;
    const displayScale = state.zoom / state.committedZoom;
    let stageWidth = 0;
    let stageHeight = 0;

    for (let pass = 0; pass < 2; pass += 1) {
        stageWidth = Math.max(elements.viewport.clientWidth, scaledWidth + gutter);
        stageHeight = Math.max(elements.viewport.clientHeight, scaledHeight + gutter);
        elements.stage.style.width = `${stageWidth}px`;
        elements.stage.style.height = `${stageHeight}px`;
        state.canPanX = stageWidth > elements.viewport.clientWidth + 0.5;
        state.canPanY = stageHeight > elements.viewport.clientHeight + 0.5;
        elements.viewport.style.overflowX = state.canPanX ? 'auto' : 'hidden';
        elements.viewport.style.overflowY = state.canPanY ? 'auto' : 'hidden';
    }

    elements.pagedDocument.style.width = `${layoutWidth}px`;
    elements.pagedDocument.style.height = `${layoutHeight}px`;
    elements.pagedDocument.style.left = `${(stageWidth - scaledWidth) / 2}px`;
    elements.pagedDocument.style.top = `${(stageHeight - scaledHeight) / 2}px`;
    elements.pagedDocument.style.transform = `scale(${displayScale})`;

    if (!state.canPanX) elements.viewport.scrollLeft = 0;
    if (!state.canPanY) elements.viewport.scrollTop = 0;
    elements.viewport.classList.toggle('can-pan', state.canPanX || state.canPanY);
}

function updateContinuousTransform() {
    if (state.mode !== 'continuous' || state.continuousBaseWidth === 0) return;

    const sideMargin = window.innerWidth <= 680 ? 24 : 48;
    const topMargin = window.innerWidth <= 680 ? 18 : 28;
    const bottomMargin = window.innerWidth <= 680 ? 60 : 72;
    const displayScale = state.zoom / state.committedZoom;
    const scaledWidth = state.continuousBaseWidth * displayScale;
    const scaledHeight = state.continuousBaseHeight * displayScale;
    const stageWidth = Math.max(elements.viewport.clientWidth, scaledWidth + sideMargin);
    const stageHeight = Math.max(
        elements.viewport.clientHeight,
        scaledHeight + topMargin + bottomMargin,
    );

    elements.stage.style.width = `${stageWidth}px`;
    elements.stage.style.height = `${stageHeight}px`;
    elements.continuousDocument.style.left = `${(stageWidth - scaledWidth) / 2}px`;
    elements.continuousDocument.style.top = `${topMargin}px`;
    elements.continuousDocument.style.transform = `scale(${displayScale})`;
    elements.viewport.style.overflowX = 'auto';
    elements.viewport.style.overflowY = 'auto';

    state.canPanX = stageWidth > elements.viewport.clientWidth + 0.5;
    state.canPanY = stageHeight > elements.viewport.clientHeight + 0.5;
    elements.viewport.classList.toggle('can-pan', state.canPanX || state.canPanY);
}

function renderPagedPage() {
    if (!state.loaded) return;

    elements.pagedShell.dataset.page = String(state.page);
    elements.pagedCanvas.setAttribute('aria-label', `Rendered PDF page ${state.page}`);
    elements.pagedCanvas.width = 1;
    elements.pagedCanvas.height = 1;
    elements.pagedShell.classList.remove('rendered', 'error');
    resetPagePlaceholder(elements.pagedShell);
    updatePageLayouts();
    updateControls();
    requestPage(state.page);
}

function buildContinuousPages() {
    elements.continuousDocument.replaceChildren();
    const fragment = document.createDocumentFragment();

    for (let page = 1; page <= state.totalPages; page += 1) {
        const shell = document.createElement('div');
        shell.className = 'page-shell continuous-shell';
        shell.dataset.page = String(page);

        const canvas = document.createElement('canvas');
        canvas.setAttribute('aria-label', `Rendered PDF page ${page}`);

        const placeholder = document.createElement('div');
        placeholder.className = 'page-placeholder';

        const badge = document.createElement('span');
        badge.className = 'page-number';
        badge.textContent = String(page);

        shell.append(canvas, placeholder, badge);
        resetPagePlaceholder(shell);
        fragment.append(shell);
    }

    elements.continuousDocument.append(fragment);
}

function observeContinuousPages() {
    state.observer = new IntersectionObserver(entries => {
        updateVisiblePage();
        for (const entry of entries) {
            const page = Number(entry.target.dataset.page);
            if (entry.isIntersecting) {
                state.visiblePages.add(page);
                if (!state.scrolling || !entry.target.classList.contains('rendered')) {
                    requestPage(page);
                }
            } else {
                state.visiblePages.delete(page);
            }
        }
        enforceCacheLimit();
    }, {
        root: elements.viewport,
        rootMargin: '600px 0px',
        threshold: 0.01,
    });

    elements.continuousDocument.querySelectorAll('.page-shell').forEach(shell => {
        state.observer.observe(shell);
    });
}

function requestVisiblePages() {
    const pages = [state.page];
    const visiblePages = [...state.visiblePages]
        .sort((left, right) => Math.abs(left - state.page) - Math.abs(right - state.page));
    pages.push(...visiblePages);

    for (let distance = 1; distance <= getBufferPageCount(); distance += 1) {
        pages.push(state.page + distance, state.page - distance);
    }

    new Set(pages).forEach(page => requestPage(page));
}

function refillContinuousRenderQueue() {
    if (state.mode === 'continuous' && !state.scrolling) {
        requestVisiblePages();
    }
}

function requestPage(page) {
    if (!state.loaded || page < 1 || page > state.totalPages) return;

    const existing = state.rendered.get(page);
    if (existing && existing.mode === state.mode && Math.abs(existing.zoom - state.zoom) < 0.001) {
        touchCachedPage(existing);
        if (page === state.page) commitRenderedZoom(existing.zoom);
        paintPage(page);
        return;
    }

    const failure = state.renderFailures.get(page);
    if (
        failure
        && failure.mode === state.mode
        && Math.abs(failure.zoom - state.zoom) < 0.001
    ) {
        return;
    }
    state.renderFailures.delete(page);

    const pending = state.pending.get(page);
    if (pending && Math.abs(pending.zoom - state.zoom) < 0.001 && pending.mode === state.mode) return;
    if (page !== state.page && state.pending.size >= getCachePageLimit()) return;

    const shell = state.mode === 'paged'
        ? elements.pagedShell
        : elements.continuousDocument.querySelector(`[data-page="${page}"]`);
    if (shell?.classList.contains('error')) {
        shell.classList.remove('error');
        resetPagePlaceholder(shell);
    }

    const viewport = getViewportMetrics();
    const requestId = ++state.requestId;
    state.pending.set(page, {
        requestId,
        zoom: state.zoom,
        mode: state.mode,
    });
    state.worker.postMessage({
        type: 'render',
        generation: state.generation,
        requestId,
        page,
        viewportWidth: viewport.width,
        viewportHeight: viewport.height,
        devicePixelRatio: getRenderPixelRatio(),
        zoom: state.zoom,
        fitWidth: state.mode === 'continuous',
        priority: page === state.page,
    });
}

function paintPage(page) {
    const rendered = state.rendered.get(page);
    if (!rendered) return;

    const shell = state.mode === 'paged'
        ? elements.pagedShell
        : elements.continuousDocument.querySelector(`[data-page="${page}"]`);
    if (!shell) return;

    const canvas = shell.querySelector('canvas');
    if (canvas.dataset.cacheKey === String(rendered.cacheKey)) {
        touchCachedPage(rendered);
        return;
    }
    const context = canvas.getContext('2d', {
        alpha: false,
        desynchronized: true,
    });

    canvas.width = rendered.width;
    canvas.height = rendered.height;
    if (rendered.bitmap) {
        context.drawImage(rendered.bitmap, 0, 0);
    } else {
        context.putImageData(rendered.imageData, 0, 0);
    }
    const size = {
        width: shell.offsetWidth,
        height: shell.offsetHeight,
    };
    canvas.style.width = `${size.width}px`;
    canvas.style.height = `${size.height}px`;
    canvas.dataset.cacheKey = String(rendered.cacheKey);
    canvas.classList.remove('soft');
    shell.classList.add('rendered');
    shell.classList.remove('error');
    touchCachedPage(rendered);
}

function markPageError(page) {
    const shell = state.mode === 'paged'
        ? elements.pagedShell
        : elements.continuousDocument.querySelector(`[data-page="${page}"]`);
    if (!shell) return;
    if (shell.classList.contains('rendered')) return;
    shell.classList.add('error');
    const placeholder = shell.querySelector('.page-placeholder');
    placeholder.removeAttribute('role');
    placeholder.removeAttribute('aria-label');
    placeholder.textContent = 'Page could not be rendered';
}

function setPage(page, scroll = true) {
    const nextPage = Math.max(1, Math.min(state.totalPages, Math.round(page)));
    if (nextPage === state.page && state.mode === 'paged') return;
    state.page = nextPage;

    if (state.mode === 'paged') {
        renderPagedPage();
    } else if (scroll) {
        scrollToPage(nextPage);
    }
    updateControls();
}

function scrollToPage(page, smooth = true) {
    const shell = elements.continuousDocument.querySelector(`[data-page="${page}"]`);
    shell?.scrollIntoView({
        behavior: smooth ? 'smooth' : 'auto',
        block: 'start',
        inline: 'center',
    });
}

function updateVisiblePage() {
    if (state.mode !== 'continuous') return;
    const viewportRect = elements.viewport.getBoundingClientRect();
    const targetY = viewportRect.top + Math.min(160, viewportRect.height * 0.35);
    const shell = findContinuousShellAtY(targetY);
    const page = Number(shell?.dataset.page);

    if (page && page !== state.page) {
        state.page = page;
        updateControls();
    }
}

function updateControls() {
    elements.pageInput.value = String(state.page);
    elements.pageInput.max = String(state.totalPages || 1);
    elements.pageTotal.textContent = `/ ${state.totalPages || 1}`;
    elements.prevPage.disabled = state.page <= 1;
    elements.nextPage.disabled = state.page >= state.totalPages;
    elements.zoomValue.textContent = `${Math.round(state.zoom * 100)}%`;
    elements.zoomOut.disabled = state.zoom <= ZOOM_STEPS[0];
    elements.zoomIn.disabled = state.zoom >= ZOOM_STEPS[ZOOM_STEPS.length - 1];
}

function getAnchorShell(clientX, clientY) {
    if (state.mode === 'paged') return elements.pagedShell;

    const hit = document.elementFromPoint(clientX, clientY)?.closest('.page-shell');
    if (hit) return hit;
    return findContinuousShellAtY(clientY);
}

function findContinuousShellAtY(clientY) {
    const shells = elements.continuousDocument.children;
    if (shells.length === 0) return null;

    const documentRect = elements.continuousDocument.getBoundingClientRect();
    const displayScale = state.zoom / state.committedZoom;
    const localY = (clientY - documentRect.top) / displayScale;
    let low = 0;
    let high = shells.length - 1;

    while (low <= high) {
        const middle = Math.floor((low + high) / 2);
        const shell = shells[middle];
        const top = shell.offsetTop;
        const bottom = top + shell.offsetHeight;
        if (localY < top) {
            high = middle - 1;
        } else if (localY > bottom) {
            low = middle + 1;
        } else {
            return shell;
        }
    }

    return shells[Math.max(0, Math.min(shells.length - 1, low))];
}

function captureZoomAnchor(anchor) {
    const viewportRect = elements.viewport.getBoundingClientRect();
    const clientX = anchor?.x ?? viewportRect.left + viewportRect.width / 2;
    const clientY = anchor?.y ?? viewportRect.top + viewportRect.height / 2;
    const shell = getAnchorShell(clientX, clientY);
    const shellRect = shell?.getBoundingClientRect();

    if (state.mode === 'paged' && shellRect) {
        const visibleLeft = Math.max(shellRect.left, viewportRect.left);
        const visibleRight = Math.min(shellRect.right, viewportRect.right);
        const visibleTop = Math.max(shellRect.top, viewportRect.top);
        const visibleBottom = Math.min(shellRect.bottom, viewportRect.bottom);
        const isVisible = visibleLeft < visibleRight && visibleTop < visibleBottom;

        if (!isVisible) {
            return {
                clientX,
                clientY,
                shell,
                pageX: 0.5,
                pageY: 0.5,
                recenter: true,
            };
        }

        const anchoredX = anchor
            ? Math.max(visibleLeft, Math.min(visibleRight, clientX))
            : (visibleLeft + visibleRight) / 2;
        const anchoredY = anchor
            ? Math.max(visibleTop, Math.min(visibleBottom, clientY))
            : (visibleTop + visibleBottom) / 2;

        return {
            clientX: anchoredX,
            clientY: anchoredY,
            shell,
            pageX: Math.max(0, Math.min(1, (anchoredX - shellRect.left) / shellRect.width)),
            pageY: Math.max(0, Math.min(1, (anchoredY - shellRect.top) / shellRect.height)),
            recenter: false,
        };
    }

    return {
        clientX,
        clientY,
        shell,
        pageX: shellRect ? (clientX - shellRect.left) / shellRect.width : 0.5,
        pageY: shellRect ? (clientY - shellRect.top) / shellRect.height : 0.5,
    };
}

function restoreZoomAnchor(anchor) {
    if (!anchor.shell) return;
    const shellRect = anchor.shell.getBoundingClientRect();
    const viewportRect = elements.viewport.getBoundingClientRect();
    const deltaX = anchor.recenter
        ? shellRect.left + shellRect.width / 2 - (viewportRect.left + viewportRect.width / 2)
        : shellRect.left + anchor.pageX * shellRect.width - anchor.clientX;
    const deltaY = anchor.recenter
        ? shellRect.top + shellRect.height / 2 - (viewportRect.top + viewportRect.height / 2)
        : shellRect.top + anchor.pageY * shellRect.height - anchor.clientY;

    if (state.mode === 'continuous' || state.canPanX) {
        elements.viewport.scrollLeft += deltaX;
    }
    if (state.mode === 'continuous' || state.canPanY) {
        elements.viewport.scrollTop += deltaY;
    }
    keepPagedPageVisible();
}

function commitRenderedZoom(zoom) {
    if (Math.abs(state.committedZoom - zoom) < 0.001) return;

    const anchor = captureZoomAnchor(state.zoomAnchor);
    state.committedZoom = zoom;
    updatePageLayouts();
    restoreZoomAnchor(anchor);
    state.zoomAnchor = null;
}

function keepPagedPageVisible() {
    if (state.mode !== 'paged') return;

    const viewportRect = elements.viewport.getBoundingClientRect();
    const shellRect = elements.pagedShell.getBoundingClientRect();
    const minimumVisible = 32;

    if (state.canPanX && shellRect.right < viewportRect.left + minimumVisible) {
        elements.viewport.scrollLeft -= viewportRect.left + minimumVisible - shellRect.right;
    } else if (state.canPanX && shellRect.left > viewportRect.right - minimumVisible) {
        elements.viewport.scrollLeft += shellRect.left - (viewportRect.right - minimumVisible);
    }

    if (state.canPanY && shellRect.bottom < viewportRect.top + minimumVisible) {
        elements.viewport.scrollTop -= viewportRect.top + minimumVisible - shellRect.bottom;
    } else if (state.canPanY && shellRect.top > viewportRect.bottom - minimumVisible) {
        elements.viewport.scrollTop += shellRect.top - (viewportRect.bottom - minimumVisible);
    }
}

function setZoom(nextZoom, anchor) {
    const clamped = Math.max(0.1, Math.min(4, nextZoom));
    if (Math.abs(clamped - state.targetZoom) < 0.001) return;

    state.targetZoom = clamped;
    state.zoomAnchor = anchor;
    if (state.zoomFrame === null) {
        state.zoomFrame = requestAnimationFrame(flushZoom);
    }
}

function flushZoom() {
    state.zoomFrame = null;
    if (Math.abs(state.targetZoom - state.zoom) < 0.001) return;

    const zoomAnchor = captureZoomAnchor(state.zoomAnchor);
    state.zoom = state.targetZoom;
    if (state.mode === 'continuous') {
        updateContinuousTransform();
    } else {
        updatePagedTransform();
    }
    restoreZoomAnchor(zoomAnchor);
    updateControls();
    enforceCacheLimit();

    if (state.pending.size > 0) {
        state.worker.postMessage({ type: 'cancel' });
        state.pending.clear();
    }
    clearTimeout(state.renderTimer);
    state.renderTimer = setTimeout(() => {
        if (state.scrolling) return;
        if (state.mode === 'paged') {
            requestPage(state.page);
        } else {
            requestVisiblePages();
        }
    }, 240);
}

function zoomStep(direction) {
    const next = direction > 0
        ? ZOOM_STEPS.find(value => value > state.targetZoom + 0.001)
        : ZOOM_STEPS.findLast(value => value < state.targetZoom - 0.001);
    setZoom(next ?? (direction > 0 ? ZOOM_STEPS.at(-1) : ZOOM_STEPS[0]));
}

function preventDefaults(event) {
    event.preventDefault();
    event.stopPropagation();
}

function getTouchGesture(touches) {
    const first = touches[0];
    const second = touches[1];
    return {
        distance: Math.hypot(second.clientX - first.clientX, second.clientY - first.clientY),
        x: (first.clientX + second.clientX) / 2,
        y: (first.clientY + second.clientY) / 2,
    };
}

function configureFileInteractions() {
    elements.dropZone.addEventListener('click', () => elements.fileInput.click());
    elements.fileInput.addEventListener('change', event => {
        if (event.target.files.length > 0) handleFile(event.target.files[0]);
    });

    document.addEventListener('dragenter', event => {
        preventDefaults(event);
        if (state.loaded) {
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
        if (event.dataTransfer.files.length > 0) handleFile(event.dataTransfer.files[0]);
    });
}

function configureViewerInteractions() {
    elements.settingsButton.addEventListener('click', () => {
        setSettingsOpen(elements.settingsPopover.hidden);
    });
    document.addEventListener('pointerdown', event => {
        if (elements.settingsPopover.hidden) return;
        if (elements.settingsPopover.contains(event.target)) return;
        if (event.target === elements.settingsButton) return;
        setSettingsOpen(false);
    });
    elements.themeOptions.forEach(option => {
        option.addEventListener('click', () => {
            applyTheme(option.dataset.themeChoice);
        });
    });
    elements.pagedMode.addEventListener('click', () => switchMode('paged'));
    elements.continuousMode.addEventListener('click', () => switchMode('continuous'));
    elements.prevPage.addEventListener('click', () => setPage(state.page - 1));
    elements.nextPage.addEventListener('click', () => setPage(state.page + 1));
    elements.zoomOut.addEventListener('click', () => zoomStep(-1));
    elements.zoomIn.addEventListener('click', () => zoomStep(1));
    elements.zoomValue.addEventListener('click', () => setZoom(1));
    elements.homeButton.addEventListener('click', () => {
        if (state.loaded) showFileSelector();
    });

    elements.pageInput.addEventListener('change', () => {
        const page = Number(elements.pageInput.value);
        if (Number.isFinite(page)) setPage(page);
        updateControls();
    });
    elements.pageInput.addEventListener('keydown', event => {
        if (event.key === 'Enter') {
            event.currentTarget.blur();
        }
    });

    elements.viewport.addEventListener('wheel', event => {
        if (!event.ctrlKey && !event.metaKey) return;
        event.preventDefault();
        const lineMultiplier = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : 1;
        const scale = event.ctrlKey ? 0.01 : 0.002 * lineMultiplier;
        const factor = Math.exp(-event.deltaY * scale);
        setZoom(state.targetZoom * factor, { x: event.clientX, y: event.clientY });
    }, { passive: false });

    elements.viewport.addEventListener('pointerdown', event => {
        if (event.pointerType === 'touch') return;
        if (event.button !== 0 || !event.target.closest('.page-shell')) return;
        updateViewportOverflow();
        if (!state.canPanX && !state.canPanY) return;
        state.drag = {
            pointerId: event.pointerId,
            x: event.clientX,
            y: event.clientY,
            scrollLeft: elements.viewport.scrollLeft,
            scrollTop: elements.viewport.scrollTop,
        };
        elements.viewport.setPointerCapture(event.pointerId);
        elements.viewport.classList.add('dragging');
    });
    elements.viewport.addEventListener('pointermove', event => {
        if (!state.drag || state.drag.pointerId !== event.pointerId) return;
        if (state.canPanX) {
            elements.viewport.scrollLeft = state.drag.scrollLeft - (event.clientX - state.drag.x);
        }
        if (state.canPanY) {
            elements.viewport.scrollTop = state.drag.scrollTop - (event.clientY - state.drag.y);
        }
    });
    const stopDragging = event => {
        if (!state.drag || state.drag.pointerId !== event.pointerId) return;
        state.drag = null;
        elements.viewport.classList.remove('dragging');
    };
    elements.viewport.addEventListener('pointerup', stopDragging);
    elements.viewport.addEventListener('pointercancel', stopDragging);
    elements.viewport.addEventListener('touchstart', event => {
        if (event.touches.length === 2) {
            event.preventDefault();
            const gesture = getTouchGesture(event.touches);
            state.pinch = {
                distance: gesture.distance,
                zoom: state.targetZoom,
            };
            state.swipe = null;
            return;
        }

        if (event.touches.length !== 1 || state.mode !== 'paged') return;
        updateViewportOverflow();
        if (state.canPanX || state.canPanY) return;
        const touch = event.touches[0];
        state.swipe = {
            x: touch.clientX,
            y: touch.clientY,
            time: performance.now(),
        };
    }, { passive: false });
    elements.viewport.addEventListener('touchmove', event => {
        if (!state.pinch || event.touches.length !== 2) return;
        event.preventDefault();
        const gesture = getTouchGesture(event.touches);
        const factor = gesture.distance / Math.max(1, state.pinch.distance);
        setZoom(state.pinch.zoom * factor, { x: gesture.x, y: gesture.y });
    }, { passive: false });
    elements.viewport.addEventListener('touchend', event => {
        if (state.pinch) {
            if (event.touches.length < 2) state.pinch = null;
            state.swipe = null;
            return;
        }

        if (!state.swipe || event.changedTouches.length === 0) return;
        const touch = event.changedTouches[0];
        const deltaX = touch.clientX - state.swipe.x;
        const deltaY = touch.clientY - state.swipe.y;
        const elapsed = performance.now() - state.swipe.time;
        state.swipe = null;

        if (elapsed > 600 || Math.abs(deltaX) < 50 || Math.abs(deltaX) < Math.abs(deltaY) * 1.5) {
            return;
        }
        setPage(state.page + (deltaX < 0 ? 1 : -1));
    }, { passive: true });
    elements.viewport.addEventListener('touchcancel', () => {
        state.pinch = null;
        state.swipe = null;
    }, { passive: true });
    elements.viewport.addEventListener('scroll', () => {
        if (state.mode !== 'continuous') return;

        if (!state.scrolling) {
            state.scrolling = true;
            if (state.pending.size > 0) {
                state.worker.postMessage({ type: 'cancel' });
                state.pending.clear();
            }
        }
        clearTimeout(state.scrollEndTimer);
        state.scrollEndTimer = setTimeout(() => {
            state.scrolling = false;
            requestVisiblePages();
            enforceCacheLimit();
        }, 160);

        if (state.scrollFrame === null) {
            state.scrollFrame = requestAnimationFrame(() => {
                state.scrollFrame = null;
                updateVisiblePage();
            });
        }
    }, { passive: true });

    document.addEventListener('keydown', event => {
        if (event.key === 'Escape' && !elements.settingsPopover.hidden) {
            setSettingsOpen(false);
            elements.settingsButton.focus();
            return;
        }
        if (!state.loaded || event.target.matches('input')) return;

        if (state.mode === 'paged' && event.key === 'ArrowLeft') {
            event.preventDefault();
            setPage(state.page - 1);
        } else if (state.mode === 'paged' && event.key === 'ArrowRight') {
            event.preventDefault();
            setPage(state.page + 1);
        } else if ((event.ctrlKey || event.metaKey) && (event.key === '+' || event.key === '=')) {
            event.preventDefault();
            zoomStep(1);
        } else if ((event.ctrlKey || event.metaKey) && event.key === '-') {
            event.preventDefault();
            zoomStep(-1);
        } else if ((event.ctrlKey || event.metaKey) && event.key === '0') {
            event.preventDefault();
            setZoom(1);
        }
    });

    window.addEventListener('resize', () => {
        const viewportWidth = window.innerWidth;
        const pixelRatio = getRenderPixelRatio();
        const widthChanged = Math.abs(viewportWidth - state.layoutViewportWidth) > 1;
        const pixelRatioChanged = Math.abs(pixelRatio - state.layoutPixelRatio) > 0.01;
        state.layoutViewportWidth = viewportWidth;
        state.layoutPixelRatio = pixelRatio;
        if (!state.loaded) return;

        if (state.mode === 'continuous' && !widthChanged && !pixelRatioChanged) {
            updateContinuousTransform();
            return;
        }

        clearTimeout(state.resizeTimer);
        updatePageLayouts();
        state.resizeTimer = setTimeout(() => {
            state.worker.postMessage({ type: 'cancel' });
            state.pending.clear();
            state.renderFailures.clear();
            clearRenderedPages();
            if (state.mode === 'paged') {
                renderPagedPage();
            } else {
                requestVisiblePages();
            }
        }, 220);
    });
}

systemThemeQuery.addEventListener('change', () => {
    if (state.themePreference === 'system') applyTheme('system');
});
applyTheme(state.themePreference);
updateModeControls();
state.worker = createWorker();
configureFileInteractions();
configureViewerInteractions();

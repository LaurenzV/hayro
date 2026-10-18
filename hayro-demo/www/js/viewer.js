// Shows a document in the viewport: places the pages, keeps the ones near the viewport in the
// DOM, paints their renders and tells the scheduler which pages are needed.

import {
    CACHE_BYTES,
    MAX_CACHED_PAGES,
    MAX_PAGE_PIXELS,
    MAX_ZOOM,
    MIN_ZOOM,
    MOUNT_MARGIN,
    PREFETCH_PAGES,
    SETTLE_DELAY,
    ZOOM_STEPS,
} from './config.js';
import { computeLayout, pageAt, pageRect, pagesBetween } from './layout.js';
import { sameScale } from './scheduler.js';

const COMPACT_WIDTH = 680;
// Pixmaps can't be wider or taller than this.
const MAX_RENDER_SIDE = 16384;

function clamp(value, min, max) {
    return Math.max(min, Math.min(max, value));
}

function createShell(page, mode) {
    const shell = document.createElement('div');
    shell.className = mode === 'continuous' ? 'page-shell continuous-shell' : 'page-shell';
    shell.dataset.page = String(page);

    const canvas = document.createElement('canvas');
    canvas.setAttribute('aria-label', `Rendered PDF page ${page}`);

    const placeholder = document.createElement('div');
    placeholder.className = 'page-placeholder';
    placeholder.setAttribute('role', 'status');
    placeholder.setAttribute('aria-label', 'Rendering page');
    const spinner = document.createElement('span');
    spinner.className = 'spinner page-spinner';
    spinner.setAttribute('aria-hidden', 'true');
    placeholder.append(spinner);

    shell.append(canvas, placeholder);

    if (mode === 'continuous') {
        const badge = document.createElement('span');
        badge.className = 'page-number';
        badge.textContent = String(page);
        shell.append(badge);
    }

    return shell;
}

export class Viewer {
    #viewport;
    #stage;
    #cache;
    #scheduler;
    #onChange;
    #document = null;
    #layout = null;
    #mounted = new Map();
    #settled = true;
    #settleTimer = null;
    #scrollFrame = null;
    #zoomFrame = null;
    #targetZoom = 1;
    #zoomAnchor = null;

    mode = 'continuous';
    page = 1;
    zoom = 1;
    canPanX = false;
    canPanY = false;

    // `onChange` is called whenever the page, the zoom or the mode changes.
    constructor({ viewport, stage }, cache, scheduler, onChange) {
        this.#viewport = viewport;
        this.#stage = stage;
        this.#cache = cache;
        this.#scheduler = scheduler;
        this.#onChange = onChange;
    }

    get loaded() {
        return this.#document !== null;
    }

    get pageCount() {
        return this.#document?.pageCount ?? 0;
    }

    get targetZoom() {
        return this.#targetZoom;
    }

    open(document) {
        this.#document = document;
        this.page = 1;
        this.zoom = 1;
        this.#targetZoom = 1;
        this.#unmountAll();
        this.#relayout();
        this.#viewport.scrollTo(0, 0);
        this.#scrollToPage(1, false);
        this.#update();
    }

    close() {
        this.#document = null;
        this.#unmountAll();
        this.#scheduler.setWanted([], new Set(), 1);
    }

    setMode(mode) {
        this.mode = mode === 'paged' ? 'paged' : 'continuous';
        if (!this.loaded) {
            this.#onChange();
            return;
        }

        this.#unmountAll();
        this.#relayout();
        this.#scrollToPage(this.page, false);
        this.#update();
    }

    setPage(page, smooth = true) {
        if (!this.loaded) return;

        const target = clamp(Math.round(page), 1, this.pageCount);
        if (this.mode === 'paged') {
            if (target === this.page) return;
            this.page = target;
            this.#relayout();
            this.#scrollToPage(target, false);
            this.#update();
            return;
        }

        this.page = target;
        this.#scrollToPage(target, smooth);
        this.#onChange();
    }

    // Zooms to `zoom`, keeping the content under `anchor` (a client point) in place, or the
    // content in the middle of the viewport if there is none.
    setZoom(zoom, anchor = null) {
        const clamped = clamp(zoom, MIN_ZOOM, MAX_ZOOM);
        if (Math.abs(clamped - this.#targetZoom) < 0.0005) return;

        this.#targetZoom = clamped;
        this.#zoomAnchor = anchor;
        this.#zoomFrame ??= requestAnimationFrame(() => {
            this.#zoomFrame = null;
            this.#applyZoom();
        });
    }

    zoomStep(direction) {
        const next = direction > 0
            ? ZOOM_STEPS.find(step => step > this.#targetZoom + 0.001)
            : ZOOM_STEPS.findLast(step => step < this.#targetZoom - 0.001);
        this.setZoom(next ?? (direction > 0 ? MAX_ZOOM : MIN_ZOOM));
    }

    handleScroll() {
        this.#scrollFrame ??= requestAnimationFrame(() => {
            this.#scrollFrame = null;
            this.#update();
        });
    }

    handleResize() {
        if (!this.loaded) return;

        const anchor = this.#captureAnchor(null);
        this.#relayout();
        this.#restoreAnchor(anchor);
        this.#unsettle();
        this.#update();
    }

    // Shows a render that has just arrived.
    handleRendered(page) {
        this.#paint(page);
    }

    #applyZoom() {
        if (!this.loaded || Math.abs(this.#targetZoom - this.zoom) < 0.0005) return;

        const anchor = this.#captureAnchor(this.#zoomAnchor);
        this.zoom = this.#targetZoom;
        this.#zoomAnchor = null;
        this.#relayout();
        this.#restoreAnchor(anchor);
        this.#unsettle();
        this.#update();
    }

    // While zooming or resizing, existing renders are shown scaled, and only pages without any
    // render are rendered. Once the size stops changing, everything is rendered at the new size.
    #unsettle() {
        this.#settled = false;
        clearTimeout(this.#settleTimer);
        this.#settleTimer = setTimeout(() => {
            this.#settled = true;
            this.#update();
        }, SETTLE_DELAY);
    }

    #layoutOptions(page = this.page) {
        return {
            mode: this.mode,
            pageSizes: this.#document.pageSizes,
            page,
            zoom: this.zoom,
            viewport: { width: this.#viewport.clientWidth, height: this.#viewport.clientHeight },
            compact: window.innerWidth <= COMPACT_WIDTH,
        };
    }

    #relayout() {
        // Showing or hiding scroll bars changes the size of the viewport, which the layout
        // depends on, so lay out again if it changed.
        for (let pass = 0; pass < 2; pass += 1) {
            const { clientWidth, clientHeight } = this.#viewport;
            this.#applyLayout(computeLayout(this.#layoutOptions()));
            if (clientWidth === this.#viewport.clientWidth && clientHeight === this.#viewport.clientHeight) {
                break;
            }
        }
    }

    #applyLayout(layout) {
        const paged = layout.mode === 'paged';
        this.#layout = layout;

        this.#stage.style.width = `${layout.contentWidth}px`;
        this.#stage.style.height = `${layout.contentHeight}px`;
        this.#stage.classList.toggle('paged', paged);
        this.#stage.classList.toggle('continuous', !paged);

        this.canPanX = layout.contentWidth > this.#viewport.clientWidth + 0.5;
        this.canPanY = layout.contentHeight > this.#viewport.clientHeight + 0.5;
        // A paged page is fitted into the viewport, so scroll bars would only come from rounding.
        this.#viewport.style.overflowX = paged && !this.canPanX ? 'hidden' : 'auto';
        this.#viewport.style.overflowY = paged && !this.canPanY ? 'hidden' : 'auto';
        this.#viewport.classList.toggle('can-pan', this.canPanX || this.canPanY);

        for (const [page, shell] of this.#mounted) {
            const rect = pageRect(layout, page);
            if (rect) {
                this.#position(shell, rect);
            } else {
                this.#unmount(page);
            }
        }
    }

    #scrollToPage(page, smooth) {
        const layout = this.#layout;
        const left = (layout.contentWidth - this.#viewport.clientWidth) / 2;

        if (layout.mode === 'paged') {
            this.#viewport.scrollTo({ left, top: 0 });
            return;
        }

        this.#viewport.scrollTo({
            left,
            top: pageRect(layout, page).y - layout.top,
            behavior: smooth ? 'smooth' : 'auto',
        });
    }

    // Brings the DOM and the wanted renders in line with the current layout and scroll position.
    #update() {
        if (!this.loaded) return;

        const top = this.#viewport.scrollTop;
        const bottom = top + this.#viewport.clientHeight;
        let visible = [this.page];
        let nearby = [this.page];

        if (this.mode === 'continuous') {
            visible = pagesBetween(this.#layout, top, bottom);
            nearby = pagesBetween(this.#layout, top - MOUNT_MARGIN, bottom + MOUNT_MARGIN);
            this.page = pageAt(this.#layout, top + Math.min(160, this.#viewport.clientHeight * 0.35));
            if (visible.length === 0) visible = [this.page];
        }

        this.#mount(nearby);
        this.#requestRenders(visible, nearby, (top + bottom) / 2);
        nearby.forEach(page => this.#paint(page));
        this.#onChange();
    }

    #mount(pages) {
        const keep = new Set(pages);
        for (const page of this.#mounted.keys()) {
            if (!keep.has(page)) this.#unmount(page);
        }

        for (const page of pages) {
            if (this.#mounted.has(page)) continue;
            const shell = createShell(page, this.mode);
            this.#position(shell, pageRect(this.#layout, page));
            this.#stage.append(shell);
            this.#mounted.set(page, shell);
        }
    }

    #unmount(page) {
        this.#mounted.get(page).remove();
        this.#mounted.delete(page);
        this.#cache.setShown(page, false);
    }

    #unmountAll() {
        [...this.#mounted.keys()].forEach(page => this.#unmount(page));
    }

    #position(shell, rect) {
        shell.style.left = `${rect.x}px`;
        shell.style.top = `${rect.y}px`;
        shell.style.width = `${rect.width}px`;
        shell.style.height = `${rect.height}px`;
    }

    // The resolution, in pixels per PDF point, at which `page` should be rendered.
    #renderScale(page) {
        const size = this.#document.pageSizes[page - 1];
        const rect = pageRect(this.#layout, page)
            ?? computeLayout(this.#layoutOptions(page)).pages[0];
        let scale = rect.width * Math.max(window.devicePixelRatio || 1, 1) / size.width;

        const pixels = size.width * size.height * scale * scale;
        if (pixels > MAX_PAGE_PIXELS) scale *= Math.sqrt(MAX_PAGE_PIXELS / pixels);
        const side = Math.max(size.width, size.height) * scale;
        if (side > MAX_RENDER_SIDE) scale *= MAX_RENDER_SIDE / side;

        return scale;
    }

    // Wants the visible pages first, closest to the middle of the viewport first, then the
    // pages around them, as far as the cache can hold them.
    #requestRenders(visible, nearby, center) {
        const wanted = [];
        const seen = new Set();
        const anyScale = !this.#settled;
        let bytes = 0;

        const want = (page, required) => {
            if (page < 1 || page > this.pageCount || seen.has(page)) return;

            const size = this.#document.pageSizes[page - 1];
            const scale = this.#renderScale(page);
            const pageBytes = size.width * size.height * scale * scale * 4;
            if (!required && (bytes + pageBytes > CACHE_BYTES || wanted.length >= MAX_CACHED_PAGES)) {
                return;
            }

            seen.add(page);
            bytes += pageBytes;
            wanted.push({ page, scale, anyScale });
        };

        const distance = page => {
            const rect = pageRect(this.#layout, page);
            return Math.abs(rect.y + rect.height / 2 - center);
        };
        [...visible].sort((left, right) => distance(left) - distance(right)).forEach(page => want(page, true));
        nearby.forEach(page => want(page, false));

        const first = Math.min(...visible);
        const last = Math.max(...visible);
        for (let offset = 1; offset <= PREFETCH_PAGES; offset += 1) {
            want(last + offset, false);
            want(first - offset, false);
        }

        this.#scheduler.setWanted(wanted, new Set(visible), this.page);
    }

    #paint(page) {
        const shell = this.#mounted.get(page);
        if (!shell) return;

        const entry = this.#cache.get(page);
        if (!entry) {
            if (this.#scheduler.hasFailed(page)) this.#showError(shell);
            return;
        }

        const canvas = shell.querySelector('canvas');
        if (shell.dataset.renderId !== String(entry.id)) {
            canvas.width = entry.width;
            canvas.height = entry.height;
            const context = canvas.getContext('2d', { alpha: false });
            if (entry.bitmap) {
                context.drawImage(entry.bitmap, 0, 0);
            } else {
                const pixels = new Uint8ClampedArray(entry.pixels);
                context.putImageData(new ImageData(pixels, entry.width, entry.height), 0, 0);
            }
            shell.dataset.renderId = String(entry.id);
            this.#cache.setShown(page, true);
        }

        canvas.classList.toggle('soft', !sameScale(entry.scale, this.#renderScale(page)));
        shell.classList.add('rendered');
        shell.classList.remove('error');
    }

    #showError(shell) {
        if (shell.classList.contains('rendered')) return;

        shell.classList.add('error');
        const placeholder = shell.querySelector('.page-placeholder');
        placeholder.removeAttribute('role');
        placeholder.removeAttribute('aria-label');
        placeholder.textContent = 'Page could not be rendered';
    }

    // Remembers which point of which page is at `point` (or the middle of the viewport).
    #captureAnchor(point) {
        const bounds = this.#viewport.getBoundingClientRect();
        const offsetX = point ? point.x - bounds.left : this.#viewport.clientWidth / 2;
        const offsetY = point ? point.y - bounds.top : this.#viewport.clientHeight / 2;
        const x = this.#viewport.scrollLeft + offsetX;
        const y = this.#viewport.scrollTop + offsetY;
        const page = this.mode === 'paged' ? this.page : pageAt(this.#layout, y);
        const rect = pageRect(this.#layout, page);

        return {
            page,
            offsetX,
            offsetY,
            pageX: (x - rect.x) / rect.width,
            pageY: (y - rect.y) / rect.height,
        };
    }

    // Scrolls so that the point remembered by `#captureAnchor` is back under the same spot.
    #restoreAnchor(anchor) {
        const rect = pageRect(this.#layout, anchor.page);
        if (!rect) return;

        this.#viewport.scrollLeft = rect.x + anchor.pageX * rect.width - anchor.offsetX;
        this.#viewport.scrollTop = rect.y + anchor.pageY * rect.height - anchor.offsetY;
    }
}

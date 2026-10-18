export const CONSTRAINED_DEVICE = matchMedia('(pointer: coarse)').matches
    || (navigator.deviceMemory && navigator.deviceMemory <= 4)
    || navigator.hardwareConcurrency <= 4;

// The number of renderer workers. Each one holds its own copy of the document.
export const WORKER_COUNT = CONSTRAINED_DEVICE ? 1 : 2;

// The memory available for rendered pages.
export const CACHE_BYTES = (CONSTRAINED_DEVICE ? 96 : 192) * 1024 * 1024;
export const MAX_CACHED_PAGES = CONSTRAINED_DEVICE ? 6 : 12;

// The largest render of a single page. Larger pages are rendered at a lower resolution and
// scaled up.
export const MAX_PAGE_PIXELS = (CONSTRAINED_DEVICE ? 12 : 32) * 1024 * 1024;

// How many pages before and after the visible ones are rendered in advance.
export const PREFETCH_PAGES = CONSTRAINED_DEVICE ? 1 : 2;

// How far outside the viewport pages are kept in the DOM, in CSS pixels.
export const MOUNT_MARGIN = 600;

export const ZOOM_STEPS = [0.1, 0.25, 0.5, 0.67, 0.8, 1, 1.25, 1.5, 2, 2.5, 3, 4];
export const MIN_ZOOM = ZOOM_STEPS[0];
export const MAX_ZOOM = ZOOM_STEPS.at(-1);

// How long zooming or resizing has to pause before pages are rendered at the new size.
export const SETTLE_DELAY = 220;

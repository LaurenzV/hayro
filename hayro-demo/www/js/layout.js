// Computes where pages are placed, in CSS pixels, from their sizes in PDF points. Nothing here
// touches the DOM.

const MAX_FIT_WIDTH = 1240;
const MIN_FIT_SIZE = 200;

function spacing(compact) {
    return compact
        ? { sideGutter: 24, navigationGutter: 72, pageGap: 16, top: 18, bottom: 60 }
        : { sideGutter: 48, navigationGutter: 88, pageGap: 24, top: 28, bottom: 72 };
}

// The area a page is fitted into at 100% zoom.
function fitArea(viewport, space) {
    return {
        width: Math.max(MIN_FIT_SIZE, Math.min(viewport.width - space.sideGutter, MAX_FIT_WIDTH)),
        height: Math.max(MIN_FIT_SIZE, viewport.height - space.navigationGutter),
    };
}

// Lays out the current page alone, fitted into the viewport.
function pagedLayout({ pageSizes, page, zoom, viewport, compact }) {
    const space = spacing(compact);
    const fit = fitArea(viewport, space);
    const size = pageSizes[page - 1];
    const scale = Math.min(fit.width / size.width, fit.height / size.height) * zoom;
    const width = size.width * scale;
    const height = size.height * scale;
    const contentWidth = Math.max(viewport.width, width + space.sideGutter);
    const contentHeight = Math.max(viewport.height, height + space.sideGutter);

    return {
        mode: 'paged',
        contentWidth,
        contentHeight,
        pages: [{
            page,
            x: (contentWidth - width) / 2,
            y: (contentHeight - height) / 2,
            width,
            height,
        }],
    };
}

// Lays out all pages below each other, with the width of the viewport.
function continuousLayout({ pageSizes, zoom, viewport, compact }) {
    const space = spacing(compact);
    const width = fitArea(viewport, space).width * zoom;
    const gap = space.pageGap * zoom;
    const contentWidth = Math.max(viewport.width, width + space.sideGutter);
    const x = (contentWidth - width) / 2;
    const pages = new Array(pageSizes.length);
    let y = space.top;

    for (let index = 0; index < pageSizes.length; index += 1) {
        const size = pageSizes[index];
        const height = width * size.height / size.width;
        pages[index] = { page: index + 1, x, y, width, height };
        y += height + gap;
    }

    return {
        mode: 'continuous',
        contentWidth,
        contentHeight: Math.max(viewport.height, y - gap + space.bottom),
        top: space.top,
        pages,
    };
}

export function computeLayout(options) {
    return options.mode === 'paged' ? pagedLayout(options) : continuousLayout(options);
}

// The rectangle of `page`, if it is part of the layout.
export function pageRect(layout, page) {
    if (layout.mode === 'paged') {
        return layout.pages[0].page === page ? layout.pages[0] : null;
    }

    return layout.pages[page - 1] ?? null;
}

// The index of the first page whose bottom edge is below `y`.
function firstPageBelow(layout, y) {
    let low = 0;
    let high = layout.pages.length;

    while (low < high) {
        const middle = (low + high) >> 1;
        const rect = layout.pages[middle];
        if (rect.y + rect.height < y) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }

    return low;
}

// The pages that intersect the vertical range from `top` to `bottom`.
export function pagesBetween(layout, top, bottom) {
    const pages = [];

    for (let index = firstPageBelow(layout, top); index < layout.pages.length; index += 1) {
        const rect = layout.pages[index];
        if (rect.y > bottom) break;
        if (rect.y + rect.height >= top) pages.push(rect.page);
    }

    return pages;
}

// The page at the vertical position `y`, or the closest one.
export function pageAt(layout, y) {
    const index = Math.min(firstPageBelow(layout, y), layout.pages.length - 1);
    return layout.pages[index].page;
}

let viewer = null;
let PdfViewer = null;
let generation = 0;
let queue = [];
let busy = false;
let initialization = null;
let rendererVariant = new URL(self.location.href).searchParams.get('variant') === 'simd'
    ? 'simd'
    : 'nosimd';

async function loadRenderer(variant) {
    const module = variant === 'simd'
        ? await import('./hayro_demo_simd.js')
        : await import('./hayro_demo_nosimd.js');
    await module.default();
    PdfViewer = module.PdfViewer;
}

async function ensureInitialized() {
    if (!initialization) {
        initialization = loadRenderer(rendererVariant).catch(error => {
            if (rendererVariant !== 'simd') throw error;
            rendererVariant = 'nosimd';
            return loadRenderer(rendererVariant);
        });
    }
    await initialization;
}

function describeError(error) {
    return error instanceof Error ? error.message : String(error);
}

async function processNext() {
    if (busy || queue.length === 0 || !viewer) return;

    busy = true;
    const job = queue.shift();

    try {
        const result = viewer.render_page(
            job.page,
            job.viewportWidth,
            job.viewportHeight,
            job.devicePixelRatio,
            job.zoom,
            job.fitWidth,
        );
        const width = result[0];
        const height = result[1];
        const pixels = result[2];
        const buffer = pixels.byteOffset === 0 && pixels.byteLength === pixels.buffer.byteLength
            ? pixels.buffer
            : pixels.buffer.slice(pixels.byteOffset, pixels.byteOffset + pixels.byteLength);

        if ('createImageBitmap' in self) {
            const imageData = new ImageData(new Uint8ClampedArray(buffer), width, height);
            const bitmap = await createImageBitmap(imageData);
            self.postMessage({
                type: 'rendered',
                generation: job.generation,
                requestId: job.requestId,
                page: job.page,
                zoom: job.zoom,
                width,
                height,
                bitmap,
            }, [bitmap]);
        } else {
            self.postMessage({
                type: 'rendered',
                generation: job.generation,
                requestId: job.requestId,
                page: job.page,
                zoom: job.zoom,
                width,
                height,
                buffer,
            }, [buffer]);
        }
    } catch (error) {
        self.postMessage({
            type: 'render-error',
            generation: job.generation,
            requestId: job.requestId,
            page: job.page,
            message: describeError(error),
        });
    } finally {
        busy = false;
        setTimeout(processNext, 0);
    }
}

self.addEventListener('message', async ({ data }) => {
    try {
        if (data.type === 'load') {
            generation = data.generation;
            queue = [];
            await ensureInitialized();
            viewer = new PdfViewer();
            viewer.load_pdf(new Uint8Array(data.buffer));

            self.postMessage({
                type: 'loaded',
                generation,
                variant: rendererVariant,
                totalPages: viewer.get_total_pages(),
                dimensions: Array.from(viewer.get_page_dimensions()),
            });
            return;
        }

        if (data.type === 'cancel') {
            queue = [];
            return;
        }

        if (data.type === 'render' && data.generation === generation) {
            queue = queue.filter(job => job.page !== data.page);
            if (data.priority) {
                queue.unshift(data);
            } else {
                queue.push(data);
            }
            processNext();
        }
    } catch (error) {
        self.postMessage({
            type: 'load-error',
            generation: data.generation,
            message: describeError(error),
        });
    }
});

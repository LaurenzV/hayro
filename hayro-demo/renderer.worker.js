// Renders pages of one document. The worker never queues work: the main thread sends a new
// job only after the previous one was answered, so it can always pick the most useful page.

let PdfViewer = null;
let initialization = null;
let variant = new URL(self.location.href).searchParams.get('variant') === 'simd' ? 'simd' : 'nosimd';
let viewer = null;
let generation = 0;

async function loadModule(name) {
    const module = name === 'simd'
        ? await import('./hayro_demo_simd.js')
        : await import('./hayro_demo_nosimd.js');
    await module.default();
    PdfViewer = module.PdfViewer;
}

function ensureInitialized() {
    initialization ??= loadModule(variant).catch(error => {
        if (variant !== 'simd') throw error;
        variant = 'nosimd';
        return loadModule(variant);
    });
    return initialization;
}

function describeError(error) {
    return error instanceof Error ? error.message : String(error);
}

async function load({ generation: nextGeneration, buffer }) {
    generation = nextGeneration;
    viewer?.free();
    viewer = null;

    try {
        await ensureInitialized();
        if (generation !== nextGeneration) return;
        viewer = new PdfViewer(new Uint8Array(buffer));
        self.postMessage({
            type: 'loaded',
            generation,
            variant,
            pageCount: viewer.page_count(),
            pageSizes: viewer.page_sizes(),
        });
    } catch (error) {
        self.postMessage({ type: 'load-error', generation: nextGeneration, message: describeError(error) });
    }
}

async function render(job) {
    if (job.generation !== generation || !viewer) {
        self.postMessage({ type: 'render-skipped', id: job.id });
        return;
    }

    try {
        const [width, height, pixels] = viewer.render_page(job.page - 1, job.scale);
        const data = new Uint8ClampedArray(pixels.buffer, pixels.byteOffset, pixels.byteLength);
        const reply = { type: 'rendered', id: job.id, width, height };

        if (typeof createImageBitmap === 'function') {
            reply.bitmap = await createImageBitmap(new ImageData(data, width, height));
            self.postMessage(reply, [reply.bitmap]);
        } else {
            reply.pixels = data.slice().buffer;
            self.postMessage(reply, [reply.pixels]);
        }
    } catch (error) {
        self.postMessage({ type: 'render-error', id: job.id, message: describeError(error) });
    }
}

self.addEventListener('message', ({ data }) => {
    if (data.type === 'load') {
        load(data);
    } else if (data.type === 'render') {
        render(data);
    }
});

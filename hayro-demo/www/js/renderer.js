// A pool of renderer workers. Each worker holds its own copy of the document and renders one
// page at a time.

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

export class RendererPool {
    #slots = [];
    #nextJobId = 0;
    #jobs = new Map();
    #pendingLoad = null;
    generation = 0;

    // `onIdle` is called whenever a worker becomes available, `onCrash` if a worker dies.
    constructor(count, { onIdle, onCrash }) {
        this.onIdle = onIdle;
        const url = new URL('../renderer.worker.js', import.meta.url);
        url.searchParams.set('variant', supportsWasmSimd() ? 'simd' : 'nosimd');

        for (let index = 0; index < count; index += 1) {
            const slot = { worker: new Worker(url, { type: 'module' }), ready: false, busy: false, broken: false };
            slot.worker.addEventListener('message', ({ data }) => this.#handleMessage(slot, data));
            slot.worker.addEventListener('error', () => {
                slot.broken = true;
                onCrash();
            });
            this.#slots.push(slot);
        }
    }

    // Loads a document into all workers. Resolves with its page sizes as soon as the first worker
    // has parsed it, and rejects if that worker fails.
    load(buffer) {
        this.generation += 1;
        this.#pendingLoad?.reject(new Error('superseded'));
        this.#jobs.forEach(job => job.resolve(null));
        this.#jobs.clear();

        const promise = new Promise((resolve, reject) => {
            this.#pendingLoad = { resolve, reject, generation: this.generation };
        });

        const slots = this.#slots.filter(slot => !slot.broken);
        // Copy the data for the other workers before the original is transferred.
        const buffers = slots.map((_, index) => (index === 0 ? buffer : buffer.slice(0)));
        slots.forEach((slot, index) => {
            slot.ready = false;
            slot.busy = false;
            slot.worker.postMessage(
                { type: 'load', generation: this.generation, buffer: buffers[index] },
                [buffers[index]],
            );
        });

        return promise;
    }

    // A worker that has the current document and nothing to do.
    idleSlot() {
        return this.#slots.find(slot => slot.ready && !slot.busy && !slot.broken) ?? null;
    }

    // Renders `page` at `scale` on `slot`. Resolves with the render, with `{ error }`, or with
    // `null` if the document changed in the meantime.
    render(slot, page, scale) {
        const id = ++this.#nextJobId;
        slot.busy = true;

        return new Promise(resolve => {
            this.#jobs.set(id, { slot, resolve, generation: this.generation });
            slot.worker.postMessage({ type: 'render', id, generation: this.generation, page, scale });
        });
    }

    #handleMessage(slot, data) {
        if (data.type === 'loaded' || data.type === 'load-error') {
            this.#handleLoad(slot, data);
            return;
        }

        const job = this.#jobs.get(data.id);
        this.#jobs.delete(data.id);
        slot.busy = false;

        if (job && job.generation === this.generation) {
            if (data.type === 'rendered') {
                job.resolve({ width: data.width, height: data.height, bitmap: data.bitmap, pixels: data.pixels });
            } else if (data.type === 'render-error') {
                job.resolve({ error: data.message });
            } else {
                job.resolve(null);
            }
        } else {
            data.bitmap?.close();
        }

        this.onIdle();
    }

    #handleLoad(slot, data) {
        if (data.generation !== this.generation) return;

        const load = this.#pendingLoad;
        if (data.type === 'load-error') {
            slot.ready = false;
            if (load) {
                this.#pendingLoad = null;
                load.reject(new Error(data.message));
            }
            return;
        }

        slot.ready = true;
        if (load) {
            this.#pendingLoad = null;
            const sizes = [];
            for (let index = 0; index < data.pageSizes.length; index += 2) {
                sizes.push({ width: data.pageSizes[index], height: data.pageSizes[index + 1] });
            }
            load.resolve({ pageCount: data.pageCount, pageSizes: sizes, variant: data.variant });
        }
        this.onIdle();
    }
}

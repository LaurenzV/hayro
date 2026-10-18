// Decides what the renderer pool works on. The viewer describes the pages it wants, most
// important first, and whenever a worker is free it gets the first one that is neither cached
// nor already being rendered. Nothing is queued in the workers, so changing what is wanted
// takes effect with the next free worker.

// Renders within this relative difference in scale are considered equal.
const SCALE_TOLERANCE = 0.005;

export function sameScale(left, right) {
    return Math.abs(left - right) <= SCALE_TOLERANCE * right;
}

export class Scheduler {
    #wanted = [];
    #protectedPages = new Set();
    #currentPage = 1;
    #inFlight = new Map();
    #failures = new Map();

    constructor(pool, cache, { onRendered, onFailed }) {
        this.pool = pool;
        this.cache = cache;
        this.onRendered = onRendered;
        this.onFailed = onFailed;
    }

    reset() {
        this.#wanted = [];
        this.#protectedPages = new Set();
        this.#inFlight.clear();
        this.#failures.clear();
        this.cache.clear();
    }

    // `wanted` lists `{ page, scale, anyScale }`, most important first. With `anyScale`, any
    // existing render of the page is good enough. `protectedPages` are never evicted from the
    // cache, and the renders farthest from `currentPage` are evicted first.
    setWanted(wanted, protectedPages, currentPage) {
        this.#wanted = wanted;
        this.#protectedPages = protectedPages;
        this.#currentPage = currentPage;
        this.pump();
    }

    hasFailed(page) {
        return this.#failures.has(page);
    }

    // Hands work to all free workers.
    pump() {
        for (let slot = this.pool.idleSlot(); slot; slot = this.pool.idleSlot()) {
            const item = this.#wanted.find(candidate => this.#needsRender(candidate));
            if (!item) return;
            this.#dispatch(slot, item);
        }
    }

    #needsRender({ page, scale, anyScale }) {
        if (this.#inFlight.has(page)) return false;

        const failedScale = this.#failures.get(page);
        if (failedScale !== undefined && sameScale(failedScale, scale)) return false;

        const cached = this.cache.peek(page);
        return !cached || (!anyScale && !sameScale(cached.scale, scale));
    }

    async #dispatch(slot, { page, scale }) {
        const generation = this.pool.generation;
        this.#inFlight.set(page, scale);
        const result = await this.pool.render(slot, page, scale);
        if (generation !== this.pool.generation) return;
        this.#inFlight.delete(page);

        if (result?.error) {
            this.#failures.set(page, scale);
            console.error(`Page ${page} could not be rendered: ${result.error}`);
            this.onFailed(page);
        } else if (result) {
            this.#failures.delete(page);
            this.cache.put(page, { ...result, scale });
            this.cache.trim(this.#protectedPages, this.#currentPage);
            this.onRendered(page);
        }

        this.pump();
    }
}

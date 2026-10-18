// Keeps the most recent render of each page within a memory budget.

export class RenderCache {
    #entries = new Map();
    #bytes = 0;
    #clock = 0;
    #nextId = 0;

    constructor(maxBytes, maxPages) {
        this.maxBytes = maxBytes;
        this.maxPages = maxPages;
    }

    // Returns the render of `page` without marking it as used.
    peek(page) {
        return this.#entries.get(page);
    }

    get(page) {
        const entry = this.#entries.get(page);
        if (entry) entry.lastUsed = ++this.#clock;
        return entry;
    }

    // Stores a render of `page`, replacing an older one.
    put(page, render) {
        this.delete(page);
        const entry = {
            ...render,
            id: ++this.#nextId,
            bytes: render.width * render.height * 4,
            lastUsed: ++this.#clock,
        };
        this.#entries.set(page, entry);
        this.#bytes += entry.bytes;
        return entry;
    }

    delete(page) {
        const entry = this.#entries.get(page);
        if (!entry) return;
        entry.bitmap?.close();
        this.#bytes -= entry.bytes;
        this.#entries.delete(page);
    }

    clear() {
        for (const page of [...this.#entries.keys()]) this.delete(page);
    }

    // Evicts the least recently used renders until the budget is met, keeping `protectedPages`.
    trim(protectedPages) {
        const candidates = [...this.#entries.entries()]
            .filter(([page]) => !protectedPages.has(page))
            .sort((left, right) => left[1].lastUsed - right[1].lastUsed);

        for (const [page] of candidates) {
            if (this.#bytes <= this.maxBytes && this.#entries.size <= this.maxPages) break;
            this.delete(page);
        }
    }
}

// Pointer, wheel and touch gestures on the document viewport.

function touchGesture(touches) {
    const [first, second] = touches;
    return {
        distance: Math.hypot(second.clientX - first.clientX, second.clientY - first.clientY),
        x: (first.clientX + second.clientX) / 2,
        y: (first.clientY + second.clientY) / 2,
    };
}

export function attachGestures(viewport, viewer) {
    let drag = null;
    let pinch = null;
    let swipe = null;

    viewport.addEventListener('wheel', event => {
        if (!event.ctrlKey && !event.metaKey) return;
        event.preventDefault();
        // Pinching on a trackpad sends small deltas with `ctrlKey` set.
        const lineMultiplier = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : 1;
        const sensitivity = event.ctrlKey ? 0.01 : 0.002 * lineMultiplier;
        const factor = Math.exp(-event.deltaY * sensitivity);
        viewer.setZoom(viewer.targetZoom * factor, { x: event.clientX, y: event.clientY });
    }, { passive: false });

    viewport.addEventListener('pointerdown', event => {
        if (event.pointerType === 'touch' || event.button !== 0) return;
        if (!event.target.closest('.page-shell') || !(viewer.canPanX || viewer.canPanY)) return;

        drag = {
            pointerId: event.pointerId,
            x: event.clientX,
            y: event.clientY,
            scrollLeft: viewport.scrollLeft,
            scrollTop: viewport.scrollTop,
        };
        viewport.setPointerCapture(event.pointerId);
        viewport.classList.add('dragging');
    });

    viewport.addEventListener('pointermove', event => {
        if (drag?.pointerId !== event.pointerId) return;
        if (viewer.canPanX) viewport.scrollLeft = drag.scrollLeft - (event.clientX - drag.x);
        if (viewer.canPanY) viewport.scrollTop = drag.scrollTop - (event.clientY - drag.y);
    });

    const stopDragging = event => {
        if (drag?.pointerId !== event.pointerId) return;
        drag = null;
        viewport.classList.remove('dragging');
    };
    viewport.addEventListener('pointerup', stopDragging);
    viewport.addEventListener('pointercancel', stopDragging);

    viewport.addEventListener('touchstart', event => {
        if (event.touches.length === 2) {
            event.preventDefault();
            pinch = { distance: touchGesture(event.touches).distance, zoom: viewer.targetZoom };
            swipe = null;
            return;
        }

        // Swiping turns pages only in paged mode, and only when the page isn't panned instead.
        if (event.touches.length !== 1 || viewer.mode !== 'paged' || viewer.canPanX || viewer.canPanY) {
            return;
        }
        const [touch] = event.touches;
        swipe = { x: touch.clientX, y: touch.clientY, time: performance.now() };
    }, { passive: false });

    viewport.addEventListener('touchmove', event => {
        if (!pinch || event.touches.length !== 2) return;
        event.preventDefault();
        const gesture = touchGesture(event.touches);
        const factor = gesture.distance / Math.max(1, pinch.distance);
        viewer.setZoom(pinch.zoom * factor, { x: gesture.x, y: gesture.y });
    }, { passive: false });

    viewport.addEventListener('touchend', event => {
        if (pinch) {
            if (event.touches.length < 2) pinch = null;
            swipe = null;
            return;
        }

        if (!swipe || event.changedTouches.length === 0) return;
        const [touch] = event.changedTouches;
        const deltaX = touch.clientX - swipe.x;
        const deltaY = touch.clientY - swipe.y;
        const elapsed = performance.now() - swipe.time;
        swipe = null;

        if (elapsed > 600 || Math.abs(deltaX) < 50 || Math.abs(deltaX) < Math.abs(deltaY) * 1.5) {
            return;
        }
        viewer.setPage(viewer.page + (deltaX < 0 ? 1 : -1));
    }, { passive: true });

    viewport.addEventListener('touchcancel', () => {
        pinch = null;
        swipe = null;
    }, { passive: true });

    viewport.addEventListener('scroll', () => viewer.handleScroll(), { passive: true });
}

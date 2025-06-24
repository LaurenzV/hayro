import init, { PdfViewer } from './hayro_demo.js';

let pdfViewer = null;
let currentImage = null;

async function run() {
    await init();

    const dropZone = document.getElementById('drop-zone');
    const fileInput = document.getElementById('file-input');
    const fileSelector = document.getElementById('file-selector');
    const viewer = document.getElementById('viewer');
    const canvas = document.getElementById('pdf-canvas');
    const prevButton = document.getElementById('prev-page');
    const nextButton = document.getElementById('next-page');
    const pageInfo = document.getElementById('page-info');
    const pageInput = document.getElementById('page-input');
    const dropOverlay = document.getElementById('drop-overlay');

    dropZone.addEventListener('click', () => fileInput.click());

    const preventDefaults = (e) => {
        e.preventDefault();
        e.stopPropagation();
    };

    ['dragenter', 'dragover', 'dragleave', 'drop'].forEach(eventName => {
        document.addEventListener(eventName, preventDefaults, false);
    });

    function handlePDFDrop(e, files) {
        preventDefaults(e);
        
        dropZone.classList.remove('dragover');
        dropOverlay.style.display = 'none';
        
        if (files.length > 0) {
            handleFile(files[0]);
        }
    }

    ['dragenter', 'dragover'].forEach(eventName => {
        dropZone.addEventListener(eventName, (e) => {
            preventDefaults(e);
            dropZone.classList.add('dragover');
        }, false);
        
        viewer.addEventListener(eventName, (e) => {
            preventDefaults(e);
            if (pdfViewer) {
                dropOverlay.style.display = 'flex';
            }
        }, false);
    });

    ['dragleave'].forEach(eventName => {
        dropZone.addEventListener(eventName, (e) => {
            preventDefaults(e);
            dropZone.classList.remove('dragover');
        }, false);
        
        viewer.addEventListener(eventName, (e) => {
            preventDefaults(e);
            if (!viewer.contains(e.relatedTarget)) {
                dropOverlay.style.display = 'none';
            }
        }, false);
    });

    dropZone.addEventListener('drop', (e) => handlePDFDrop(e, e.dataTransfer.files), false);
    viewer.addEventListener('drop', (e) => handlePDFDrop(e, e.dataTransfer.files), false);

    fileInput.addEventListener('change', (e) => {
        if (e.target.files.length > 0) {
            handleFile(e.target.files[0]);
        }
    });

    prevButton.addEventListener('click', () => {
        if (pdfViewer && pdfViewer.previous_page()) {
            renderCurrentPage();
        }
    });

    nextButton.addEventListener('click', () => {
        if (pdfViewer && pdfViewer.next_page()) {
            renderCurrentPage();
        }
    });

    pageInput.addEventListener('keypress', (e) => {
        if (e.key === 'Enter') {
            const pageNum = parseInt(pageInput.value);
            if (pdfViewer && pdfViewer.set_page(pageNum)) {
                renderCurrentPage();
            } else {
                pageInput.value = pdfViewer ? pdfViewer.get_current_page() : 1;
            }
        }
    });

    async function handleFile(file) {
        if (file.type !== 'application/pdf') {
            console.error('Please select a PDF file.');
            return;
        }
        
        try {
            const arrayBuffer = await file.arrayBuffer();
            const uint8Array = new Uint8Array(arrayBuffer);
            await loadPDFData(uint8Array);
        } catch (error) {
            console.error('Error reading file:', error);
        }
    }

    async function loadPDFData(uint8Array) {
        try {
            pdfViewer = new PdfViewer();
            await pdfViewer.load_pdf(uint8Array);
            
            fileSelector.style.display = 'none';
            viewer.style.display = 'block';
            
            renderCurrentPage();
        } catch (error) {
            console.error('Error loading PDF:', error);
            if (error && error.toString().includes('encrypted')) {
                console.warn('PDF appears to be encrypted and cannot be opened');
            }
            pdfViewer = null;
        }
    }

    async function renderCurrentPage() {
        if (!pdfViewer) return;
        
        try {
            const pngData = pdfViewer.render_current_page();
            
            const blob = new Blob([pngData], { type: 'image/png' });
            const imageUrl = URL.createObjectURL(blob);
            
            const img = new Image();
            img.onload = () => {
                currentImage = img;
                drawImage();
                updatePageInfo();
                URL.revokeObjectURL(imageUrl);
            };
            img.onerror = () => {
                console.error('Error loading rendered image');
            };
            img.src = imageUrl;
            
        } catch (error) {
            console.error('Error rendering page:', error);
        }
    }

    function drawImage() {
        if (!currentImage) return;

        const ctx = canvas.getContext('2d');
        const dpr = window.devicePixelRatio || 1;
        
        const viewportWidth = window.innerWidth;
        const viewportHeight = window.innerHeight - 120;
        
        const scaleX = viewportWidth / currentImage.width;
        const scaleY = viewportHeight / currentImage.height;
        const scale = Math.min(scaleX, scaleY, 1);
        
        const scaledWidth = currentImage.width * scale;
        const scaledHeight = currentImage.height * scale;
        
        canvas.width = scaledWidth * dpr;
        canvas.height = scaledHeight * dpr;
        
        canvas.style.width = scaledWidth + 'px';
        canvas.style.height = scaledHeight + 'px';
        
        ctx.scale(dpr, dpr);
        
        ctx.clearRect(0, 0, scaledWidth, scaledHeight);
        ctx.drawImage(currentImage, 0, 0, scaledWidth, scaledHeight);
    }

    function updatePageInfo() {
        if (!pdfViewer) return;
        
        const currentPage = pdfViewer.get_current_page();
        const totalPages = pdfViewer.get_total_pages();
        
        pageInfo.textContent = `${currentPage} / ${totalPages}`;
        pageInput.value = currentPage;
        pageInput.max = totalPages;
        
        prevButton.disabled = currentPage === 1;
        nextButton.disabled = currentPage === totalPages;
    }

    document.addEventListener('keydown', (e) => {
        if (!pdfViewer) return;
        
        switch (e.key) {
            case 'ArrowLeft':
            case 'ArrowUp':
                e.preventDefault();
                if (pdfViewer.previous_page()) {
                    renderCurrentPage();
                }
                break;
            case 'ArrowRight':
            case 'ArrowDown':
                e.preventDefault();
                if (pdfViewer.next_page()) {
                    renderCurrentPage();
                }
                break;
        }
    });

    window.addEventListener('resize', () => {
        if (currentImage) {
            drawImage();
        }
    });

    setupLogWindow();
}

function setupLogWindow() {
    const logContent = document.getElementById('log-content');
    const clearLogsButton = document.getElementById('clear-logs');

    logContent.innerHTML = '';

    window.addLogEntry = function(level, message) {
        const logEntry = document.createElement('div');
        logEntry.className = `log-entry ${level}`;
        
        const timestamp = new Date().toLocaleTimeString();
        logEntry.innerHTML = `<span class="log-timestamp">[${timestamp}]</span>${message}`;
        
        logContent.appendChild(logEntry);
        logContent.scrollTop = logContent.scrollHeight;
    };

    clearLogsButton.addEventListener('click', () => {
        logContent.innerHTML = '';
    });
    
    window.addLogEntry('info', 'Hayro PDF Demo initialized');

    const originalConsole = {
        warn: console.warn,
        error: console.error,
        log: console.log,
        info: console.info
    };

    console.warn = function(...args) {
        originalConsole.warn.apply(console, args);
        window.addLogEntry('warn', args.join(' '));
    };

    console.error = function(...args) {
        originalConsole.error.apply(console, args);
        window.addLogEntry('error', args.join(' '));
    };

    console.log = function(...args) {
        originalConsole.log.apply(console, args);
        window.addLogEntry('info', args.join(' '));
    };

    console.info = function(...args) {
        originalConsole.info.apply(console, args);
        window.addLogEntry('info', args.join(' '));
    };
}

run().catch(console.error);
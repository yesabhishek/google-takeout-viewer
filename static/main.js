
// Minimal utility
const imageExt = ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.heic']; // keep HEIC
const videoExt = ['.mp4', '.mov', '.webm', '.mkv', '.avi'];
const isImage = (name) => imageExt.some(e => name.toLowerCase().endsWith(e));
const isVideo = (name) => videoExt.some(e => name.toLowerCase().endsWith(e));

let allItems = [];   // {name,path,handle|fileRef,type,taken,caption,album,url,people,location,size,lastModified,thumbUrl?}
let viewItems = [];
let currentIndex = 0;

const $ = (id) => document.getElementById(id);
const grid = $('grid');
const stats = $('stats');
const chips = $('chips');

(function initDefaults() {
    // Default to Photos only and Newest first
    const typeSel = $('type');
    const sortSel = $('sort');
    if (typeSel) typeSel.value = 'image';
    if (sortSel) sortSel.value = 'takenDesc';
})();

// Hide FS Access button if not supported or not secure context
(function () {
    const supported = !!window.showDirectoryPicker && (location.protocol === 'https:' || location.hostname === 'localhost');
    if (!supported) {
        document.getElementById('btn-open').classList.add('hidden');
    }
})();

const controls = {
    type: $('type'),
    sort: $('sort'),
    q: $('q'),
    album: $('album')
};

// Clear resets to Photos only
$('btn-clear').onclick = () => {
    controls.type.value = 'image';      // default to Photos
    controls.sort.value = 'takenDesc';
    controls.q.value = '';
    controls.album.value = '';
    applyFilters();
};

// Robust picker setup
(function setupPickers() {
    const btnOpen = document.getElementById('btn-open');
    const btnFallback = document.getElementById('btn-fallback');
    const btnFiles = document.getElementById('btn-files');
    const dirInput = document.getElementById('dir-input');

    const hasFSAccess = !!window.showDirectoryPicker;
    const isSecure = (location.protocol === 'https:' || location.hostname === 'localhost');
    const canUseFSAccess = hasFSAccess && isSecure;

    if (!canUseFSAccess) {
        btnOpen.classList.add('hidden');
    } else {
        btnOpen.classList.remove('hidden');
    }

    // Primary: File System Access API
    btnOpen.onclick = async () => {
        if (!canUseFSAccess) {
            alert('Folder picker requires https or localhost and a Chromium-based browser. Falling back.');
            tryOpenFallback();
            return;
        }
        try {
            const dirHandle = await showDirectoryPicker({ mode: 'read' });
            await scanRoot(dirHandle);
        } catch (e) {
            if (e && e.name !== 'AbortError') {
                console.warn('showDirectoryPicker failed, falling back:', e);
                alert('Could not open folder with File System Access API. Falling back to directory selector.');
                tryOpenFallback();
            }
        }
    };

    // Fallback: directory input
    btnFallback.onclick = () => tryOpenFallback();

    dirInput.addEventListener('change', async (e) => {
        const files = Array.from(e.target.files || []);
        if (!files.length) return;
        await scanFileList(files);
    });

    function tryOpenFallback() {
        if (dirInput) {
            if (!('webkitdirectory' in dirInput)) {
                alert('This browser may not support directory selection. A file picker will open; select files manually or use drag-and-drop.');
            }
            dirInput.click();
        } else {
            alert('Fallback directory input not found.');
        }
    }

    // Extra: Pick files (flat)
    btnFiles?.addEventListener('click', () => {
        const input = document.createElement('input');
        input.type = 'file';
        input.multiple = true;
        input.accept = imageExt.concat(videoExt).join(',');
        input.onchange = async () => {
            const files = Array.from(input.files || []);
            if (!files.length) return;
            await scanFileList(files);
        };
        input.click();
    });
})();

// Drag-and-drop folder support
document.addEventListener('dragover', e => { e.preventDefault(); });
document.addEventListener('drop', async e => {
    e.preventDefault();
    if (!e.dataTransfer || !e.dataTransfer.items) return;
    const items = Array.from(e.dataTransfer.items).filter(i => i.kind === 'file');
    // Prefer webkitGetAsEntry for recursive directories (Chrome/Safari)
    const entries = items.map(i => i.webkitGetAsEntry?.()).filter(Boolean);
    if (entries.length) {
        allItems = [];
        const albums = new Set();
        await scanEntries(entries, '', albums);
        buildAlbumSelect(albums);
        applyFilters();
        return;
    }
    // Fallback to flat file list
    const files = Array.from(e.dataTransfer.files || []);
    if (files.length) {
        await scanFileList(files);
    }
});

// ---------------------------
// Performance & thumbnails
// ---------------------------
const THUMB_W = 360;
const THUMB_H = 240;
let MAX_CONCURRENT_THUMBS = 4;
const BATCH_RENDER = 48;
const PERF_WINDOW_MS = 5000;

const perf = {
    scanned: 0,
    thumbsDone: 0,
    thumbTimes: [],
    events: [],
    lastFlush: 0
};
const $perf = { items: null, thumbs: null, rate: null, lat: null, score: null };

function now() { return performance.now(); }
function pushEvent(type) {
    const t = now();
    perf.events.push({ t, type });
    const cutoff = t - PERF_WINDOW_MS;
    while (perf.events.length && perf.events.t < cutoff) perf.events.shift();
}
function updateHUD() {
    if (!$perf.items) {
        $perf.items = document.getElementById('perf-items');
        $perf.thumbs = document.getElementById('perf-thumbs');
        $perf.rate = document.getElementById('perf-rate');
        $perf.lat = document.getElementById('perf-lat');
        $perf.score = document.getElementById('perf-score');
    }
    const t = now();
    const recent = perf.events.filter(e => e.type === 'thumb');
    const span = recent.length ? (recent[recent.length - 1].t - recent.t) : 0.0001;
    const rate = span > 0 ? (recent.length / (span / 1000)) : 0;

    const times = perf.thumbTimes.slice(-200);
    const sorted = [...times].sort((a, b) => a - b);
    const trim = Math.floor(sorted.length * 0.1);
    const trimmed = sorted.slice(trim, sorted.length - trim);
    const avg = trimmed.length ? Math.round(trimmed.reduce((s, v) => s + v, 0) / trimmed.length) : 0;

    const rateScore = Math.max(0, Math.min(100, Math.round((rate / 10) * 100)));
    const latScore = Math.max(0, Math.min(100, Math.round((1500 / Math.max(100, avg)) * 100)));
    const score = Math.round(0.6 * rateScore + 0.4 * latScore);

    $perf.items.textContent = `Scanned: ${perf.scanned}`;
    $perf.thumbs.textContent = `Thumbs: ${perf.thumbsDone}`;
    $perf.rate.textContent = `Rate: ${rate.toFixed(1)}/s`;
    $perf.lat.textContent = `Avg thumb: ${avg || '—'}ms`;
    $perf.score.textContent = `Score: ${score}`;

    $perf.score.classList.remove('perf-good', 'perf-warn', 'perf-bad');
    if (score >= 70) $perf.score.classList.add('perf-good');
    else if (score >= 40) $perf.score.classList.add('perf-warn');
    else $perf.score.classList.add('perf-bad');

    if (t - perf.lastFlush > 100) perf.lastFlush = t;
}
setInterval(updateHUD, 150);

// Try to decode to ImageBitmap with resize; gracefully handle HEIC failures
async function toThumbBlobURL(file, targetW = THUMB_W, targetH = THUMB_H) {
    const t0 = now();
    let bmp;
    try {
        // Some browsers fail on HEIC here; we'll catch and fallback
        bmp = await createImageBitmap(file, { resizeWidth: targetW, resizeHeight: targetH, resizeQuality: 'high' });
    } catch (e) {
        // Fallback path: draw via <img> -> canvas (if supported), else use original URL
        try {
            const url = URL.createObjectURL(file);
            const imgEl = await loadImg(url);
            const { blob, revoke } = await drawToBlob(imgEl, targetW, targetH);
            revoke?.();
            const thumbUrl = URL.createObjectURL(blob);
            perf.thumbTimes.push(now() - t0);
            pushEvent('thumb');
            perf.thumbsDone++;
            return thumbUrl;
        } catch {
            // Last resort: return original URL (bigger), or caller will fallback
            const url = URL.createObjectURL(file);
            perf.thumbTimes.push(now() - t0);
            pushEvent('thumb');
            perf.thumbsDone++;
            return url;
        }
    }

    let blob;
    if (self.OffscreenCanvas) {
        const cnv = new OffscreenCanvas(targetW, targetH);
        const ctx = cnv.getContext('2d', { alpha: false });
        ctx.drawImage(bmp, 0, 0, targetW, targetH);
        blob = await cnv.convertToBlob({ type: 'image/webp', quality: 0.85 });
    } else {
        const cnv = document.createElement('canvas');
        cnv.width = targetW; cnv.height = targetH;
        const ctx = cnv.getContext('2d', { alpha: false });
        ctx.drawImage(bmp, 0, 0, targetW, targetH);
        blob = await new Promise(res => cnv.toBlob(res, 'image/webp', 0.85));
    }
    bmp.close?.();
    const url = URL.createObjectURL(blob);
    perf.thumbTimes.push(now() - t0);
    pushEvent('thumb');
    perf.thumbsDone++;
    return url;
}

function loadImg(src) {
    return new Promise((resolve, reject) => {
        const img = new Image();
        img.onload = () => resolve(img);
        img.onerror = reject;
        img.src = src;
    });
}
async function drawToBlob(img, w, h) {
    if (self.OffscreenCanvas) {
        const cnv = new OffscreenCanvas(w, h);
        const ctx = cnv.getContext('2d', { alpha: false });
        ctx.drawImage(img, 0, 0, w, h);
        const blob = await cnv.convertToBlob({ type: 'image/webp', quality: 0.85 });
        const revoke = () => { try { URL.revokeObjectURL(img.src); } catch { } };
        return { blob, revoke };
    } else {
        const cnv = document.createElement('canvas');
        cnv.width = w; cnv.height = h;
        const ctx = cnv.getContext('2d', { alpha: false });
        ctx.drawImage(img, 0, 0, w, h);
        const blob = await new Promise(res => cnv.toBlob(res, 'image/webp', 0.85));
        const revoke = () => { try { URL.revokeObjectURL(img.src); } catch { } };
        return { blob, revoke };
    }
}

function createLimiter(n) {
    let active = 0;
    const q = [];
    const run = () => {
        if (!q.length || active >= n) return;
        const { fn, resolve, reject } = q.shift();
        active++;
        fn().then(v => { active--; resolve(v); run(); })
            .catch(e => { active--; reject(e); run(); });
    };
    return (fn) => new Promise((resolve, reject) => { q.push({ fn, resolve, reject }); run(); });
}
let thumbLimit = createLimiter(MAX_CONCURRENT_THUMBS);

document.addEventListener('visibilitychange', () => {
    MAX_CONCURRENT_THUMBS = document.hidden ? 2 : 4;
    thumbLimit = createLimiter(MAX_CONCURRENT_THUMBS);
}, { passive: true });

// ---------------------------
// Scanning (unchanged, but respects HEIC via isImage)
// ---------------------------
async function scanRoot(dirHandle) {
    allItems = [];
    const albumsSet = new Set();
    for await (const entry of walk(dirHandle, '')) {
        const { path, handle } = entry;
        if (handle.kind !== 'file') continue;
        const name = handle.name;
        const lower = name.toLowerCase();
        if (lower.endsWith('.json')) continue;

        const type = isImage(lower) ? 'image' : isVideo(lower) ? 'video' : '';
        if (!type) continue;

        const segs = path.split('/').filter(Boolean);
        const album = segs.length > 1 ? segs[segs.length - 2] : '';
        if (album) albumsSet.add(album);

        const file = await handle.getFile();
        const url = URL.createObjectURL(file);
        const meta = await readSidecarIfAny(dirHandle, path, name);
        const taken = meta.taken || Math.floor(file.lastModified / 1000);
        const caption = meta.caption || '';
        const people = meta.people || [];
        const location = meta.location || null;

        allItems.push({ name, path, handle, type, taken, caption, album, url, people, location, size: file.size, lastModified: file.lastModified });
        perf.scanned++;
        if ((perf.scanned % 100) === 0) updateHUD();
    }
    buildAlbumSelect(albumsSet);
    applyFilters();
}

async function* walk(dirHandle, prefix) {
    for await (const [name, handle] of dirHandle.entries()) {
        const path = prefix ? prefix + '/' + name : name;
        if (handle.kind === 'directory') {
            yield* walk(handle, path);
        } else {
            yield { path, handle };
        }
    }
}

async function scanEntries(entries, prefix, albumsSet) {
    for (const entry of entries) {
        if (!entry) continue;
        if (entry.isDirectory) {
            await new Promise((resolve, reject) => {
                const reader = entry.createReader();
                const readBatch = () => {
                    reader.readEntries(async results => {
                        if (!results.length) return resolve();
                        await scanEntries(results, prefix ? prefix + '/' + entry.name : entry.name, albumsSet);
                        readBatch();
                    }, reject);
                };
                readBatch();
            });
        } else if (entry.isFile) {
            await new Promise((resolve) => {
                entry.file(async file => {
                    const p = (file.webkitRelativePath || (prefix ? prefix + '/' + file.name : file.name));
                    const lower = file.name.toLowerCase();
                    if (lower.endsWith('.json')) return resolve();
                    const type = isImage(lower) ? 'image' : isVideo(lower) ? 'video' : '';
                    if (!type) return resolve();
                    const segs = p.split('/').filter(Boolean);
                    const album = segs.length > 1 ? segs[segs.length - 2] : '';
                    if (album) albumsSet.add(album);
                    const url = URL.createObjectURL(file);
                    allItems.push({ name: file.name, path: p, fileRef: file, type, taken: Math.floor(file.lastModified / 1000), caption: '', album, url, people: [], location: null, size: file.size, lastModified: file.lastModified });
                    perf.scanned++;
                    if ((perf.scanned % 100) === 0) updateHUD();
                    resolve();
                });
            });
        }
    }
}

async function readSidecarIfAny(rootHandle, path, name) {
    let meta = {};
    try {
        const parts = path.split('/');
        const baseDirParts = parts.slice(0, -1);
        const baseDir = await getDirectoryFromPath(rootHandle, baseDirParts);
        const jsonName = name + '.json';
        const jsonHandle = await baseDir.getFileHandle(jsonName, { create: false }).catch(() => null);
        if (jsonHandle) {
            const jf = await jsonHandle.getFile();
            const text = await jf.text();
            const data = await parseJSONIdle(text);
            const takenTs = (
                (data.photoTakenTime && Number(data.photoTakenTime.timestamp)) ||
                (data.creationTime && Number(data.creationTime.timestamp))
            );
            const caption = data.description || '';
            const people = Array.isArray(data.people) ? data.people.map(p => p?.name).filter(Boolean) : [];
            const location = data.geoData || data.geoDataExif || null;
            if (takenTs) meta.taken = takenTs;
            if (caption) meta.caption = caption;
            if (people.length) meta.people = people;
            if (location) meta.location = location;
        }
    } catch { }
    return meta;
}

async function getDirectoryFromPath(rootHandle, parts) {
    let dir = rootHandle;
    for (const p of parts) {
        if (!p) continue;
        dir = await dir.getDirectoryHandle(p);
    }
    return dir;
}

async function scanFileList(files) {
    allItems = [];
    const albumsSet = new Set();
    const sidecars = new Map();
    for (const f of files) {
        if (f.name.toLowerCase().endsWith('.json')) {
            sidecars.set(f.webkitRelativePath, f);
        }
    }
    for (const f of files) {
        const p = f.webkitRelativePath || f.name;
        const lower = f.name.toLowerCase();
        if (lower.endsWith('.json')) continue;
        const type = isImage(lower) ? 'image' : isVideo(lower) ? 'video' : '';
        if (!type) continue;

        const segs = p.split('/').filter(Boolean);
        const album = segs.length > 1 ? segs[segs.length - 2] : '';
        if (album) albumsSet.add(album);

        const sidecarKey = p + '.json';
        let meta = {};
        if (sidecars.has(sidecarKey)) {
            try {
                const text = await sidecars.get(sidecarKey).text();
                const data = await parseJSONIdle(text);
                const takenTs = (
                    (data.photoTakenTime && Number(data.photoTakenTime.timestamp)) ||
                    (data.creationTime && Number(data.creationTime.timestamp))
                );
                const caption = data.description || '';
                const people = Array.isArray(data.people) ? data.people.map(p => p?.name).filter(Boolean) : [];
                const location = data.geoData || data.geoDataExif || null;
                if (takenTs) meta.taken = takenTs;
                if (caption) meta.caption = caption;
                if (people?.length) meta.people = people;
                if (location) meta.location = location;
            } catch { }
        }

        const url = URL.createObjectURL(f);
        const taken = meta.taken || Math.floor(f.lastModified / 1000);
        const caption = meta.caption || '';
        const people = meta.people || [];
        const location = meta.location || null;

        allItems.push({ name: f.name, path: p, fileRef: f, type, taken, caption, album, url, people, location, size: f.size, lastModified: f.lastModified });
        perf.scanned++;
        if ((perf.scanned % 100) === 0) updateHUD();
    }
    buildAlbumSelect(albumsSet);
    applyFilters();
}

function buildAlbumSelect(albumsSet) {
    const albumSel = controls.album;
    albumSel.innerHTML = '';
    const optAll = document.createElement('option');
    optAll.value = '';
    optAll.textContent = 'All folders';
    albumSel.appendChild(optAll);

    const albums = Array.from(albumsSet).sort((a, b) => a.localeCompare(b));
    for (const a of albums) {
        const opt = document.createElement('option');
        opt.value = a;
        opt.textContent = a;
        albumSel.appendChild(opt);
    }
}

// Filtering/sorting
for (const el of [controls.type, controls.sort, controls.album]) {
    el.addEventListener('change', applyFilters);
}
controls.q.addEventListener('input', debounce(applyFilters, 200));

// Default apply on load
window.addEventListener('load', applyFilters);

function applyFilters() {
    const q = controls.q.value.trim().toLowerCase();
    const type = controls.type.value; // 'image' | 'video' | '' (both)
    const album = controls.album.value;
    const sort = controls.sort.value;

    viewItems = allItems.filter(it => {
        if (type && it.type !== type) return false; // if 'image' => only images; if '' => both
        if (album && it.album !== album) return false;
        if (q && !(it.name.toLowerCase().includes(q) || it.caption.toLowerCase().includes(q))) return false;
        return true;
    });

    switch (sort) {
        case 'takenAsc': viewItems.sort((a, b) => a.taken - b.taken); break;
        case 'nameAsc': viewItems.sort((a, b) => a.name.localeCompare(b.name)); break;
        case 'nameDesc': viewItems.sort((a, b) => b.name.localeCompare(a.name)); break;
        default: viewItems.sort((a, b) => b.taken - a.taken); break;
    }

    stats.textContent = `${viewItems.length} items`;
    renderGrid();
    renderChips(type, album, q);
}

function renderChips(type, album, q) {
    chips.innerHTML = '';
    if (type) chips.appendChild(chip(`Type: ${type === 'image' ? 'Photos' : 'Videos'}`));
    else chips.appendChild(chip('Type: Photos & Videos'));
    if (album) chips.appendChild(chip(`Folder: ${album}`));
    if (q) chips.appendChild(chip(`Search: “${q}”`));
}
function chip(text) {
    const el = document.createElement('span');
    el.className = 'chip';
    el.textContent = text;
    return el;
}

// Incremental grid rendering with thumbnail jobs
async function renderGrid() {
    grid.innerHTML = '';
    const frag = document.createDocumentFragment();
    const toThumbs = [];

    for (let i = 0; i < viewItems.length; i++) {
        const it = viewItems[i];

        const card = document.createElement('div');
        card.className = 'card';

        const ph = document.createElement('div');
        ph.className = 'thumb';
        ph.style.display = 'flex';
        ph.style.alignItems = 'center';
        ph.style.justifyContent = 'center';
        ph.style.background = '#11151d';
        ph.textContent = (it.type === 'video') ? 'Loading video…' : 'Loading image…';
        card.appendChild(ph);

        if (it.type === 'video') {
            const b = document.createElement('div');
            b.className = 'badge';
            b.textContent = 'Video';
            card.appendChild(b);
        }

        const meta = document.createElement('div');
        meta.className = 'meta';
        const d = new Date(it.taken * 1000);
        const left = document.createElement('div');
        left.textContent = d.toLocaleString();
        const right = document.createElement('div');
        right.className = 'dim';
        right.textContent = it.album || '—';
        meta.appendChild(left);
        meta.appendChild(right);
        card.appendChild(meta);

        card.addEventListener('click', () => openLightbox(i));
        frag.appendChild(card);

        if (it.type === 'image') {
            toThumbs.push({ idx: i, job: () => makeImageThumb(it, card, ph) });
        } else {
            toThumbs.push({ idx: i, job: () => makeVideoThumb(it, card, ph) });
        }

        if ((i + 1) % BATCH_RENDER === 0) {
            grid.appendChild(frag);
        }
    }
    if (frag.childNodes.length) grid.appendChild(frag);

    toThumbs.sort((a, b) => a.idx - b.idx);
    await Promise.all(toThumbs.map(({ job }) => thumbLimit(job)));
}

async function makeImageThumb(it, card, ph) {
    // Reuse cached thumb if available
    if (it.thumbUrl) {
        const img = document.createElement('img');
        img.className = 'thumb';
        img.loading = 'lazy';
        img.src = it.thumbUrl;
        img.alt = it.name;
        card.replaceChild(img, ph);
        perf.thumbsDone++; pushEvent('thumb');
        return;
    }
    const file = it.fileRef || await it.handle.getFile();
    try {
        const url = await toThumbBlobURL(file);
        const img = document.createElement('img');
        img.className = 'thumb';
        img.loading = 'lazy';
        img.src = url;
        img.alt = it.name;
        card.replaceChild(img, ph);
        it.thumbUrl = url;
    } catch {
        // As ultimate fallback, try original URL (may not decode HEIC in some browsers)
        const img = document.createElement('img');
        img.className = 'thumb';
        img.loading = 'lazy';
        img.src = it.url;
        img.alt = it.name;
        card.replaceChild(img, ph);
    }
}

async function makeVideoThumb(it, card, ph) {
    const vid = document.createElement('video');
    vid.className = 'thumb';
    vid.muted = true;
    vid.playsInline = true;
    vid.preload = 'metadata';
    vid.src = it.url;
    card.replaceChild(vid, ph);
}

// Lightbox (unchanged)
const lb = $('lightbox');
const lbStage = $('lb-stage');
const lbMeta = $('lb-meta');
$('lb-close').onclick = () => lb.close();
$('lb-prev').onclick = () => step(-1);
$('lb-next').onclick = () => step(1);
$('lb-open').onclick = () => {
    const it = viewItems[currentIndex];
    if (!it) return;
    window.open(it.url, '_blank');
};
function openLightbox(idx) {
    currentIndex = idx;
    renderLightbox();
    lb.showModal();
}
function step(delta) {
    if (!viewItems.length) return;
    currentIndex = (currentIndex + delta + viewItems.length) % viewItems.length;
    renderLightbox();
}
function renderLightbox() {
    lbStage.innerHTML = '';
    lbMeta.innerHTML = '';
    const it = viewItems[currentIndex];
    if (!it) return;
    const el = document.createElement(it.type === 'image' ? 'img' : 'video');
    el.src = it.url;
    if (it.type === 'video') {
        el.controls = true;
        el.preload = 'metadata';
    } else {
        el.alt = it.name;
        el.decoding = 'async';
        el.loading = 'eager';
    }
    lbStage.appendChild(el);

    const d = new Date(it.taken * 1000).toLocaleString();
    lbMeta.innerHTML = `
    <div style="display:flex; gap:8px; align-items:center; flex-wrap:wrap; width:100%">
      <div>${it.name}</div>
      <div class="dim">- </div>
      <div class="dim">${d}</div>
      ${it.album ? `<div class="dim">- </div><div>${it.album}</div>` : ''}
      ${it.caption ? `<div class="dim">- </div><div class="dim">${escapeHtml(it.caption)}</div>` : ''}
    </div>
  `;
}

function escapeHtml(s) {
    return s.replace(/[&<>"']/g, m => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[m]));
}

// Debounce util
function debounce(fn, ms) {
    let t;
    return (...args) => {
        clearTimeout(t);
        t = setTimeout(() => fn.apply(null, args), ms);
    };
}

// Idle JSON parsing
function parseJSONIdle(text) {
    return new Promise((resolve) => {
        if ('requestIdleCallback' in window) {
            requestIdleCallback(() => resolve(JSON.parse(text)), { timeout: 100 });
        } else {
            setTimeout(() => resolve(JSON.parse(text)), 0);
        }
    });
}

// Cleanup URLs on unload
window.addEventListener('beforeunload', () => {
    for (const it of allItems) {
        try { URL.revokeObjectURL(it.url); } catch { }
        if (it.thumbUrl) { try { URL.revokeObjectURL(it.thumbUrl); } catch { } }
    }
});
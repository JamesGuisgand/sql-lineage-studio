'use strict';

// The SQL engine runs server-side, compiled from the original Rust source.
// The browser only receives JSON + Graphviz DOT: no SQL is stored persistently.
const $ = (id) => document.getElementById(id);
const SVG_NS = 'http://www.w3.org/2000/svg';
const state = { result: null, tab: 'graph', zoom: 1, svg: null, running: false, toastTimer: null };

function toast(message) {
  const node = $('toast');
  node.textContent = message;
  node.classList.add('visible');
  if (state.toastTimer) clearTimeout(state.toastTimer);
  state.toastTimer = setTimeout(() => node.classList.remove('visible'), 3200);
}

function updateEditor() {
  const textarea = $('sql-input');
  const count = textarea.value.split('\n').length;
  $('line-numbers').textContent = Array.from({ length: Math.min(count, 10000) }, (_, n) => n + 1).join('\n');
  $('editor-count').textContent = `${count} ligne${count !== 1 ? 's' : ''} · ${textarea.value.length.toLocaleString('fr-FR')} caractère${textarea.value.length !== 1 ? 's' : ''}`;
}

function setLoading(loading) {
  state.running = loading;
  $('analyze-btn').disabled = loading;
  $('analyze-label').textContent = loading ? 'Analyse en cours...' : 'Analyser le SQL';
  if (loading) {
    $('result-state').className = 'result-state';
    $('result-state').lastChild.textContent = ' Analyse...';
  }
}

function setStatus(value, success) {
  $('result-state').className = 'result-state ' + (success ? 'ready' : 'failed');
  $('result-state').lastChild.textContent = ' ' + value;
}

async function analyze() {
  if (state.running) return;
  const sql = $('sql-input').value;
  $('error-alert').hidden = true;
  if (!sql.trim()) {
    $('error-alert').textContent = 'Collez une instruction SQL ou chargez l’exemple.';
    $('error-alert').hidden = false;
    $('sql-input').focus();
    return;
  }
  if (new TextEncoder().encode(sql).length > 131072) {
    $('error-alert').textContent = 'Le fichier SQL dépasse la limite de 128 Kio.';
    $('error-alert').hidden = false;
    return;
  }
  setLoading(true);
  try {
    const response = await fetch('/api/analyze', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ sql, dialect: $('dialect').value }), signal: AbortSignal.timeout(20000)
    });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error || `Erreur HTTP ${response.status}`);
    state.result = data;
    for (const key of ['statements', 'sources', 'targets', 'relations']) {
      $('stat-' + key).textContent = data.stats[key];
    }
    $('json-output').textContent = JSON.stringify(data, null, 2);
    $('dot-output').textContent = data.dot;
    $('analysis-time').textContent = `Moteur Rust · ${data.stats.duration_ms.toLocaleString('fr-FR')} ms`;
    $('download-json').disabled = false;
    $('download-dot').disabled = false;
    $('download-svg').disabled = false;
    buildTable(data.statements);
    buildColumns(data.columns || []);
    showWarnings(data.warnings || []);
    populateImpactTables(data.statements);
    drawGraph(data.statements);
    setStatus('Terminé', true);
    toast(`${data.stats.statements} instruction${data.stats.statements > 1 ? 's' : ''} analysée${data.stats.statements > 1 ? 's' : ''}`);
  } catch (error) {
    $('error-alert').textContent = error instanceof Error ? error.message : 'Erreur réseau.';
    $('error-alert').hidden = false;
    setStatus('Erreur', false);
  } finally {
    setLoading(false);
  }
}

function buildColumns(columns) {
  const wrapper = $('column-results');
  wrapper.replaceChildren();
  $('column-count').textContent = `${columns.length} relation(s) de colonne`;
  if (!columns.length) {
    wrapper.appendChild(el('p', 'empty-text', 'Aucune dépendance de colonne résolue. Consultez les avertissements (notamment SELECT *).'));
    return;
  }
  columns.slice(0, 500).forEach((edge) => {
    const item = el('div', 'lineage-edge');
    item.append(el('code', 'edge-source', edge.source),
                el('span', 'edge-arrow', '→'),
                el('code', 'edge-target', edge.target));
    wrapper.appendChild(item);
  });
  if (columns.length > 500) wrapper.appendChild(el('p', 'empty-text', 'Aperçu limité à 500 relations ; export JSON complet.'));
}

function showWarnings(warnings) {
  const box = $('analysis-warnings');
  box.replaceChildren();
  box.hidden = warnings.length === 0;
  if (!warnings.length) return;
  const details = el('details');
  details.appendChild(el('summary', '', `${warnings.length} avertissement(s) sur la précision du lineage`));
  const list = el('ul');
  warnings.slice(0, 30).forEach((warning) => list.appendChild(el('li', '', warning)));
  if (warnings.length > 30) list.appendChild(el('li', '', 'Avertissements supplémentaires dans le JSON.'));
  details.appendChild(list);
  box.appendChild(details);
}

function populateImpactTables(statements) {
  const select = $('impact-table');
  const values = new Set();
  statements.forEach(s => {
    if (s.target) values.add(s.target);
    (s.sources || []).forEach(v => values.add(v));
  });
  const previous = select.value;
  select.replaceChildren();
  Array.from(values).sort().forEach(table => {
    const option = document.createElement('option'); option.value = table;
    option.textContent = table; select.appendChild(option);
  });
  if (values.has(previous)) select.value = previous;
  $('impact-run').disabled = values.size === 0;
  $('impact-results').replaceChildren(el('p', 'empty-text', 'Choisissez une table et cliquez sur Calculer l’impact.'));
}

async function runImpact() {
  const table = $('impact-table').value;
  if (!table || state.running) return;
  $('impact-run').disabled = true;
  $('impact-results').replaceChildren(el('p', 'empty-text', 'Calcul de l’impact en cours…'));
  try {
    const response = await fetch('/api/analyze', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ sql: $('sql-input').value, dialect: $('dialect').value, impact_table: table }),
      signal: AbortSignal.timeout(20000),
    });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error || `Erreur HTTP ${response.status}`);
    const impact = data.impact;
    const wrapper = $('impact-results');
    wrapper.replaceChildren();
    wrapper.appendChild(el('p', 'impact-summary', `${impact.affected.length} objet(s) impacté(s) depuis ${impact.start}.`));
    if (!impact.paths.length) wrapper.appendChild(el('p', 'empty-text', 'Aucune dépendance descendante détectée.'));
    impact.paths.forEach(path => {
      const item = el('div', 'lineage-edge');
      item.appendChild(el('code', 'edge-target', path.path.join('  →  ')));
      item.appendChild(el('span', 'impact-distance', `${path.distance} niveau(x)`));
      wrapper.appendChild(item);
    });
    if (impact.cycles?.length) wrapper.appendChild(el('p', 'impact-warning', `${impact.cycles.length} cycle(s) détecté(s) dans le graphe.`));
  } catch (error) {
    $('impact-results').replaceChildren(el('p', 'impact-warning', error.message || 'Erreur d’analyse d’impact.'));
  } finally {
    $('impact-run').disabled = false;
  }
}

function showTab(tab) {
  state.tab = tab;
  for (const name of ['graph', 'tables', 'columns', 'impact', 'json', 'dot']) {
    const active = name === tab;
    $('tab-' + name).classList.toggle('active', active);
    $('tab-' + name).setAttribute('aria-selected', String(active));
    $('panel-' + name).hidden = !active;
  }
}

function el(tag, className, text) {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (text !== undefined) element.textContent = String(text);
  return element;
}

function buildTable(statements) {
  const wrapper = $('table-results');
  wrapper.replaceChildren();
  if (!statements.length) {
    wrapper.appendChild(el('p', 'empty-text', 'Aucune instruction SQL trouvée.'));
    return;
  }
  statements.forEach((statement, index) => {
    const card = el('div', 'table-item');
    const head = el('div', 'table-item-head');
    head.append(el('strong', '', `Instruction #${index + 1}`), el('span', 'table-kind', statement.kind));
    card.appendChild(head);
    const target = el('div', 'table-detail');
    target.append(el('span', '', 'Cible'));
    const targetContent = el('div');
    targetContent.append(el('span', statement.target ? 'tag target' : '', statement.target || 'Aucune (SELECT)'));
    target.append(targetContent);
    card.append(target);
    const sources = el('div', 'table-detail');
    sources.append(el('span', '', 'Sources'));
    const values = el('div', 'tag-list');
    if (statement.sources?.length) {
      statement.sources.forEach((source) => values.append(el('span', 'tag', source)));
    } else {
      values.append(el('span', '', 'Aucune table détectée'));
    }
    sources.append(values);
    card.append(sources);
    wrapper.appendChild(card);
  });
}

function svgEl(name, attrs = {}, text) {
  const node = document.createElementNS(SVG_NS, name);
  Object.entries(attrs).forEach(([key, val]) => node.setAttribute(key, String(val)));
  if (text !== undefined) node.textContent = String(text); // Never inject SQL as markup.
  return node;
}

function computeGraph(statements) {
  const sources = new Set(), targets = new Set(), links = new Map();
  for (const statement of statements) {
    const srcs = Array.isArray(statement.sources) ? statement.sources : [];
    srcs.forEach((source) => sources.add(source));
    if (statement.target) {
      targets.add(statement.target);
      srcs.forEach((source) => links.set(`${source}\u0000${statement.target}`, [source, statement.target]));
    }
  }
  const all = new Set([...sources, ...targets]);
  const sourceOnly = [...all].filter((x) => sources.has(x) && !targets.has(x)).sort();
  const both = [...all].filter((x) => sources.has(x) && targets.has(x)).sort();
  const targetOnly = [...all].filter((x) => targets.has(x) && !sources.has(x)).sort();
  return { all, sourceOnly, both, targetOnly, links: [...links.values()] };
}

function drawGraph(statements) {
  const graph = computeGraph(statements);
  const holder = $('graph-content');
  holder.replaceChildren();
  if (!graph.all.size) {
    holder.hidden = true;
    $('graph-placeholder').hidden = false;
    state.svg = null;
    $('download-svg').disabled = true;
    return;
  }
  holder.hidden = false;
  $('graph-placeholder').hidden = true;
  const width = 1020, nodeWidth = 242, nodeHeight = 55, left = 40, right = width - left - nodeWidth;
  const hasBoth = graph.both.length > 0;
  const columns = hasBoth
    ? [{ names: graph.sourceOnly, x: left, color: '#8aaeff', label: 'SOURCES' },
       { names: graph.both, x: (width - nodeWidth) / 2, color: '#c39af3', label: 'INTERMÉDIAIRES' },
       { names: graph.targetOnly, x: right, color: '#65e2be', label: 'CIBLES' }]
    : [{ names: graph.sourceOnly, x: 140, color: '#8aaeff', label: 'SOURCES' },
       { names: graph.targetOnly, x: 638, color: '#65e2be', label: 'CIBLES' }];
  const maximum = Math.max(...columns.map((c) => c.names.length), 1);
  const height = Math.max(300, 140 + maximum * 83);
  const svg = svgEl('svg', { xmlns: SVG_NS, viewBox: `0 0 ${width} ${height}`, width, height,
    role: 'img', 'aria-label': 'Graphe des dépendances de tables SQL' });
  svg.append(svgEl('title', {}, 'Dépendances des tables SQL'));
  const defs = svgEl('defs');
  const marker = svgEl('marker', { id: 'arrow', markerWidth: 9, markerHeight: 9, refX: 8, refY: 4.5, orient: 'auto' });
  marker.append(svgEl('path', { d: 'M0 0 L9 4.5 L0 9 Z', fill: '#527c80' }));
  defs.append(marker);
  svg.append(defs);
  svg.append(svgEl('rect', { x: 0, y: 0, width, height, fill: '#0b1a24' }));
  const layout = new Map();
  const drawable = columns.map((column) => ({ ...column, names: column.names.slice(0, 80) }));
  for (const column of drawable) {
    if (!column.names.length) continue;
    svg.append(svgEl('text', { x: column.x, y: 38, fill: '#74919d', 'font-family': 'Arial,sans-serif',
      'font-size': 13, 'font-weight': 700, 'letter-spacing': '1.5' }, column.label));
    const step = 83;
    column.names.forEach((name, index) => {
      const y = 70 + index * step;
      layout.set(name, { x: column.x, y, color: column.color });
    });
  }
  for (const [source, target] of graph.links) {
    const from = layout.get(source), to = layout.get(target);
    if (!from || !to) continue;
    const x1 = from.x + nodeWidth, y1 = from.y + nodeHeight / 2;
    const x2 = to.x, y2 = to.y + nodeHeight / 2;
    let d;
    if (x2 > x1 + 14) {
      const bend = Math.max(70, (x2 - x1) * .42);
      d = `M ${x1} ${y1} C ${x1+bend} ${y1}, ${x2-bend} ${y2}, ${x2} ${y2}`;
    } else {
      d = `M ${x1} ${y1} C ${x1+52} ${y1}, ${x2-52} ${y2-45}, ${x2} ${y2}`;
    }
    svg.append(svgEl('path', { d, fill: 'none', stroke: '#507985', 'stroke-width': 2,
      'marker-end': 'url(#arrow)', opacity: .88 }));
  }
  for (const [name, item] of layout) {
    const { x, y, color } = item;
    const group = svgEl('g');
    group.append(svgEl('rect', { x, y, width: nodeWidth, height: nodeHeight, rx: 8,
      fill: '#132c37', stroke: color, 'stroke-width': 1.35 }));
    group.append(svgEl('rect', { x: x+13, y: y+19, width: 17, height: 16, rx: 3,
      fill: color, opacity: .2 }));
    group.append(svgEl('path', { d: `M${x+17} ${y+23}h9m-9 4h9m-9 4h6`,
      fill: 'none', stroke: color, 'stroke-width': 1.3, 'stroke-linecap': 'round' }));
    const shortened = name.length > 26 ? name.slice(0, 24) + '…' : name;
    group.append(svgEl('text', { x: x+40, y: y+33, fill: '#e0eeee',
      'font-family': 'ui-monospace,Consolas,monospace', 'font-size': 14 }, shortened));
    group.append(svgEl('title', {}, name));
    svg.append(group);
  }
  if (graph.all.size > 160) {
    svg.append(svgEl('text', { x: 40, y: height - 15, fill: '#edaf89',
      'font-family': 'Arial', 'font-size': 12 }, 'Aperçu limité à 80 tables par colonne. Exports JSON/DOT complets.'));
  }
  state.svg = svg;
  state.zoom = 1;
  $('zoom-value').textContent = '100%';
  holder.appendChild(svg);
  applyZoom();
}

function applyZoom() {
  if (!state.svg) return;
  state.svg.style.width = `${Math.round(state.zoom * 100)}%`;
  state.svg.style.height = 'auto';
  $('zoom-value').textContent = `${Math.round(state.zoom * 100)}%`;
}
function zoomBy(delta) { state.zoom = Math.min(2.5, Math.max(.5, Math.round((state.zoom + delta) * 10) / 10)); applyZoom(); }

function downloadBlob(filename, contents, mime) {
  const href = URL.createObjectURL(new Blob([contents], { type: mime }));
  const a = document.createElement('a');
  a.href = href; a.download = filename;
  document.body.append(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(href), 2000);
}

async function copyText(text) {
  if (!state.result) return toast('Aucun résultat à copier.');
  try { await navigator.clipboard.writeText(text); toast('Copié dans le presse-papiers.'); }
  catch { toast('Copie indisponible dans ce navigateur.'); }
}

async function loadExample() {
  try {
    const response = await fetch('/example.sql');
    if (!response.ok) throw new Error('Exemple indisponible');
    $('sql-input').value = await response.text();
    updateEditor();
    await analyze();
  } catch { toast('Impossible de charger le SQL de démonstration.'); }
}

async function upload(file) {
  if (!file) return;
  if (file.size > 131072) { toast('Fichier trop volumineux (maximum 128 Kio).'); return; }
  const text = await file.text();
  $('sql-input').value = text;
  updateEditor();
  toast('Fichier importé : ' + file.name);
}

function wire() {
  const textarea = $('sql-input');
  textarea.addEventListener('input', updateEditor);
  textarea.addEventListener('scroll', () => { $('line-numbers').scrollTop = textarea.scrollTop; });
  textarea.addEventListener('keydown', (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') { event.preventDefault(); analyze(); }
    if (event.key === 'Tab') {
      event.preventDefault();
      const pos = textarea.selectionStart, end = textarea.selectionEnd;
      textarea.setRangeText('  ', pos, end, 'end');
      updateEditor();
    }
  });
  $('analyze-btn').addEventListener('click', analyze);
  $('impact-run').addEventListener('click', runImpact);
  $('load-example').addEventListener('click', loadExample);
  $('clear-sql').addEventListener('click', () => { textarea.value = ''; updateEditor(); textarea.focus(); });
  $('upload-file').addEventListener('click', () => $('file-input').click());
  $('file-input').addEventListener('change', (e) => { upload(e.target.files?.[0]); e.target.value = ''; });
  document.querySelectorAll('[data-tab]').forEach((button) => button.addEventListener('click', () => showTab(button.dataset.tab)));
  $('zoom-in').addEventListener('click', () => zoomBy(.2));
  $('zoom-out').addEventListener('click', () => zoomBy(-.2));
  $('zoom-reset').addEventListener('click', () => { state.zoom = 1; applyZoom(); });
  $('copy-json').addEventListener('click', () => copyText($('json-output').textContent));
  $('copy-dot').addEventListener('click', () => copyText($('dot-output').textContent));
  $('download-json').addEventListener('click', () => state.result && downloadBlob('sql-lineage.json', $('json-output').textContent + '\n', 'application/json'));
  $('download-dot').addEventListener('click', () => state.result && downloadBlob('sql-lineage.dot', state.result.dot, 'text/vnd.graphviz'));
  $('download-svg').addEventListener('click', () => {
    if (!state.svg) return;
    const svg = state.svg.cloneNode(true);
    svg.removeAttribute('style');
    svg.setAttribute('width', '1020');
    downloadBlob('sql-lineage.svg', new XMLSerializer().serializeToString(svg), 'image/svg+xml');
  });
  updateEditor();
  loadExample();
}

document.addEventListener('DOMContentLoaded', wire);

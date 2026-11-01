const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => Array.from(document.querySelectorAll(selector));

const state = {
  config: null,
  invoices: [],
  selected: new Set(),
  dirty: new Map(),
  syncing: false,
};

const FIELDS = ['date', 'amount', 'seller', 'no'];

/* ------------------------------------------------------------------ 基础工具 */
let toastTimer = null;
function toast(message, kind = '') {
  const node = $('#toast');
  node.textContent = message;
  node.className = `toast show ${kind}`;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    node.className = 'toast';
  }, 4200);
}

function logLine(message) {
  const box = $('#log');
  const line = document.createElement('div');
  if (String(message).startsWith('  ✕')) line.className = 'err';
  else if (String(message).includes('✓') || String(message).includes('＋')) line.className = 'ok';
  line.textContent = message;
  box.appendChild(line);
  box.scrollTop = box.scrollHeight;
  while (box.childElementCount > 800) box.removeChild(box.firstElementChild);
}

function errorText(error) {
  if (typeof error === 'string') return error;
  if (error && typeof error === 'object' && 'message' in error) return String(error.message);
  return String(error);
}

/* ------------------------------------------------------------------ 视图切换 */
function showView(name) {
  $$('.view').forEach((view) => view.classList.toggle('active', view.id === `view-${name}`));
  $$('.nav-item').forEach((item) => item.classList.toggle('active', item.dataset.view === name));
}

/* ------------------------------------------------------------------ 配置表单 */
function fillForm(config) {
  $('#imapHost').value = config.imapHost || '';
  $('#imapPort').value = config.imapPort || 993;
  $('#imapTls').value = config.imapTls === 'starttls' ? 'starttls' : 'tls';
  $('#username').value = config.username || '';
  $('#password').value = config.password || '';
  $('#folder').value = config.folder || 'INBOX';
  $('#sinceDays').value = config.sinceDays ?? 90;
  $('#maxMails').value = config.maxMails ?? 500;
  $('#maxLinksPerMail').value = config.maxLinksPerMail ?? 2;
  $('#keywordFilter').value = config.keywordFilter || '';
  $('#namingTemplate').value = config.namingTemplate || '{date}_{amount}_{seller}_{no}';
  $('#template').value = config.namingTemplate || '{date}_{amount}_{seller}_{no}';
}

function readForm() {
  const number = (selector, fallback, min, max) => {
    const parsed = Number($(selector).value);
    if (!Number.isFinite(parsed)) return fallback;
    return Math.min(Math.max(Math.trunc(parsed), min), max);
  };
  return {
    imapHost: $('#imapHost').value.trim(),
    imapPort: number('#imapPort', 993, 1, 65535),
    imapTls: $('#imapTls').value,
    username: $('#username').value.trim(),
    password: $('#password').value,
    folder: $('#folder').value.trim() || 'INBOX',
    sinceDays: number('#sinceDays', 90, 1, 3650),
    maxMails: number('#maxMails', 500, 1, 5000),
    maxLinksPerMail: number('#maxLinksPerMail', 2, 0, 10),
    keywordFilter: $('#keywordFilter').value.trim(),
    namingTemplate: $('#namingTemplate').value.trim() || '{date}_{amount}_{seller}_{no}',
  };
}

function updateSummary() {
  const config = state.config || readForm();
  $('#sum-host').textContent =
    config.imapHost ? `${config.imapHost}:${config.imapPort}` : '未配置';
  $('#sum-folder').textContent = config.folder || 'INBOX';
  $('#sum-range').textContent = `最近 ${config.sinceDays || 90} 天`;
  $('#sum-template').textContent = config.namingTemplate || '{date}_{amount}_{seller}_{no}';
}

function previewName(template) {
  const sample = {
    date: '2024-03-01',
    amount: '1299.00',
    seller: '腾讯科技有限公司',
    no: '24312000000123456789',
    from: '腾讯云',
    subject: '您的电子发票',
    kind: 'attachment',
  };
  let out = template || '{date}_{amount}_{seller}_{no}';
  for (const [key, value] of Object.entries(sample)) {
    out = out.split(`{${key}}`).join(value);
  }
  out = out
    .replace(/[\\/:*?"<>|]/g, '-')
    .replace(/_+/g, '_')
    .replace(/^[_\-. ]+/, '')
    .replace(/[_\-. ]+$/, '');
  return `${out || 'invoice'}.pdf`;
}

function updatePreview() {
  $('#template-preview').textContent = previewName($('#namingTemplate').value);
}

async function saveConfig(quiet = false) {
  const config = readForm();
  try {
    await invoke('save_config', { config });
    state.config = config;
    updateSummary();
    if (!quiet) toast('配置已保存', 'ok');
    return true;
  } catch (error) {
    toast(`保存失败：${errorText(error)}`, 'error');
    return false;
  }
}

async function testConnection() {
  const button = $('#btn-test');
  button.disabled = true;
  $('#test-output').textContent = '正在连接…';
  try {
    const config = readForm();
    const result = await invoke('test_connection', { config });
    $('#test-output').textContent = result;
    toast('连接成功', 'ok');
  } catch (error) {
    $('#test-output').textContent = `连接失败：\n${errorText(error)}`;
    toast(`连接失败：${errorText(error)}`, 'error');
  } finally {
    button.disabled = false;
  }
}

/* ------------------------------------------------------------------ 收信 */
function setSyncing(running) {
  state.syncing = running;
  const button = $('#btn-sync');
  button.disabled = running;
  button.textContent = running ? '正在收取…' : '开始收取发票';
  $('#progress').classList.toggle('hidden', !running);
  const pill = $('#sync-state');
  pill.className = `pill ${running ? 'running' : 'idle'}`;
  pill.textContent = running ? '运行中' : '待机';
}

function renderSummary(report) {
  const box = $('#sync-summary');
  box.classList.remove('hidden');
  box.innerHTML = '';
  const items = [
    ['扫描邮件', report.scannedMails],
    ['发现 PDF', report.foundPdfs],
    ['新增发票', report.newCount],
    ['重复跳过', report.duplicates],
    ['错误', report.errors ? report.errors.length : 0],
  ];
  for (const [label, value] of items) {
    const span = document.createElement('span');
    span.textContent = `${label} `;
    const strong = document.createElement('b');
    strong.textContent = String(value ?? 0);
    span.appendChild(strong);
    box.appendChild(span);
  }
}

async function startSync() {
  if (state.syncing) return;
  const config = readForm();
  if (!config.imapHost || !config.username || !config.password) {
    showView('settings');
    toast('请先在设置里填好服务器、账号和授权码', 'error');
    return;
  }
  await saveConfig(true);
  logLine(`—— ${new Date().toLocaleString()} ——`);
  setSyncing(true);
  $('#sync-summary').classList.add('hidden');
  try {
    const report = await invoke('start_sync', { config });
    renderSummary(report);
    $('#sync-state').className = 'pill done';
    $('#sync-state').textContent = '完成';
    toast(`完成：新增 ${report.newCount} 个发票`, 'ok');
    await loadInvoices();
  } catch (error) {
    logLine(`✕ ${errorText(error)}`);
    $('#sync-state').className = 'pill error';
    $('#sync-state').textContent = '失败';
    toast(`收取失败：${errorText(error)}`, 'error');
  } finally {
    state.syncing = false;
    $('#btn-sync').disabled = false;
    $('#btn-sync').textContent = '开始收取发票';
    $('#progress').classList.add('hidden');
  }
}

/* ------------------------------------------------------------------ 发票库 */
async function loadInvoices() {
  try {
    state.invoices = await invoke('list_invoices');
  } catch (error) {
    toast(`读取发票库失败：${errorText(error)}`, 'error');
    state.invoices = [];
  }
  const alive = new Set(state.invoices.map((item) => item.hash));
  [...state.selected].forEach((hash) => {
    if (!alive.has(hash)) state.selected.delete(hash);
  });
  $('#count-badge').textContent = String(state.invoices.length);
  renderRows();
}

function matchesQuery(invoice, query) {
  if (!query) return true;
  return [
    invoice.fileName,
    invoice.seller,
    invoice.no,
    invoice.amount,
    invoice.date,
    invoice.fromAddr,
    invoice.subject,
  ]
    .filter(Boolean)
    .some((value) => String(value).toLowerCase().includes(query));
}

function cellInput(invoice, field) {
  const input = document.createElement('input');
  input.className = 'cell-input';
  input.dataset.field = field;
  input.dataset.hash = invoice.hash;
  input.value = invoice[field] || '';
  if (state.dirty.get(invoice.hash)?.[field] !== undefined) {
    input.value = state.dirty.get(invoice.hash)[field];
    input.classList.add('dirty');
  }
  input.addEventListener('input', () => {
    const current = state.dirty.get(invoice.hash) || {};
    current[field] = input.value;
    state.dirty.set(invoice.hash, current);
    input.classList.add('dirty');
  });
  return input;
}

function iconButton(title, glyph, onClick) {
  const button = document.createElement('button');
  button.className = 'icon-btn';
  button.title = title;
  button.textContent = glyph;
  button.addEventListener('click', onClick);
  return button;
}

function renderRows() {
  const query = $('#search').value.trim().toLowerCase();
  const body = $('#rows');
  body.innerHTML = '';
  const visible = state.invoices.filter((invoice) => matchesQuery(invoice, query));
  $('#empty').classList.toggle('show', visible.length === 0);

  for (const invoice of visible) {
    const tr = document.createElement('tr');
    if (state.selected.has(invoice.hash)) tr.classList.add('selected');

    const checkCell = document.createElement('td');
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.checked = state.selected.has(invoice.hash);
    checkbox.addEventListener('change', () => {
      if (checkbox.checked) state.selected.add(invoice.hash);
      else state.selected.delete(invoice.hash);
      tr.classList.toggle('selected', checkbox.checked);
    });
    checkCell.appendChild(checkbox);
    tr.appendChild(checkCell);

    const nameCell = document.createElement('td');
    const name = document.createElement('div');
    name.className = 'file-name';
    name.textContent = invoice.fileName;
    name.title = invoice.warnings?.length ? invoice.warnings.join('\n') : invoice.path;
    nameCell.appendChild(name);
    tr.appendChild(nameCell);

    for (const field of FIELDS) {
      const cell = document.createElement('td');
      cell.appendChild(cellInput(invoice, field));
      tr.appendChild(cell);
    }

    const sourceCell = document.createElement('td');
    const tag = document.createElement('span');
    tag.className = invoice.sourceKind === 'link' ? 'tag link' : 'tag';
    tag.textContent = invoice.sourceKind === 'link' ? '链接' : '附件';
    tag.title = invoice.sourceDetail || '';
    sourceCell.appendChild(tag);
    tr.appendChild(sourceCell);

    const actionCell = document.createElement('td');
    const actions = document.createElement('div');
    actions.className = 'row-actions';
    actions.appendChild(
      iconButton('打开 PDF', '▷', () => {
        invoke('open_invoice', { path: invoice.path }).catch((error) =>
          toast(`打开失败：${errorText(error)}`, 'error'),
        );
      }),
    );
    actions.appendChild(
      iconButton('在文件夹中定位', '⌖', () => {
        invoke('reveal_invoice', { path: invoice.path }).catch((error) =>
          toast(`定位失败：${errorText(error)}`, 'error'),
        );
      }),
    );
    actions.appendChild(
      iconButton('删除', '✕', async () => {
        if (!confirm(`确定删除 ${invoice.fileName}？文件会一起删除。`)) return;
        await deleteInvoices([invoice.hash]);
      }),
    );
    actionCell.appendChild(actions);
    tr.appendChild(actionCell);

    body.appendChild(tr);
  }
}

/* ------------------------------------------------------------------ 批量操作 */
function selectedHashes() {
  return [...state.selected];
}

function requireSelection(action) {
  const hashes = selectedHashes();
  if (hashes.length === 0) {
    toast(`请先勾选要${action}的发票`, 'error');
    return null;
  }
  return hashes;
}

async function flushDirty() {
  const entries = [...state.dirty.entries()];
  if (entries.length === 0) return;
  for (const [hash, patch] of entries) {
    const invoice = state.invoices.find((item) => item.hash === hash);
    if (!invoice) continue;
    const payload = {
      hash,
      date: patch.date ?? invoice.date,
      amount: patch.amount ?? invoice.amount,
      seller: patch.seller ?? invoice.seller,
      no: patch.no ?? invoice.no,
    };
    try {
      await invoke('update_fields', payload);
      Object.assign(invoice, payload);
    } catch (error) {
      toast(`保存字段失败：${errorText(error)}`, 'error');
      return;
    }
  }
  state.dirty.clear();
}

async function renameSelected() {
  const hashes = selectedHashes();
  const targets = hashes.length > 0 ? hashes : [...state.dirty.keys()];
  if (targets.length === 0) {
    toast('请先勾选发票，或直接改几个字段再重命名', 'error');
    return;
  }
  const template = $('#template').value.trim() || '{date}_{amount}_{seller}_{no}';
  try {
    await flushDirty();
    const updated = await invoke('rename_invoices', { hashes: targets, template });
    toast(`已重命名 ${updated.length} 个文件`, 'ok');
    await loadInvoices();
  } catch (error) {
    toast(`重命名失败：${errorText(error)}`, 'error');
  }
}

async function exportSelected() {
  const hashes = requireSelection('导出');
  if (!hashes) return;
  try {
    await flushDirty();
    const result = await invoke('export_invoices', { hashes });
    if (!result) return;
    toast(result, 'ok');
  } catch (error) {
    toast(`导出失败：${errorText(error)}`, 'error');
  }
}

async function exportCsv() {
  try {
    const path = await invoke('export_csv');
    toast(`清单已生成：\n${path}`, 'ok');
  } catch (error) {
    toast(`生成清单失败：${errorText(error)}`, 'error');
  }
}

async function deleteInvoices(hashes) {
  try {
    await invoke('delete_invoices', { hashes });
    hashes.forEach((hash) => state.selected.delete(hash));
    toast(`已删除 ${hashes.length} 个`, 'ok');
    await loadInvoices();
  } catch (error) {
    toast(`删除失败：${errorText(error)}`, 'error');
  }
}

/* ------------------------------------------------------------------ 事件绑定 */
function bindEvents() {
  $$('.nav-item').forEach((item) => {
    item.addEventListener('click', () => showView(item.dataset.view));
  });

  $('#btn-sync').addEventListener('click', startSync);
  $('#btn-clear-log').addEventListener('click', () => {
    $('#log').innerHTML = '';
  });

  $('#btn-save').addEventListener('click', () => saveConfig());
  $('#btn-test').addEventListener('click', testConnection);
  $('#btn-eye').addEventListener('click', () => {
    const input = $('#password');
    const hidden = input.type === 'password';
    input.type = hidden ? 'text' : 'password';
    $('#btn-eye').textContent = hidden ? '隐藏' : '显示';
  });

  $$('.chip').forEach((chip) => {
    chip.addEventListener('click', () => {
      $('#imapHost').value = chip.dataset.host;
      $('#imapPort').value = chip.dataset.port;
      $('#imapTls').value = chip.dataset.port === '993' ? 'tls' : 'starttls';
    });
  });

  ['#namingTemplate', '#template'].forEach((selector) => {
    $(selector).addEventListener('input', () => {
      if (selector === '#namingTemplate') updatePreview();
      else updateSummary();
    });
  });
  $('#namingTemplate').addEventListener('change', () => {
    $('#template').value = $('#namingTemplate').value;
  });

  $('#search').addEventListener('input', renderRows);
  $('#check-all').addEventListener('change', (event) => {
    const query = $('#search').value.trim().toLowerCase();
    const visible = state.invoices.filter((invoice) => matchesQuery(invoice, query));
    if (event.target.checked) visible.forEach((invoice) => state.selected.add(invoice.hash));
    else visible.forEach((invoice) => state.selected.delete(invoice.hash));
    renderRows();
  });

  $('#btn-rename').addEventListener('click', renameSelected);
  $('#btn-export').addEventListener('click', exportSelected);
  $('#btn-export-csv').addEventListener('click', exportCsv);
  $('#btn-delete').addEventListener('click', async () => {
    const hashes = requireSelection('删除');
    if (!hashes) return;
    if (!confirm(`确定删除选中的 ${hashes.length} 个发票？文件会一起删除。`)) return;
    await deleteInvoices(hashes);
  });

  $('#btn-open-dir').addEventListener('click', () => {
    invoke('open_data_dir').catch((error) => toast(`打开失败：${errorText(error)}`, 'error'));
  });

  window.addEventListener('keydown', (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
      startSync();
    }
  });
}

/* ------------------------------------------------------------------ 启动 */
(async function init() {
  bindEvents();
  try {
    await listen('sync-log', (event) => {
      logLine(String(event.payload ?? ''));
    });
  } catch (error) {
    console.warn('事件监听失败', error);
  }
  try {
    const config = await invoke('load_config');
    state.config = config;
    fillForm(config);
  } catch (error) {
    toast(`读取配置失败：${errorText(error)}`, 'error');
  }
  updateSummary();
  updatePreview();
  await loadInvoices();
  showView('sync');
})();

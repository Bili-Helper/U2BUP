<script setup lang="ts">
/**
 * LibraryManager.vue
 * Renders two surfaces:
 *  1. Welcome screen (shown when no libraries exist)
 *  2. Library management page (shown at /library/manage)
 * Both surfaces share the same "Add Library" dialog.
 */
import { ref, computed } from 'vue';
import {
  Archive, CheckCircle, ChevronRight, Database, FolderOpen, FolderSync,
  HardDrive, Loader2, Monitor, Network, Plus, RefreshCw, Server,
  Trash2, TriangleAlert, Wifi, X,
} from 'lucide-vue-next';
import type { LibraryRoot, LibraryKind } from './types';

const props = defineProps<{
  libraries: LibraryRoot[];
  busy: boolean;
}>();

const emit = defineEmits<{
  (e: 'add', payload: AddPayload): void;
  (e: 'scan', id: string): void;
  (e: 'delete', id: string): void;
  (e: 'update', id: string, patch: Partial<LibraryRoot>): void;
  (e: 'navigate', path: string): void;
}>();

// ── Add dialog state ────────────────────────────────────────────────────────

const showAdd = ref(false);
const addKind = ref<LibraryKind>('folder');
const addName = ref('');
const addPath = ref('');
const addError = ref('');
const addBusy = ref(false);

interface AddPayload {
  name: string;
  kind: LibraryKind;
  path: string;
}

const kindOptions: { value: LibraryKind; label: string; desc: string; icon: any }[] = [
  { value: 'liverec', label: 'BililiveRecorder 录播库', desc: '{room_id}-{主播名}/ 目录结构，含 XML 元数据', icon: Archive },
  { value: 'folder',  label: '普通文件夹',              desc: '任意目录，扫描所有视频文件（mp4/mkv/ts 等）', icon: FolderOpen },
  { value: 'webdav',  label: 'WebDAV 挂载路径',         desc: '系统已挂载的 WebDAV 目录，只读模式', icon: Network },
  { value: 'openlist', label: 'OpenList / Alist 挂载', desc: '本地挂载点，只读模式', icon: Server },
];

function openAdd(kind: LibraryKind = 'folder') {
  addKind.value = kind;
  addName.value = '';
  addPath.value = '';
  addError.value = '';
  showAdd.value = true;
}

function closeAdd() {
  showAdd.value = false;
  addError.value = '';
}

async function submitAdd() {
  addError.value = '';
  if (!addName.value.trim()) { addError.value = '请填写素材库名称'; return; }
  if (!addPath.value.trim()) { addError.value = '请填写目录路径'; return; }
  addBusy.value = true;
  try {
    emit('add', { name: addName.value.trim(), kind: addKind.value, path: addPath.value.trim() });
    closeAdd();
  } finally {
    addBusy.value = false;
  }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

const kindLabel: Record<LibraryKind, string> = {
  liverec: '录播库',
  folder: '文件夹',
  webdav: 'WebDAV',
  openlist: 'OpenList',
};

const kindIcon: Record<LibraryKind, any> = {
  liverec: Archive,
  folder: FolderOpen,
  webdav: Wifi,
  openlist: Server,
};

function scanStatusClass(lib: LibraryRoot) {
  return {
    scanning: lib.scanStatus === 'scanning',
    error: lib.scanStatus === 'error',
    offline: lib.scanStatus === 'offline',
  };
}

function scanStatusLabel(lib: LibraryRoot) {
  if (lib.scanStatus === 'scanning') return '扫描中…';
  if (lib.scanStatus === 'error') return '扫描失败';
  if (lib.scanStatus === 'offline') return '路径离线';
  if (lib.lastScannedAt) return `${lib.assetCount} 个文件`;
  return '未扫描';
}

function formatPath(path: string) {
  return path.length > 60 ? '…' + path.slice(-57) : path;
}
</script>

<template>
  <!-- ── Welcome screen (no libraries) ──────────────────────────────────── -->
  <template v-if="libraries.length === 0">
    <div class="welcome-page">
      <div class="welcome-hero">
        <div class="welcome-mark"><HardDrive :size="36" /></div>
        <div class="eyebrow">开始使用 U2BUP</div>
        <h1>添加第一个素材库<span class="heading-dot">.</span></h1>
        <p>选择一个本地文件夹、录播目录或远程挂载路径，U2BUP 会扫描并索引其中的视频文件，不会复制源文件。</p>
      </div>

      <div class="welcome-cards">
        <button class="welcome-card" @click="openAdd('liverec')">
          <div class="welcome-card-icon"><Archive :size="26" /></div>
          <div>
            <strong>BililiveRecorder 录播库</strong>
            <p>导入 B 站直播录播目录，自动读取场次、主播与 XML 元数据。</p>
          </div>
          <ChevronRight :size="18" class="welcome-card-arrow" />
        </button>
        <button class="welcome-card" @click="openAdd('folder')">
          <div class="welcome-card-icon"><FolderOpen :size="26" /></div>
          <div>
            <strong>普通文件夹</strong>
            <p>扫描任意目录中的 mp4 / mkv / ts 等视频文件，适合零散素材库。</p>
          </div>
          <ChevronRight :size="18" class="welcome-card-arrow" />
        </button>
        <button class="welcome-card" @click="openAdd('webdav')">
          <div class="welcome-card-icon"><Wifi :size="26" /></div>
          <div>
            <strong>WebDAV / OpenList 挂载</strong>
            <p>已在系统里挂载的远程目录，以只读方式索引，元数据保存在本地。</p>
          </div>
          <ChevronRight :size="18" class="welcome-card-arrow" />
        </button>
      </div>

      <div class="welcome-note">
        <Monitor :size="14" />
        所有素材元数据均保存在本地数据库，源文件不会被复制或修改。
      </div>
    </div>
  </template>

  <!-- ── Library management page ────────────────────────────────────────── -->
  <template v-else>
    <div class="page-heading">
      <div>
        <div class="eyebrow">MEDIA LIBRARIES</div>
        <h1>素材库管理<span class="heading-dot">.</span></h1>
        <p>添加、扫描或移除素材来源目录。</p>
      </div>
      <button class="primary" @click="openAdd()">
        <Plus :size="16" />添加素材库
      </button>
    </div>

    <div class="lib-list">
      <div v-for="lib in libraries" :key="lib.id" class="lib-card">
        <div class="lib-card-header">
          <div class="lib-kind-badge">
            <component :is="kindIcon[lib.kind]" :size="14" />
            {{ kindLabel[lib.kind] }}
          </div>
          <div class="lib-actions">
            <button
              class="subtle"
              :disabled="lib.scanStatus === 'scanning' || lib.kind === 'liverec'"
              :title="lib.kind === 'liverec' ? '录播库扫描请使用「重新扫描素材」按钮' : '触发扫描'"
              @click="emit('scan', lib.id)"
            >
              <RefreshCw :size="14" :class="{ spin: lib.scanStatus === 'scanning' }" />
              {{ lib.scanStatus === 'scanning' ? '扫描中…' : '扫描' }}
            </button>
            <button
              class="lib-delete-btn icon-button"
              :title="'删除素材库：' + lib.name"
              @click="emit('delete', lib.id)"
            >
              <Trash2 :size="15" />
            </button>
          </div>
        </div>

        <div class="lib-card-body">
          <div class="lib-name">{{ lib.name }}</div>
          <div class="lib-path" :title="lib.path">{{ formatPath(lib.path) }}</div>
        </div>

        <div class="lib-card-footer">
          <span class="lib-status" :class="scanStatusClass(lib)">
            <span class="dot" :class="scanStatusClass(lib)"></span>
            {{ scanStatusLabel(lib) }}
          </span>
          <span v-if="lib.lastScannedAt" class="lib-scanned-at">
            最近扫描：{{ new Date(lib.lastScannedAt).toLocaleString('zh-CN', { timeZone: lib.displayTz ?? 'Asia/Shanghai' }) }}
          </span>
          <span v-if="lib.scanStatus === 'error' && lib.scanError" class="lib-error-msg">
            <TriangleAlert :size="12" />{{ lib.scanError }}
          </span>
          <span v-if="lib.readonly" class="lib-readonly-badge">只读</span>
        </div>
      </div>
    </div>

    <div class="lib-add-row">
      <button class="subtle" @click="openAdd()">
        <Plus :size="15" />添加素材库…
      </button>
    </div>
  </template>

  <!-- ── Add library dialog ─────────────────────────────────────────────── -->
  <Teleport to="body">
    <div v-if="showAdd" class="modal-backdrop" @click.self="closeAdd">
      <div class="modal" role="dialog" aria-modal="true" aria-labelledby="add-lib-title">
        <div class="modal-header">
          <h2 id="add-lib-title">添加素材库</h2>
          <button class="icon-button" aria-label="关闭" @click="closeAdd"><X :size="18" /></button>
        </div>

        <div class="modal-body">
          <!-- Kind selector -->
          <div class="field-group">
            <label class="field-label">素材库类型</label>
            <div class="kind-grid">
              <button
                v-for="opt in kindOptions"
                :key="opt.value"
                class="kind-option"
                :class="{ selected: addKind === opt.value }"
                @click="addKind = opt.value"
              >
                <component :is="opt.icon" :size="18" />
                <div>
                  <strong>{{ opt.label }}</strong>
                  <small>{{ opt.desc }}</small>
                </div>
                <CheckCircle v-if="addKind === opt.value" :size="16" class="kind-check" />
              </button>
            </div>
          </div>

          <!-- Name -->
          <div class="field-group">
            <label class="field-label" for="lib-name">显示名称</label>
            <input
              id="lib-name"
              v-model="addName"
              type="text"
              placeholder="例：B站录播、相机素材、NAS 存档"
              maxlength="80"
              @keydown.enter="submitAdd"
            />
          </div>

          <!-- Path -->
          <div class="field-group">
            <label class="field-label" for="lib-path">
              目录路径
              <span class="field-hint">
                {{ addKind === 'webdav' || addKind === 'openlist' ? '（请先在系统里完成挂载）' : '' }}
              </span>
            </label>
            <input
              id="lib-path"
              v-model="addPath"
              type="text"
              :placeholder="addKind === 'liverec' ? 'C:\\Users\\...\\LiveRec' : addKind === 'folder' ? 'D:\\Videos\\素材' : '/mnt/webdav'"
              @keydown.enter="submitAdd"
            />
          </div>

          <div v-if="(addKind === 'webdav' || addKind === 'openlist') " class="modal-info-note">
            <Wifi :size="14" />
            WebDAV / OpenList 素材库以只读模式索引，元数据保存在本地数据库，不写入远程目录。
          </div>

          <div v-if="addError" class="modal-error">
            <TriangleAlert :size="14" />{{ addError }}
          </div>
        </div>

        <div class="modal-footer">
          <button class="subtle" @click="closeAdd">取消</button>
          <button class="primary" :disabled="addBusy || busy" @click="submitAdd">
            <Loader2 v-if="addBusy" :size="15" class="spin" />
            <FolderSync v-else :size="15" />
            添加并扫描
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
/* Welcome screen */
.welcome-page { max-width: 680px; margin: 60px auto; text-align: center; }
.welcome-hero { margin-bottom: 48px; }
.welcome-mark {
  width: 72px; height: 72px; border-radius: 18px;
  background: var(--accent-soft); color: var(--accent);
  display: grid; place-items: center; margin: 0 auto 28px;
}
.welcome-hero .eyebrow { margin-bottom: 14px; }
.welcome-hero h1 { font-size: 32px; margin-bottom: 14px; }
.welcome-hero p { font-size: 13px; color: var(--text-soft); max-width: 480px; margin: 0 auto; }

.welcome-cards { display: flex; flex-direction: column; gap: 12px; text-align: left; margin-bottom: 32px; }
.welcome-card {
  display: flex; align-items: center; gap: 18px;
  background: var(--surface); border: 1px solid var(--border);
  border-radius: 10px; padding: 20px 22px; cursor: pointer;
  transition: border-color 0.15s, box-shadow 0.15s;
}
.welcome-card:hover { border-color: var(--accent-border); box-shadow: 0 2px 8px var(--shadow); }
.welcome-card-icon {
  flex-shrink: 0; width: 48px; height: 48px; border-radius: 11px;
  background: var(--accent-soft); color: var(--accent);
  display: grid; place-items: center;
}
.welcome-card > div strong { font-size: 14px; display: block; margin-bottom: 5px; }
.welcome-card > div p { font-size: 11px; color: var(--text-soft); margin: 0; line-height: 1.6; }
.welcome-card-arrow { color: var(--text-soft); margin-left: auto; flex-shrink: 0; }
.welcome-note {
  display: flex; align-items: center; justify-content: center; gap: 8px;
  font-size: 10px; color: var(--text-soft);
  border: 1px solid var(--border); border-radius: 7px; padding: 10px 16px;
}

/* Library list */
.lib-list { display: flex; flex-direction: column; gap: 12px; margin-bottom: 20px; }
.lib-card {
  background: var(--surface); border: 1px solid var(--line);
  border-radius: 9px; overflow: hidden;
}
.lib-card-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 13px 18px; border-bottom: 1px solid var(--line);
  background: var(--surface-soft);
}
.lib-kind-badge {
  display: flex; align-items: center; gap: 7px;
  font-size: 10px; color: var(--text-soft); font-weight: 500;
}
.lib-actions { display: flex; align-items: center; gap: 8px; }
.lib-delete-btn { color: var(--text-soft); }
.lib-delete-btn:hover { color: #ef4444; background: #fee2e2; }
.lib-card-body { padding: 15px 18px 12px; }
.lib-name { font-weight: 600; font-size: 14px; margin-bottom: 5px; }
.lib-path { font-size: 11px; color: var(--text-soft); font-family: monospace; }
.lib-card-footer {
  display: flex; align-items: center; flex-wrap: wrap; gap: 12px;
  padding: 10px 18px; border-top: 1px solid var(--line);
  font-size: 10px; color: var(--text-soft);
}
.lib-status { display: flex; align-items: center; gap: 5px; }
.lib-status .dot { flex-shrink: 0; }
.lib-status.scanning .dot { background: var(--accent); animation: pulse 1s infinite; }
.lib-status.error .dot { background: #ef4444; }
.lib-status.offline .dot { background: #6b7280; }
.lib-error-msg { display: flex; align-items: center; gap: 4px; color: #ef4444; }
.lib-readonly-badge {
  margin-left: auto; font-size: 9px; letter-spacing: 0.5px;
  border: 1px solid var(--border); border-radius: 3px; padding: 1px 5px;
}
.lib-add-row { margin-top: 8px; }

@keyframes pulse { 0%,100%{opacity:1} 50%{opacity:.4} }

/* Modal */
.modal-backdrop {
  position: fixed; inset: 0; background: rgba(0,0,0,.45);
  display: flex; align-items: center; justify-content: center;
  z-index: 100;
}
.modal {
  background: var(--surface); border-radius: 12px;
  width: min(560px, 95vw); max-height: 90vh;
  overflow-y: auto; box-shadow: 0 20px 60px rgba(0,0,0,.25);
  border: 1px solid var(--border);
}
.modal-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 20px 24px 16px; border-bottom: 1px solid var(--border);
}
.modal-header h2 { font-size: 16px; }
.modal-body { padding: 22px 24px; display: flex; flex-direction: column; gap: 20px; }
.modal-footer {
  display: flex; justify-content: flex-end; gap: 10px;
  padding: 16px 24px; border-top: 1px solid var(--border);
}

.field-group { display: flex; flex-direction: column; gap: 8px; }
.field-label { font-size: 12px; font-weight: 550; }
.field-hint { font-weight: 400; color: var(--text-soft); }

.kind-grid { display: flex; flex-direction: column; gap: 8px; }
.kind-option {
  display: flex; align-items: flex-start; gap: 12px;
  border: 1px solid var(--border); border-radius: 8px; padding: 12px 14px;
  text-align: left; cursor: pointer; position: relative;
  transition: border-color 0.15s;
}
.kind-option:hover { border-color: var(--accent-border); }
.kind-option.selected { border-color: var(--accent); background: var(--accent-soft); }
.kind-option > div strong { font-size: 12px; display: block; margin-bottom: 3px; }
.kind-option > div small { font-size: 10px; color: var(--text-soft); line-height: 1.5; }
.kind-check { margin-left: auto; color: var(--accent); flex-shrink: 0; }

.modal-info-note {
  display: flex; align-items: flex-start; gap: 8px;
  font-size: 11px; color: var(--text-soft); line-height: 1.6;
  background: var(--surface-soft); border-radius: 7px; padding: 10px 12px;
}
.modal-error {
  display: flex; align-items: center; gap: 7px;
  font-size: 12px; color: #ef4444;
  background: #fee2e2; border-radius: 7px; padding: 10px 12px;
}
</style>

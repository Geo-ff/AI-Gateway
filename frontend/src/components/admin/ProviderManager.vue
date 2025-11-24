<script setup lang="ts">
import { onMounted, onBeforeUnmount, reactive, ref, computed } from 'vue'
import {
  listProviders,
  createProvider,
  updateProvider,
  deleteProvider,
  listProviderKeysRaw,
  addProviderKey,
  deleteProviderKey,
  addProviderKeysBatch,
  deleteProviderKeysBatch,
  type ProviderKeyEntry,
  type Provider,
} from '../../api/providers'
import {
  listProviderModels,
  listCachedModels,
  refreshProviderCache,
  clearProviderCache,
  type ModelInfo,
  type CachedModel,
} from '../../api/models'

interface ProviderForm {
  name: string
  api_type: Provider['api_type']
  base_url: string
  models_endpoint?: string
}

import { useNotify } from '../../composables/useNotify'

const providers = ref<Provider[]>([])
const loadingProviders = ref(false)
const providerError = ref<string | null>(null)
const selectedProvider = ref<Provider | null>(null)
const keys = ref<ProviderKeyEntry[]>([])
const keysTotal = ref(0)
const keysLoading = ref(false)
const keysError = ref<string | null>(null)
const keySearch = ref('')
const selectedKeys = reactive(new Set<string>())
const visibleKeys = reactive(new Set<string>())
const singleKey = ref('')
const batchAddText = ref('')
const message = ref<string | null>(null)
const cachedModels = ref<CachedModel[]>([])
const providerModels = ref<ModelInfo[]>([])
const modelsError = ref<string | null>(null)
const selectedModelIds = reactive(new Set<string>())
const selectedRemoveIds = reactive(new Set<string>())
const addingModels = ref(false)
const messageTimer = ref<number | null>(null)
const removingModelIds = reactive(new Set<string>())

const providerForm = reactive<ProviderForm>({
  name: '',
  api_type: 'openai',
  base_url: '',
  models_endpoint: '',
})

const providerTypeOptions = [
  { value: 'openai', label: 'OpenAI' },
  { value: 'anthropic', label: 'Anthropic' },
  { value: 'zhipu', label: 'Zhipu' },
] as const

const providerTypeLabel = (type: Provider['api_type']) => {
  const option = providerTypeOptions.find((item) => item.value === type)
  return option ? option.label : type
}

const cachedModelIds = computed(() => new Set(cachedModels.value.map((model) => model.id)))
const cachedModelMetadata = computed(() => {
  const map = new Map<string, CachedModel>()
  cachedModels.value.forEach((model) => {
    map.set(model.id, model)
  })
  return map
})

const isEditing = computed(() => !!selectedProvider.value)

async function loadProviders() {
  loadingProviders.value = true
  providerError.value = null
  try {
    providers.value = await listProviders()
    if (providers.value.length > 0) {
      const match = selectedProvider.value
        ? providers.value.find((p) => p.name === selectedProvider.value?.name)
        : providers.value[0]
      if (match) {
        selectProvider(match)
      }
    } else {
      selectedProvider.value = null
      keys.value = []
      keysTotal.value = 0
    }
  } catch (err: any) {
    providerError.value = err?.message || String(err)
  } finally {
    loadingProviders.value = false
  }
}

function resetProviderForm() {
  providerForm.name = ''
  providerForm.api_type = 'openai'
  providerForm.base_url = ''
  providerForm.models_endpoint = ''
}

function startCreateProvider() {
  selectedProvider.value = null
  resetProviderForm()
}

async function submitProvider() {
  message.value = null
  try {
    if (!providerForm.name.trim() || !providerForm.base_url.trim()) {
      showMessage('名称和 Base URL 不能为空', 'warning')
      return
    }
    if (isEditing.value && selectedProvider.value) {
      const updated = await updateProvider(selectedProvider.value.name, {
        api_type: providerForm.api_type,
        base_url: providerForm.base_url.trim(),
        models_endpoint: providerForm.models_endpoint?.trim() || null,
      })
      const idx = providers.value.findIndex((p) => p.name === updated.name)
      if (idx >= 0) {
        providers.value.splice(idx, 1, updated)
        // 保持选中状态
        selectProvider(updated)
      }
      showMessage('供应商已更新', 'success')
    } else {
      const created = await createProvider({
        name: providerForm.name.trim(),
        api_type: providerForm.api_type,
        base_url: providerForm.base_url.trim(),
        models_endpoint: providerForm.models_endpoint?.trim() || null,
      })
      providers.value.push(created)
      selectProvider(created)
      showMessage('供应商已创建', 'success')
      resetProviderForm()
    }
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  }
}

const { showMessage } = useNotify()

async function removeProvider(provider: Provider) {
  providerToDelete.value = provider
  showProviderDeleteConfirm.value = true
}

async function confirmDeleteProvider() {
  if (!providerToDelete.value) return
  try {
    await deleteProvider(providerToDelete.value.name)
    providers.value = providers.value.filter((p) => p.name !== providerToDelete.value!.name)
    if (selectedProvider.value?.name === providerToDelete.value.name) {
      selectedProvider.value = null
      keys.value = []
      keysTotal.value = 0
    }
    showMessage('供应商已删除', 'success')
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  } finally {
    showProviderDeleteConfirm.value = false
    providerToDelete.value = null
  }
}

function cancelDeleteProvider() {
  showProviderDeleteConfirm.value = false
  providerToDelete.value = null
}

function selectProvider(provider: Provider) {
  selectedProvider.value = provider
  providerForm.name = provider.name
  providerForm.api_type = provider.api_type
  providerForm.base_url = provider.base_url
  providerForm.models_endpoint = provider.models_endpoint || ''
  selectedModelIds.clear()
  modelsError.value = null
  loadKeys()
}

async function loadKeys() {
  if (!selectedProvider.value) return
  keysLoading.value = true
  keysError.value = null
  selectedKeys.clear()
   visibleKeys.clear()
  try {
    const res = await listProviderKeysRaw(selectedProvider.value.name, {
      q: keySearch.value.trim() || undefined,
    })
    keys.value = res.keys
    keysTotal.value = res.total
  } catch (err: any) {
    keysError.value = err?.message || String(err)
    keys.value = []
    keysTotal.value = 0
  } finally {
    keysLoading.value = false
  }
}

async function handleAddKey() {
  if (!selectedProvider.value) return
  const key = singleKey.value.trim()
  if (!key) {
    showMessage('请输入要添加的 Key', 'warning')
    return
  }
  message.value = null
  try {
    await addProviderKey(selectedProvider.value.name, key)
    singleKey.value = ''
    showMessage('Key 已添加', 'success')
    await loadKeys()
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  }
}

const showDeleteConfirm = ref(false)
const keyToDelete = ref('')
const showProviderDeleteConfirm = ref(false)
const providerToDelete = ref<Provider | null>(null)
const showBatchDeleteConfirm = ref(false)
const batchDeleteCount = ref(0)
const showModelsModal = ref(false)
const loadingModels = ref(false)

async function handleDeleteKey(keyValue: string) {
  keyToDelete.value = keyValue
  showDeleteConfirm.value = true
}

async function confirmDeleteKey() {
  if (!selectedProvider.value || !keyToDelete.value) return
  message.value = null
  try {
    await deleteProviderKey(selectedProvider.value.name, keyToDelete.value)
    showMessage('Key 已删除', 'success')
    await loadKeys()
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  } finally {
    showDeleteConfirm.value = false
    keyToDelete.value = ''
  }
}

function cancelDeleteKey() {
  showDeleteConfirm.value = false
  keyToDelete.value = ''
}

async function handleBatchAdd() {
  if (!selectedProvider.value) return
  const entries = batchAddText.value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
  if (!entries.length) {
    showMessage('请粘贴至少一条 Key', 'warning')
    return
  }
  message.value = null
  try {
    const result = await addProviderKeysBatch(selectedProvider.value.name, entries)
    const errors = result.results.filter((r) => r.status === 'error')
    batchAddText.value = ''
    showMessage(
      errors.length
        ? `成功 ${result.success} 条，失败 ${errors.length} 条`
        : `成功添加 ${result.success} 条 Key`,
      errors.length ? 'warning' : 'success',
    )
    await loadKeys()
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  }
}

async function handleBatchDelete() {
  if (!selectedProvider.value) return
  const targets = Array.from(selectedKeys)
  if (!targets.length) {
    showMessage('请选择要删除的 Key', 'warning')
    return
  }
  batchDeleteCount.value = targets.length
  showBatchDeleteConfirm.value = true
}

async function confirmBatchDelete() {
  if (!selectedProvider.value) return
  const targets = Array.from(selectedKeys)
  try {
    const result = await deleteProviderKeysBatch(selectedProvider.value.name, targets)
    const errors = result.results.filter((r) => r.status === 'error')
    selectedKeys.clear()
    showMessage(
      errors.length
        ? `已删除 ${result.removed} 条，失败 ${errors.length} 条`
        : `已删除 ${result.removed} 条 Key`,
      errors.length ? 'warning' : 'success',
    )
    await loadKeys()
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  } finally {
    showBatchDeleteConfirm.value = false
    batchDeleteCount.value = 0
  }
}

function cancelBatchDelete() {
  showBatchDeleteConfirm.value = false
  batchDeleteCount.value = 0
}

function toggleKeySelection(key: string, checked: boolean) {
  if (checked) {
    selectedKeys.add(key)
  } else {
    selectedKeys.delete(key)
  }
}

async function openModelsModal() {
  if (!selectedProvider.value) return
  showModelsModal.value = true
  selectedModelIds.clear()
  await loadProviderModels({ refresh: true })
}

async function loadProviderModels(options?: { refresh?: boolean }) {
  if (!selectedProvider.value) return
  loadingModels.value = true
  modelsError.value = null
  if (!options?.refresh) {
    selectedModelIds.clear()
  }
  try {
    const [upstream, cached] = await Promise.all([
      listProviderModels(selectedProvider.value.name, { refresh: options?.refresh ?? true }),
      listCachedModels(selectedProvider.value.name),
    ])
    providerModels.value = upstream
    cachedModels.value = cached.data
  } catch (err: any) {
    modelsError.value = err?.message || String(err)
    providerModels.value = []
    cachedModels.value = []
  } finally {
    loadingModels.value = false
  }
}

function closeModelsModal() {
  showModelsModal.value = false
  providerModels.value = []
  cachedModels.value = []
  modelsError.value = null
  selectedModelIds.clear()
  selectedRemoveIds.clear()
}

function toggleModelSelection(id: string, checked: boolean) {
  if (checked) {
    selectedModelIds.add(id)
  } else {
    selectedModelIds.delete(id)
  }
}

async function addModels(ids?: string[]) {
  if (!selectedProvider.value) return
  const targets = ids ?? Array.from(selectedModelIds)
  if (!targets.length) {
    showMessage('请选择至少一个模型', 'warning')
    return
  }
  addingModels.value = true
  try {
    await refreshProviderCache(selectedProvider.value.name, {
      mode: 'selected',
      include: targets,
      replace: false,
    })
    showMessage(`已添加 ${targets.length} 个模型`, 'success')
    selectedModelIds.clear()
    // 仅刷新缓存状态，避免整表闪烁
    try {
      const cached = await listCachedModels(selectedProvider.value.name)
      cachedModels.value = cached.data
    } catch (err: any) {
      modelsError.value = err?.message || String(err)
    }
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  } finally {
    addingModels.value = false
  }
}

async function removeCachedModel(id: string) {
  if (!selectedProvider.value) return
  if (removingModelIds.has(id)) return
  removingModelIds.add(id)
  try {
    await clearProviderCache(selectedProvider.value.name, [id])
    showMessage('已移除模型缓存', 'success')
    try {
      const cached = await listCachedModels(selectedProvider.value.name)
      cachedModels.value = cached.data
    } catch (err: any) {
      modelsError.value = err?.message || String(err)
    }
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  } finally {
    removingModelIds.delete(id)
  }
}

function toggleRemoveSelection(id: string, checked: boolean) {
  if (checked) {
    selectedRemoveIds.add(id)
  } else {
    selectedRemoveIds.delete(id)
  }
}

async function removeSelectedCached() {
  if (!selectedProvider.value) return
  const ids = Array.from(selectedRemoveIds)
  if (!ids.length) {
    showMessage('请选择要移除的缓存模型', 'warning')
    return
  }
  try {
    await clearProviderCache(selectedProvider.value.name, ids)
    showMessage(`已移除 ${ids.length} 个缓存模型`, 'success')
    selectedRemoveIds.clear()
    try {
      const cached = await listCachedModels(selectedProvider.value.name)
      cachedModels.value = cached.data
    } catch (err: any) {
      modelsError.value = err?.message || String(err)
    }
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  }
}

onMounted(() => {
  loadProviders()
})

onBeforeUnmount(() => {
  if (messageTimer.value) {
    window.clearTimeout(messageTimer.value)
  }
})
</script>

<template>
  <div class="provider-manager">
    <div class="columns">
      <aside class="providers">
        <header>
          <h3>供应商</h3>
          <button @click="loadProviders" :disabled="loadingProviders">刷新</button>
        </header>
        <p v-if="providerError" class="error">{{ providerError }}</p>
        <ul v-if="providers.length">
          <li
            v-for="p in providers"
            :key="p.name"
            :class="{ active: selectedProvider?.name === p.name }"
            @click="selectProvider(p)"
          >
            <div class="provider-item">
              <span>{{ p.name }}</span>
              <small>{{ providerTypeLabel(p.api_type) }}</small>
            </div>
          </li>
        </ul>
        <p v-else-if="!loadingProviders">暂无供应商</p>
      </aside>

      <section class="details">
        <header class="details-header">
          <h3>{{ isEditing ? '编辑供应商' : '创建供应商' }}</h3>
          <div class="header-actions">
            <button @click="startCreateProvider">
              新建供应商
            </button>
            <button v-if="selectedProvider" @click="openModelsModal" class="models-btn">
              获取模型
            </button>
            <button v-if="selectedProvider" class="danger" @click="removeProvider(selectedProvider)">
              删除当前供应商
            </button>
          </div>
        </header>

        <form class="provider-form" @submit.prevent="submitProvider">
          <label>
            名称
            <input v-model="providerForm.name" :disabled="isEditing" required />
          </label>
          <label>
            类型
            <select v-model="providerForm.api_type">
              <option
                v-for="option in providerTypeOptions"
                :key="option.value"
                :value="option.value"
              >
                {{ option.label }}
              </option>
            </select>
          </label>
          <label>
            Base URL
            <input v-model="providerForm.base_url" required />
          </label>
          <label>
            Models Endpoint
            <input v-model="providerForm.models_endpoint" placeholder="可选" />
          </label>
          <div class="form-actions">
            <button type="submit">{{ isEditing ? '保存修改' : '创建' }}</button>
            <button type="button" @click="resetProviderForm">重置</button>
          </div>
        </form>

        <section v-if="selectedProvider" class="keys">
          <header>
            <div>
              <h4>API Keys</h4>
              <p class="meta">共 {{ keysTotal }} 条{{ selectedKeys.size > 0 ? `，已选中 ${selectedKeys.size} 条` : '' }}</p>
            </div>
            <div class="key-actions">
              <input
                v-model="keySearch"
                placeholder="搜索 Key"
                @keyup.enter="loadKeys"
              />
              <button @click="loadKeys">搜索</button>
              <button
                v-if="keys.length > 0"
                @click="
                  selectedKeys.size === keys.length
                    ? selectedKeys.clear()
                    : keys.forEach((k) => selectedKeys.add(k.value))
                "
              >
                {{ selectedKeys.size === keys.length ? '取消全选' : '全选' }}
              </button>
            </div>
          </header>
          <p v-if="keysError" class="error">{{ keysError }}</p>
          <div v-if="keysLoading">正在加载 Key ...</div>
          <div v-else class="keys-container">
            <div v-if="!keys.length" class="empty-keys">暂无 API Key</div>
            <div v-else class="key-list-container">
              <div class="key-list-header">
                <span>API Key</span>
                <span>操作</span>
              </div>
              <div class="key-list-scrollable">
                <div
                  v-for="key in keys"
                  :key="key.value"
                  :class="['key-item', { selected: selectedKeys.has(key.value) }]"
                  @click="toggleKeySelection(key.value, !selectedKeys.has(key.value))"
                >
                  <label class="key-checkbox" @click.stop>
                    <input
                      type="checkbox"
                      :value="key.value"
                      :checked="selectedKeys.has(key.value)"
                      @change="toggleKeySelection(key.value, ($event.target as HTMLInputElement).checked)"
                    />
                    <span class="key-text">
                      {{ visibleKeys.has(key.value) ? key.value : (key.masked || '********') }}
                    </span>
                  </label>
                  <button
                    class="toggle-key-btn"
                    @click.stop="
                      visibleKeys.has(key.value)
                        ? visibleKeys.delete(key.value)
                        : visibleKeys.add(key.value)
                    "
                  >
                    {{ visibleKeys.has(key.value) ? '隐藏' : '显示' }}
                  </button>
                  <button class="delete-key-btn" @click.stop="handleDeleteKey(key.value)">删除</button>
                </div>
              </div>
            </div>
          </div>

          <div class="batch-operations">
            <div class="batch-delete-section">
              <h5>批量删除</h5>
              <div class="batch-controls">
                <button
                  @click="handleBatchDelete"
                  :disabled="!selectedKeys.size"
                  class="danger"
                >
                  删除选中的 {{ selectedKeys.size }} 个 Key
                </button>
              </div>
            </div>

            <div class="batch-add-section">
              <h5>添加 Key</h5>
              <div class="add-controls">
                <div class="single-add">
                  <input v-model="singleKey" placeholder="输入单个 API Key" />
                  <button @click="handleAddKey" :disabled="!singleKey.trim()">添加</button>
                </div>
                <div class="batch-add">
                  <textarea
                    v-model="batchAddText"
                    placeholder="批量添加：每行一个 Key"
                    rows="4"
                  ></textarea>
                  <button @click="handleBatchAdd" :disabled="!batchAddText.trim()">批量添加</button>
                </div>
              </div>
            </div>
          </div>
        </section>

        <section v-else class="placeholder">
          <p>选择左侧供应商以查看详情，或输入信息创建新的供应商。</p>
        </section>
      </section>
    </div>

    <!-- 删除Key确认弹窗 -->
    <div v-if="showDeleteConfirm" class="modal-overlay" @click="cancelDeleteKey">
      <div class="modal-content" @click.stop>
        <header class="modal-header">
          <h4>确认删除Key</h4>
          <button @click="cancelDeleteKey" class="close-btn">✕</button>
        </header>
        <div class="modal-body">
          <p>确定要删除这个 API Key 吗？</p>
          <div class="key-preview">
            <code>{{ keyToDelete }}</code>
          </div>
          <div class="modal-actions">
            <button @click="confirmDeleteKey" class="danger-btn">删除</button>
            <button @click="cancelDeleteKey" class="cancel-btn">取消</button>
          </div>
        </div>
      </div>
    </div>

    <!-- 删除供应商确认弹窗 -->
    <div v-if="showProviderDeleteConfirm" class="modal-overlay" @click="cancelDeleteProvider">
      <div class="modal-content" @click.stop>
        <header class="modal-header">
          <h4>确认删除供应商</h4>
          <button @click="cancelDeleteProvider" class="close-btn">✕</button>
        </header>
        <div class="modal-body">
          <p>确定要删除供应商 <strong>{{ providerToDelete?.name }}</strong> 吗？</p>
          <p class="warning-text">删除供应商将同时删除其所有相关的API Key和配置信息，此操作不可恢复。</p>
          <div class="modal-actions">
            <button @click="confirmDeleteProvider" class="danger-btn">删除</button>
            <button @click="cancelDeleteProvider" class="cancel-btn">取消</button>
          </div>
        </div>
      </div>
    </div>

    <!-- 批量删除确认弹窗 -->
    <div v-if="showBatchDeleteConfirm" class="modal-overlay" @click="cancelBatchDelete">
      <div class="modal-content" @click.stop>
        <header class="modal-header">
          <h4>确认批量删除</h4>
          <button @click="cancelBatchDelete" class="close-btn">✕</button>
        </header>
        <div class="modal-body">
          <p>确定要删除选中的 <strong>{{ batchDeleteCount }}</strong> 个 API Key 吗？</p>
          <p class="warning-text">此操作不可恢复。</p>
          <div class="modal-actions">
            <button @click="confirmBatchDelete" class="danger-btn">删除</button>
            <button @click="cancelBatchDelete" class="cancel-btn">取消</button>
          </div>
        </div>
      </div>
    </div>

    <!-- 模型获取弹窗 -->
    <div v-if="showModelsModal" class="modal-overlay" @click="closeModelsModal">
      <div class="modal-content models-modal" @click.stop>
        <header class="modal-header">
          <h4>{{ selectedProvider?.name }} 的模型列表</h4>
          <button @click="closeModelsModal" class="close-btn">✕</button>
        </header>
        <div class="modal-body">
          <div v-if="loadingModels" class="loading-models">正在获取模型列表...</div>
          <div v-else>
            <p v-if="modelsError" class="error">{{ modelsError }}</p>
            <div v-else>
              <div class="models-toolbar">
                <div class="selection-info">
                  已选择 <strong>{{ selectedModelIds.size }}</strong> 个待添加模型
                  <span class="divider">|</span>
                  待移除 <strong>{{ selectedRemoveIds.size }}</strong> 个缓存模型
                </div>
                <div class="toolbar-actions">
                  <button
                    class="refresh-btn"
                    @click="loadProviderModels({ refresh: true })"
                    :disabled="loadingModels"
                  >
                    刷新上游
                  </button>
                  <button
                    class="primary-btn"
                    @click="addModels()"
                    :disabled="addingModels || !selectedModelIds.size"
                  >
                    {{ addingModels ? '添加中...' : '批量添加' }}
                  </button>
                  <button
                    class="danger-btn"
                    @click="removeSelectedCached"
                    :disabled="!selectedRemoveIds.size"
                  >
                    批量移除缓存
                  </button>
                </div>
              </div>
              <div v-if="!providerModels.length" class="no-models">
                未找到任何模型
              </div>
              <div v-else class="models-table-wrapper">
                <table class="models-table">
                  <thead>
                    <tr>
                      <th>选择</th>
                      <th>模型 ID</th>
                      <th>状态</th>
                      <th>缓存时间</th>
                      <th>操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr
                      v-for="model in providerModels"
                      :key="model.id"
                      :class="{ cached: cachedModelIds.has(model.id) }"
                    >
                      <td class="select-cell">
                        <input
                          type="checkbox"
                          :disabled="cachedModelIds.has(model.id)"
                          :checked="selectedModelIds.has(model.id)"
                          @change="toggleModelSelection(model.id, ($event.target as HTMLInputElement).checked)"
                        />
                      </td>
                      <td class="model-id-cell">
                        <div class="model-id-text">{{ model.id }}</div>
                      </td>
                      <td class="status-cell">
                        <span :class="['status-pill', cachedModelIds.has(model.id) ? 'cached' : 'pending']">
                          {{ cachedModelIds.has(model.id) ? '已缓存' : '未缓存' }}
                        </span>
                      </td>
                      <td class="cached-at-cell">
                        <span v-if="cachedModelMetadata.get(model.id)">
                          {{ new Date(cachedModelMetadata.get(model.id)!.cached_at).toLocaleString() }}
                        </span>
                        <span v-else>—</span>
                      </td>
                      <td class="row-actions">
                        <template v-if="!cachedModelIds.has(model.id)">
                          <button
                            class="link-btn"
                            @click="addModels([model.id])"
                            :disabled="addingModels"
                          >
                            {{ addingModels ? '添加中...' : '添加' }}
                          </button>
                        </template>
                        <template v-else>
                          <label class="remove-checkbox">
                            <input
                              type="checkbox"
                              :checked="selectedRemoveIds.has(model.id)"
                              @change="toggleRemoveSelection(model.id, ($event.target as HTMLInputElement).checked)"
                            />
                            <span>选择移除</span>
                          </label>
                        </template>
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>
          </div>
          <div class="modal-actions">
            <button @click="closeModelsModal" class="cancel-btn">关闭</button>
          </div>
        </div>
      </div>
    </div>

  </div>
</template>

<style scoped>
.provider-manager {
  display: flex;
  flex-direction: column;
  gap: 16px;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  padding: 16px;
  background: white;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.04);
  height: calc(100vh - 168px);
  overflow: hidden;
}

.columns {
  display: flex;
  gap: 24px;
  flex: 1;
  min-height: 0;
}

.providers {
  width: 280px;
  flex-shrink: 0;
  border: 1px solid #e8e8e8;
  border-radius: 8px;
  padding: 20px;
  background: #fafafa;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.providers header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: 12px;
  border-bottom: 1px solid #e0e0e0;
}

.providers h3 {
  font-size: 16px;
  font-weight: 600;
  color: #333;
  margin: 0;
}

.providers ul {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
  overflow-y: auto;
}

.providers li {
  border: 1px solid #e0e0e0;
  border-radius: 6px;
  padding: 12px;
  cursor: pointer;
  transition: all 0.2s ease;
  background: white;
}

.providers li:hover {
  border-color: #d0d0d0;
  background-color: #f8f9fa;
}

.providers li.active {
  border-color: #333;
  background: #f0f7ff;
  box-shadow: 0 2px 4px rgba(0, 0, 0, 0.1);
}

.provider-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.provider-item span {
  font-weight: 500;
  color: #333;
  font-size: 14px;
}

.provider-item small {
  color: #666;
  font-size: 12px;
}

.details {
  flex: 1;
  border: 1px solid #e8e8e8;
  border-radius: 8px;
  padding: 24px;
  background: white;
  display: flex;
  flex-direction: column;
  gap: 24px;
  overflow-y: auto;
}

.details-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: 16px;
  border-bottom: 1px solid #f0f0f0;
}

.details-header h3 {
  font-size: 18px;
  font-weight: 600;
  color: #333;
  margin: 0;
}

.provider-form {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 20px;
  align-items: start;
}

.provider-form label {
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-size: 14px;
  font-weight: 500;
  color: #333;
}

.provider-form input,
.provider-form select {
  padding: 12px 16px;
  border-radius: 6px;
  border: 1px solid #ddd;
  font-size: 14px;
  color: #333;
  transition: border-color 0.2s ease;
  background: white;
}

.provider-form input:focus,
.provider-form select:focus {
  outline: none;
  border-color: #999;
}

.provider-form input:disabled {
  background-color: #f8f9fa;
  color: #666;
}

.form-actions {
  grid-column: 1 / -1;
  display: flex;
  gap: 12px;
  align-items: center;
  padding-top: 8px;
}

.keys {
  border-top: 1px solid #f0f0f0;
  padding-top: 24px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.keys header {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
  margin-bottom: 16px;
}

.keys h4 {
  font-size: 16px;
  font-weight: 600;
  color: #333;
  margin: 0;
}

.meta {
  color: #666;
  font-size: 13px;
  margin: 4px 0 0 0;
}

.key-actions {
  display: flex;
  gap: 8px;
  align-items: center;
  flex-wrap: wrap;
}

.key-actions input {
  padding: 8px 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  font-size: 13px;
  width: 200px;
}

.keys-container {
  margin-bottom: 24px;
}

.empty-keys {
  text-align: center;
  color: #999;
  font-style: italic;
  padding: 40px 20px;
  border: 2px dashed #e0e0e0;
  border-radius: 8px;
  background: #fafafa;
}

.key-list-container {
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  background: white;
}

.key-list-header {
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 16px;
  padding: 12px 16px;
  background: #f8f9fa;
  border-bottom: 1px solid #e0e0e0;
  font-weight: 600;
  color: #333;
  font-size: 14px;
}

.key-list-scrollable {
  max-height: 400px;
  overflow-y: auto;
}

.key-item {
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 16px;
  padding: 12px 16px;
  border-bottom: 1px solid #f0f0f0;
  cursor: pointer;
  transition: background-color 0.2s ease;
}

.key-item:last-child {
  border-bottom: none;
}

.key-item:hover {
  background-color: #f8f9fa;
}

.key-item.selected {
  background-color: #e3f2fd;
  border-color: #2196f3;
}


.key-checkbox {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  margin: 0;
  font-weight: normal;
}

.key-text {
  font-family: monospace;
  font-size: 13px;
  word-break: break-all;
}

.toggle-key-btn {
  padding: 4px 8px;
  font-size: 12px;
  border: 1px solid #90caf9;
  background: #e3f2fd;
  color: #1976d2;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.2s ease;
  margin-right: 8px;
}

.toggle-key-btn:hover {
  background: #bbdefb;
  border-color: #1976d2;
}

.delete-key-btn {
  padding: 4px 8px;
  font-size: 12px;
  border: 1px solid #ffcdd2;
  background: white;
  color: #d32f2f;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.2s ease;
}

.delete-key-btn:hover {
  background: #ffebee;
  border-color: #d32f2f;
}

.batch-operations {
  display: flex;
  flex-direction: column;
  gap: 24px;
  padding: 20px;
  background: #f8f9fa;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
}

.batch-delete-section,
.batch-add-section {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.batch-delete-section h5,
.batch-add-section h5 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: #333;
}

.batch-controls {
  display: flex;
  align-items: flex-start;
  gap: 16px;
  flex-wrap: wrap;
}

.or-text {
  color: #666;
  font-style: italic;
  margin: 8px 0;
}

.manual-delete {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
  min-width: 300px;
}

.add-controls {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.single-add {
  display: flex;
  gap: 8px;
  align-items: center;
}

.single-add input {
  flex: 1;
  padding: 8px 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  font-size: 14px;
}

.batch-add {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.batch-add textarea {
  min-height: 100px;
}

textarea {
  min-height: 80px;
  padding: 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  font-family: monospace;
  font-size: 12px;
  resize: vertical;
  background: white;
}

.placeholder {
  text-align: center;
  color: #666;
  font-style: italic;
  padding: 40px 20px;
  border: 2px dashed #e0e0e0;
  border-radius: 8px;
  background: #fafafa;
}

.error {
  color: #d32f2f;
  font-size: 13px;
  padding: 8px 12px;
  background: #ffebee;
  border: 1px solid #ffcdd2;
  border-radius: 4px;
}

button {
  padding: 8px 16px;
  border-radius: 4px;
  border: 1px solid #ddd;
  background: white;
  color: #333;
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  transition: all 0.2s ease;
}

button:hover:not(:disabled) {
  border-color: #999;
  background: #f8f9fa;
}

button:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

button.danger {
  color: #d32f2f;
  border-color: #ffcdd2;
}

button.danger:hover:not(:disabled) {
  border-color: #d32f2f;
  background: #ffebee;
}

button.link {
  border: none;
  background: none;
  color: #1976d2;
  padding: 4px 8px;
  text-decoration: underline;
}

button.link:hover {
  background: #e3f2fd;
  text-decoration: none;
}

/* 确认弹窗样式 */
.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.modal-content {
  background: white;
  border-radius: 8px;
  width: 90%;
  max-width: 450px;
  overflow: hidden;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15);
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 24px;
  background: #f8f9fa;
  border-bottom: 1px solid #e0e0e0;
}

.modal-header h4 {
  font-size: 16px;
  font-weight: 600;
  color: #333;
  margin: 0;
}

.close-btn {
  background: none;
  border: none;
  font-size: 20px;
  color: #666;
  cursor: pointer;
  padding: 4px;
  line-height: 1;
  border-radius: 4px;
  transition: background 0.2s ease;
}

.close-btn:hover {
  background: #e0e0e0;
}

.modal-body {
  padding: 24px;
}

.modal-body p {
  margin: 0 0 16px 0;
  font-size: 14px;
  color: #333;
}

.key-preview {
  padding: 12px;
  background: #f8f9fa;
  border-radius: 6px;
  margin-bottom: 20px;
  border: 1px solid #e0e0e0;
}

.key-preview code {
  font-family: monospace;
  font-size: 13px;
  color: #333;
  word-break: break-all;
}

.modal-actions {
  display: flex;
  gap: 12px;
  justify-content: flex-end;
}

.danger-btn {
  background: #d32f2f;
  color: white;
  border: 1px solid #d32f2f;
}

.danger-btn:hover {
  background: #b71c1c;
  border-color: #b71c1c;
}

.cancel-btn {
  background: white;
  color: #333;
  border: 1px solid #ddd;
}

.cancel-btn:hover {
  border-color: #999;
  background: #f8f9fa;
}

.warning-text {
  color: #f57c00;
  font-size: 13px;
  margin: 8px 0 16px 0;
  padding: 8px 12px;
  background: #fff3e0;
  border-radius: 4px;
  border-left: 3px solid #f57c00;
}

/* 模型获取弹窗特殊样式 */
.modal-content.models-modal {
  width: 95vw;
  max-width: 1600px;
  max-height: 88vh;
  display: flex;
  flex-direction: column;
}

.models-modal .modal-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 24px;
}

.loading-models {
  text-align: center;
  padding: 40px 20px;
  color: #666;
}

.no-models {
  text-align: center;
  padding: 40px 20px;
  color: #999;
  font-style: italic;
}

.models-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
  margin-bottom: 16px;
}

.selection-info {
  color: #666;
  font-size: 13px;
}

.toolbar-actions {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
}

.refresh-btn {
  border: 1px solid #ddd;
  background: #fff;
}

.primary-btn {
  background: #1976d2;
  color: #fff;
  border: 1px solid #1976d2;
}

.primary-btn:hover:not(:disabled) {
  background: #1559a5;
  border-color: #1559a5;
}

.danger-btn {
  background: #d32f2f;
  color: #fff;
  border: 1px solid #d32f2f;
}

.danger-btn:disabled {
  opacity: .6;
}

.divider {
  margin: 0 8px;
  color: #bbb;
}

.remove-checkbox {
  display: inline-flex;
  gap: 6px;
  align-items: center;
  color: #d32f2f;
}

.models-table-wrapper {
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  overflow: hidden;
  max-height: 65vh;
  overflow: auto;
  background: white;
}

.models-table {
  width: 100%;
  border-collapse: collapse;
}

.models-table th,
.models-table td {
  padding: 12px 16px;
  border-bottom: 1px solid #f0f0f0;
  font-size: 13px;
  vertical-align: middle;
}

.models-table thead th {
  background: #f8f9fa;
  font-weight: 600;
  color: #333;
  position: sticky;
  top: 0;
  z-index: 1;
}

.models-table tbody tr:hover {
  background: #f8f9fa;
}

.models-table tbody tr.cached {
  background: #f1f8e9;
}

.select-cell {
  width: 60px;
  text-align: center;
}

.model-id-cell {
  font-family: monospace;
  color: #333;
}

.model-id-text {
  font-size: 13px;
  font-weight: 500;
  word-break: break-all;
}

.model-meta {
  display: flex;
  gap: 12px;
  font-size: 12px;
  color: #666;
  margin-top: 4px;
  flex-wrap: wrap;
}

.status-cell {
  text-align: center;
}

.status-pill {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 4px 10px;
  border-radius: 12px;
  font-size: 12px;
  font-weight: 500;
}

.status-pill.cached {
  background: #e8f5e9;
  color: #2e7d32;
}

.status-pill.pending {
  background: #fff3e0;
  color: #ef6c00;
}

.cached-at-cell {
  color: #666;
  font-size: 12px;
}

.row-actions {
  text-align: center;
}

.link-btn {
  border: none;
  background: none;
  color: #1976d2;
  cursor: pointer;
  padding: 4px 8px;
}

.link-btn:hover:not(:disabled) {
  text-decoration: underline;
  background: rgba(25, 118, 210, 0.08);
}

.danger-link {
  border: none;
  background: none;
  color: #d32f2f;
  cursor: pointer;
  padding: 4px 8px;
}

.danger-link:hover:not(:disabled) {
  text-decoration: underline;
  background: rgba(211, 47, 47, 0.08);
}

.muted-text {
  color: #999;
  font-size: 12px;
}

.header-actions {
  display: flex;
  gap: 12px;
  align-items: center;
}

.models-btn {
  background: #4caf50;
  color: white;
  border: 1px solid #4caf50;
}

.models-btn:hover {
  background: #388e3c;
  border-color: #388e3c;
}

@media (max-width: 1024px) {
  .columns {
    flex-direction: column;
    height: auto;
  }

  .providers {
    width: 100%;
  }

  .providers ul {
    max-height: 200px;
  }

  .provider-form {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 768px) {
  .provider-manager {
    padding: 16px;
    gap: 20px;
  }

  .columns {
    gap: 16px;
  }

  .details,
  .providers {
    padding: 16px;
  }

  .keys header {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }

  .search input {
    width: 100%;
  }

  .batch-add .inline {
    flex-direction: column;
    align-items: stretch;
  }

  .batch-add .inline input {
    min-width: auto;
  }
}
</style>

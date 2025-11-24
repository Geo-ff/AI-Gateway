<script setup lang="ts">
import { onMounted, ref, computed, reactive, watch } from 'vue'
import { useNotify } from '../../composables/useNotify'
import { listProviders, type Provider } from '../../api/providers'
import {
  listModels,
  listProviderModels,
  listCachedModels,
  type ModelInfo,
  type CachedModel,
} from '../../api/models'
import { listModelPrices, getModelPrice, upsertModelPrice, type ModelPrice } from '../../api/prices'

interface EnhancedModelInfo extends ModelInfo {
  provider_name?: string
  price?: ModelPrice
  has_price?: boolean
  cached_at?: string
}

const providers = ref<Provider[]>([])
const selectedProvider = ref<string>('')
const models = ref<EnhancedModelInfo[]>([])
const prices = ref<ModelPrice[]>([])
const loading = ref(false)
const loadingPrices = ref(false)
const message = ref<string | null>(null)
const error = ref<string | null>(null)

// 模态框状态
const showPriceModal = ref(false)
const currentModel = ref<EnhancedModelInfo | null>(null)

const priceForm = reactive({
  prompt_price_per_million: 0,
  completion_price_per_million: 0,
  currency: 'USD'
})

// 过滤选项
const searchText = ref('')
const sortBy = ref<'id' | 'provider' | 'price'>('id')
const showOnlyWithPrice = ref(false)
const showOnlyWithoutPrice = ref(false)

const filteredModels = computed(() => {
  let result = models.value

  // 文本搜索
  if (searchText.value.trim()) {
    const search = searchText.value.toLowerCase().trim()
    result = result.filter(m =>
      m.id.toLowerCase().includes(search) ||
      (m.provider_name?.toLowerCase() || '').includes(search)
    )
  }

  // 价格过滤
  if (showOnlyWithPrice.value) {
    result = result.filter(m => m.has_price)
  }
  if (showOnlyWithoutPrice.value) {
    result = result.filter(m => !m.has_price)
  }

  // 排序
  result.sort((a, b) => {
    switch (sortBy.value) {
      case 'provider':
        return (a.provider_name || '').localeCompare(b.provider_name || '')
      case 'price':
        if (a.has_price && !b.has_price) return -1
        if (!a.has_price && b.has_price) return 1
        if (a.price && b.price) {
          return b.price.prompt_price_per_million - a.price.prompt_price_per_million
        }
        return 0
      default:
        return a.id.localeCompare(b.id)
    }
  })

  return result
})

function extractBackendModelId(model: EnhancedModelInfo): string {
  const providerName = model.provider_name || ''
  const id = model.id
  if (providerName && id.startsWith(`${providerName}/`)) {
    return id.slice(providerName.length + 1)
  }
  return id
}

async function loadProviders() {
  try {
    providers.value = await listProviders()
  } catch (err: any) {
    error.value = err?.message || String(err)
  }
}

async function loadModels() {
  loading.value = true
  error.value = null
  try {
    let modelList: ModelInfo[]
    if (selectedProvider.value) {
      modelList = await listProviderModels(selectedProvider.value)
    } else {
      modelList = await listModels()
    }

    const cachedResponse = await listCachedModels(
      selectedProvider.value && selectedProvider.value.trim() !== '' ? selectedProvider.value : undefined,
    )
    const cachedMap = new Map<string, CachedModel>()
    cachedResponse.data.forEach((entry) => {
      cachedMap.set(`${entry.provider}/${entry.id}`, entry)
    })

    // 增强模型信息
    const enhanced: EnhancedModelInfo[] = modelList.map((model) => {
      // 优先级：1. 模型自带的provider字段  2. 从模型ID推断  3. 当前选择的provider（仅当非空且非"全部"时）
      let providerName = model.provider || getProviderForModel(model.id)

      // 只有在选择了具体供应商（非"全部"）时才覆盖
      if (selectedProvider.value && selectedProvider.value !== '' && !model.provider) {
        providerName = selectedProvider.value
      }

      const normalizedId = model.id.includes('/') ? model.id.split('/').pop() || model.id : model.id
      const candidateKeys: string[] = []
      if (model.id.includes('/')) {
        candidateKeys.push(model.id)
      }
      if (providerName && providerName !== '未知') {
        candidateKeys.push(`${providerName}/${normalizedId}`)
      }
      if (selectedProvider.value) {
        candidateKeys.push(`${selectedProvider.value}/${normalizedId}`)
      }
      const cachedEntry = candidateKeys
        .map((key) => cachedMap.get(key))
        .find((entry): entry is CachedModel => Boolean(entry))

      return {
        ...model,
        provider_name: providerName,
        has_price: false,
        cached_at: model.cached_at ?? cachedEntry?.cached_at,
      }
    })

    models.value = enhanced
    await loadPrices()
  } catch (err: any) {
    error.value = err?.message || String(err)
    models.value = []
  } finally {
    loading.value = false
  }
}

async function loadPrices() {
  loadingPrices.value = true
  try {
    const providerFilter = selectedProvider.value && selectedProvider.value.trim() !== '' ? selectedProvider.value : undefined
    prices.value = await listModelPrices(providerFilter)

    // 将价格信息关联到模型
    models.value = models.value.map((model) => {
      const normalizedId = extractBackendModelId(model)
      const providerKey = model.provider_name?.toLowerCase()
      const price = prices.value.find((p) => {
        if (p.model !== normalizedId) return false
        if (providerKey) {
          return p.provider.toLowerCase() === providerKey
        }
        return true
      })
      return {
        ...model,
        price,
        has_price: !!price
      }
    })
  } catch (err: any) {
    console.warn('Failed to load prices:', err)
  } finally {
    loadingPrices.value = false
  }
}

function getProviderForModel(modelId: string): string {
  // 首先检查模型ID是否有斜杠格式，如果有，提取斜杠前的部分
  if (modelId.includes('/')) {
    const providerFromId = modelId.split('/')[0]
    if (providerFromId) {
      return providerFromId
    }
  }

  // 基于模型名关键词的映射逻辑
  if (modelId.includes('gpt') || modelId.includes('o1')) return 'OpenAI'
  if (modelId.includes('claude')) return 'Anthropic'
  if (modelId.includes('gemini') || modelId.includes('bard')) return 'Google'
  if (modelId.includes('llama')) return 'Meta'
  if (modelId.includes('qwen') || modelId.includes('通义')) return 'Alibaba'
  if (modelId.includes('glm') || modelId.includes('chatglm')) return 'Zhipu'
  return '未知'
}

const { showMessage } = useNotify()

function openPriceModal(model: EnhancedModelInfo) {
  currentModel.value = model
  if (model.price) {
    priceForm.prompt_price_per_million = model.price.prompt_price_per_million
    priceForm.completion_price_per_million = model.price.completion_price_per_million
    priceForm.currency = model.price.currency || 'USD'
  } else {
    priceForm.prompt_price_per_million = 0
    priceForm.completion_price_per_million = 0
    priceForm.currency = 'USD'
  }
  showPriceModal.value = true
}

function closePriceModal() {
  showPriceModal.value = false
  currentModel.value = null
}

async function savePriceSettings() {
  if (!currentModel.value) return

  try {
    const backendModelId = extractBackendModelId(currentModel.value)
    await upsertModelPrice({
      provider: currentModel.value.provider_name!,
      model: backendModelId,
      prompt_price_per_million: priceForm.prompt_price_per_million,
      completion_price_per_million: priceForm.completion_price_per_million,
      currency: priceForm.currency || 'USD'
    })

    showMessage('价格设置已保存', 'success')
    closePriceModal()
    await loadPrices()
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  }
}

// 监听供应商变化
watch(selectedProvider, loadModels)

onMounted(async () => {
  await loadProviders()
  await loadModels()
})
</script>

<template>
  <div class="model-manager">
    <header class="manager-header">
      <h3>模型管理</h3>
      <div class="header-controls">
        <div class="provider-selector">
          <label>供应商</label>
          <select v-model="selectedProvider">
            <option value="">全部</option>
            <option v-for="p in providers" :key="p.name" :value="p.name">{{ p.name }}</option>
          </select>
        </div>
        <div class="cache-actions">
          <button @click="loadModels" :disabled="loading">{{ loading ? '加载中...' : '刷新' }}</button>
        </div>
      </div>
    </header>

    <div class="filters-bar">
      <div class="search-section">
        <input
          v-model="searchText"
          placeholder="搜索模型或供应商..."
          class="search-input"
        />
      </div>
      <div class="filter-section">
        <label class="filter-checkbox">
          <input type="checkbox" v-model="showOnlyWithPrice" />
          <span>仅显示已设价格</span>
        </label>
        <label class="filter-checkbox">
          <input type="checkbox" v-model="showOnlyWithoutPrice" />
          <span>仅显示未设价格</span>
        </label>
      </div>
      <div class="sort-section">
        <label>排序</label>
        <select v-model="sortBy">
          <option value="id">按模型ID</option>
          <option value="provider">按供应商</option>
          <option value="price">按价格状态</option>
        </select>
      </div>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <div class="models-container">
      <div v-if="loading" class="loading">加载模型中...</div>
      <div v-else-if="!filteredModels.length" class="empty-state">
        <p>{{ models.length === 0 ? '暂无模型数据' : '没有符合筛选条件的模型' }}</p>
      </div>
      <div v-else class="models-table-container">
        <table class="models-table">
          <thead>
            <tr>
              <th>模型ID</th>
              <th>供应商</th>
              <th>创建时间</th>
              <th>价格状态</th>
              <th>Prompt价格</th>
              <th>Completion价格</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="model in filteredModels" :key="`${model.provider_name}-${model.id}`">
              <td class="model-id">{{ model.id }}</td>
              <td class="provider-name">{{ model.provider_name || '未知' }}</td>
              <td class="created-time">
                <span v-if="model.cached_at">
                  {{ new Date(model.cached_at).toLocaleString('zh-CN', {
                    year: 'numeric',
                    month: '2-digit',
                    day: '2-digit',
                    hour: '2-digit',
                    minute: '2-digit',
                    second: '2-digit'
                  }) }}
                </span>
                <span v-else-if="model.created">
                  {{ new Date(model.created * 1000).toLocaleString('zh-CN', {
                    year: 'numeric',
                    month: '2-digit',
                    day: '2-digit',
                    hour: '2-digit',
                    minute: '2-digit',
                    second: '2-digit'
                  }) }}
                </span>
                <span v-else>—</span>
              </td>
              <td class="price-status">
                <span :class="['status-badge', model.has_price ? 'has-price' : 'no-price']">
                  {{ model.has_price ? '已设置' : '未设置' }}
                </span>
              </td>
              <td class="price-value">
                {{ model.price ? `$${model.price.prompt_price_per_million}/M` : '—' }}
              </td>
              <td class="price-value">
                {{ model.price ? `$${model.price.completion_price_per_million}/M` : '—' }}
              </td>
              <td class="actions">
                <button
                  @click="openPriceModal(model)"
                  class="price-btn"
                  :class="{ 'has-price': model.has_price }"
                >
                  {{ model.has_price ? '编辑价格' : '设置价格' }}
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- 价格设置模态框 -->
    <div v-if="showPriceModal" class="modal-overlay" @click="closePriceModal">
      <div class="modal-content" @click.stop>
        <header class="modal-header">
          <h4>设置模型价格</h4>
          <button @click="closePriceModal" class="close-btn">✕</button>
        </header>
        <div class="modal-body">
          <div class="model-info">
            <p><strong>模型:</strong> {{ currentModel?.id }}</p>
            <p><strong>供应商:</strong> {{ currentModel?.provider_name }}</p>
          </div>
          <form @submit.prevent="savePriceSettings" class="price-form">
            <div class="form-row">
              <label>
                Prompt价格 (每百万Token)
                <input
                  type="number"
                  v-model="priceForm.prompt_price_per_million"
                  step="0.01"
                  min="0"
                  required
                />
              </label>
              <label>
                Completion价格 (每百万Token)
                <input
                  type="number"
                  v-model="priceForm.completion_price_per_million"
                  step="0.01"
                  min="0"
                  required
                />
              </label>
            </div>
            <div class="form-row">
              <label>
                货币
                <select v-model="priceForm.currency">
                  <option value="USD">USD</option>
                  <option value="CNY">CNY</option>
                  <option value="EUR">EUR</option>
                </select>
              </label>
            </div>
            <div class="form-actions">
              <button type="submit" class="save-btn">保存设置</button>
              <button type="button" @click="closePriceModal" class="cancel-btn">取消</button>
            </div>
          </form>
        </div>
      </div>
    </div>

  </div>
</template>

<style scoped>
.model-manager {
  display: flex;
  flex-direction: column;
  gap: 20px;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  padding: 24px;
  background: white;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.04);
  height: calc(100vh - 120px);
  overflow: hidden;
}

.manager-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: 16px;
  border-bottom: 1px solid #f0f0f0;
}

.manager-header h3 {
  font-size: 18px;
  font-weight: 600;
  color: #333;
  margin: 0;
}

.header-controls {
  display: flex;
  align-items: center;
  gap: 16px;
}

.provider-selector {
  display: flex;
  align-items: center;
  gap: 8px;
}

.provider-selector label {
  font-weight: 500;
  color: #333;
  white-space: nowrap;
}

.provider-selector select {
  padding: 8px 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  font-size: 14px;
  background: white;
  min-width: 120px;
}

.cache-actions {
  display: flex;
  gap: 8px;
}

.filters-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding: 16px;
  background: #f8f9fa;
  border-radius: 8px;
  border: 1px solid #e8e8e8;
  flex-wrap: wrap;
}

.search-section {
  flex: 1;
  min-width: 250px;
  max-width: 400px;
}

.search-input {
  width: 100%;
  padding: 8px 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  font-size: 14px;
}

.filter-section {
  display: flex;
  gap: 16px;
  align-items: center;
}

.filter-checkbox {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 14px;
  color: #333;
  cursor: pointer;
}

.filter-checkbox input {
  margin: 0;
}

.sort-section {
  display: flex;
  align-items: center;
  gap: 8px;
  white-space: nowrap;
}

.sort-section label {
  font-weight: 500;
  color: #333;
}

.sort-section select {
  padding: 8px 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  font-size: 14px;
  background: white;
  min-width: 120px;
}

.models-container {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.loading {
  text-align: center;
  padding: 40px 20px;
  color: #666;
  font-size: 16px;
}

.empty-state {
  text-align: center;
  padding: 60px 20px;
  color: #999;
  font-style: italic;
  border: 2px dashed #e0e0e0;
  border-radius: 8px;
  background: #fafafa;
}

.models-table-container {
  flex: 1;
  overflow: auto;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
}

.models-table {
  width: 100%;
  border-collapse: collapse;
  background: white;
  table-layout: fixed;
}

.models-table thead {
  position: sticky;
  top: 0;
  background: #f8f9fa;
  z-index: 1;
}

.models-table th {
  padding: 12px 16px;
  text-align: left;
  font-weight: 600;
  color: #333;
  border-bottom: 2px solid #e0e0e0;
  font-size: 14px;
}

.models-table th:nth-child(1) { width: 30%; } /* 模型ID */
.models-table th:nth-child(2) { width: 11%; } /* 供应商 */
.models-table th:nth-child(3) { width: 16%; } /* 创建时间 */
.models-table th:nth-child(4) { width: 10%; } /* 价格状态 */
.models-table th:nth-child(5) { width: 12%; } /* Prompt价格 */
.models-table th:nth-child(6) { width: 12%; } /* Completion价格 */
.models-table th:nth-child(7) { width: 9%; } /* 操作 */

.models-table th:nth-child(3),
.models-table th:nth-child(4),
.models-table th:nth-child(5),
.models-table th:nth-child(6),
.models-table th:nth-child(7) {
  text-align: center;
}

.models-table td {
  padding: 12px 16px;
  border-bottom: 1px solid #f0f0f0;
  font-size: 14px;
}

.models-table tbody tr:hover {
  background-color: #f8f9fa;
}

.model-id {
  font-family: monospace;
  font-size: 13px;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.provider-name {
  font-weight: 500;
  color: #333;
}

.created-time {
  color: #666;
  font-size: 13px;
  text-align: center;
}

.price-status {
  text-align: center;
}

.status-badge {
  padding: 4px 8px;
  border-radius: 12px;
  font-size: 12px;
  font-weight: 500;
}

.status-badge.has-price {
  background: #e8f5e8;
  color: #2e7d32;
}

.status-badge.no-price {
  background: #fff3e0;
  color: #f57c00;
}

.price-value {
  font-family: monospace;
  font-size: 13px;
  text-align: center;
  color: #333;
}

.actions {
  text-align: center;
}

.price-btn {
  padding: 6px 12px;
  border-radius: 4px;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
  border: 1px solid #ddd;
  background: white;
  color: #333;
}

.price-btn:hover {
  border-color: #999;
  background: #f8f9fa;
}

.price-btn.has-price {
  border-color: #2196f3;
  color: #1976d2;
}

.price-btn.has-price:hover {
  background: #e3f2fd;
  border-color: #1976d2;
}

/* 模态框样式 */
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
  max-width: 500px;
  max-height: 80vh;
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

.model-info {
  padding: 16px;
  background: #f8f9fa;
  border-radius: 6px;
  margin-bottom: 20px;
}

.model-info p {
  margin: 0 0 8px 0;
  font-size: 14px;
  color: #333;
}

.model-info p:last-child {
  margin-bottom: 0;
}

.price-form {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.form-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
}

.form-row:last-of-type {
  grid-template-columns: 1fr;
}

.price-form label {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 14px;
  font-weight: 500;
  color: #333;
}

.price-form input,
.price-form select {
  padding: 10px 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  font-size: 14px;
  transition: border-color 0.2s ease;
}

.price-form input:focus,
.price-form select:focus {
  outline: none;
  border-color: #2196f3;
}

.form-actions {
  display: flex;
  gap: 12px;
  justify-content: flex-end;
  padding-top: 16px;
  border-top: 1px solid #f0f0f0;
}

.save-btn,
.cancel-btn {
  padding: 10px 16px;
  border-radius: 4px;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
}

.save-btn {
  background: #2196f3;
  color: white;
  border: 1px solid #2196f3;
}

.save-btn:hover {
  background: #1976d2;
  border-color: #1976d2;
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

/* 通用按钮样式 */
button {
  padding: 8px 16px;
  border-radius: 4px;
  border: 1px solid #ddd;
  background: white;
  color: #333;
  cursor: pointer;
  font-size: 14px;
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

/* 消息样式 */
.error {
  color: #d32f2f;
  font-size: 14px;
  padding: 12px 16px;
  background: #ffebee;
  border: 1px solid #ffcdd2;
  border-radius: 6px;
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

.danger-btn {
  background: #d32f2f;
  color: white;
  border: 1px solid #d32f2f;
}

.danger-btn:hover {
  background: #b71c1c;
  border-color: #b71c1c;
}

/* 响应式设计 */
@media (max-width: 1024px) {
  .filters-bar {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }

  .header-controls {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }

  .form-row {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 768px) {
  .model-manager {
    padding: 16px;
  }

  .models-table th,
  .models-table td {
    padding: 8px 12px;
  }

  .modal-content {
    width: 95%;
    margin: 20px;
  }

  .modal-body {
    padding: 16px;
  }

  .model-id {
    max-width: 150px;
  }
}
</style>

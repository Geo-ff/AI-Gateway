<script setup lang="ts">
import { onMounted, reactive, ref, computed } from 'vue'
import { useNotify } from '../../composables/useNotify'
import ConfirmDialog from '../ConfirmDialog.vue'
import {
  listTokens,
  createToken,
  updateToken,
  toggleToken,
  deleteToken,
  type ClientToken,
  type CreateTokenBody,
  type UpdateTokenBody,
} from '../../api/tokens'
import { listModels, type ModelInfo } from '../../api/models'

const tokens = ref<ClientToken[]>([])
const availableModels = ref<ModelInfo[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const message = ref<string | null>(null)

// 弹窗状态
const showCreateModal = ref(false)
const showEditModal = ref(false)
const currentToken = ref<ClientToken | null>(null)
const showDeleteModal = ref(false)
const tokenToDelete = ref<ClientToken | null>(null)

// 搜索和过滤
const searchText = ref('')
const statusFilter = ref<'all' | 'enabled' | 'disabled' | 'expired'>('all')
const sortBy = ref<'created' | 'amount' | 'tokens' | 'expires'>('created')

const tokenForm = reactive({
  name: '',
  allowedModels: [] as string[],
  maxAmount: null as number | null,
  maxTokens: null as number | null,
  enabled: true,
  expiresAt: '',
})

const deleteConfirmMessage = computed(() => {
  if (!tokenToDelete.value) return '确定要删除此令牌吗？该操作不可恢复。'
  const name = tokenToDelete.value.name?.trim()
  const hint = name ? `${name} (${tokenToDelete.value.token.substring(0, 8)}...)` : `${tokenToDelete.value.token.substring(0, 8)}...`
  return `确定要删除令牌 ${hint} 吗？该操作不可恢复。`
})

const filteredTokens = computed(() => {
  let result = tokens.value

  // 文本搜索
  if (searchText.value.trim()) {
    const search = searchText.value.toLowerCase().trim()
    result = result.filter(token =>
      token.token.toLowerCase().includes(search) ||
      token.id.toLowerCase().includes(search) ||
      token.name.toLowerCase().includes(search) ||
      (token.allowed_models && token.allowed_models.some(model =>
        model.toLowerCase().includes(search)
      ))
    )
  }

  // 状态过滤
  if (statusFilter.value !== 'all') {
    result = result.filter(token => {
      switch (statusFilter.value) {
        case 'enabled':
          return token.enabled
        case 'disabled':
          return !token.enabled
        case 'expired':
          return token.expires_at && new Date(token.expires_at) < new Date()
        default:
          return true
      }
    })
  }

  // 排序
  result.sort((a, b) => {
    switch (sortBy.value) {
      case 'amount':
        return b.amount_spent - a.amount_spent
      case 'tokens':
        return b.total_tokens_spent - a.total_tokens_spent
      case 'expires':
        if (!a.expires_at && !b.expires_at) return 0
        if (!a.expires_at) return 1
        if (!b.expires_at) return -1
        return new Date(a.expires_at).getTime() - new Date(b.expires_at).getTime()
      default:
        return new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
    }
  })

  return result
})

const { showMessage } = useNotify()

function resetForm() {
  tokenForm.name = ''
  tokenForm.allowedModels = []
  tokenForm.maxAmount = null
  tokenForm.maxTokens = null
  tokenForm.enabled = true
  tokenForm.expiresAt = ''
}

function fillFormFromToken(token: ClientToken) {
  tokenForm.name = token.name || ''
  tokenForm.allowedModels = token.allowed_models ? [...token.allowed_models] : []
  tokenForm.maxAmount = token.max_amount ?? null
  tokenForm.maxTokens = token.max_tokens ?? null
  tokenForm.enabled = token.enabled
  // expires_at 形如 "YYYY-MM-DD HH:MM:SS"（北京时间字符串）
  if (token.expires_at) {
    const parts = token.expires_at.split(' ')
    if (parts.length === 2) {
      const [datePart, timePart] = parts
      const [hh, mm] = timePart.split(':')
      if (datePart && hh && mm) {
        tokenForm.expiresAt = `${datePart}T${hh}:${mm}`
        return
      }
    }
  }
  tokenForm.expiresAt = ''
}

function toBeijingStringFromLocalInput(value: string): string | null {
  if (!value) return null
  // value 形如 "YYYY-MM-DDTHH:MM"，转换为 "YYYY-MM-DD HH:MM:00"
  const [datePart, timePart] = value.split('T')
  if (!datePart || !timePart) return null
  const [hh, mm] = timePart.split(':')
  if (!hh || !mm) return null
  return `${datePart} ${hh}:${mm}:00`
}

async function loadTokens() {
  loading.value = true
  error.value = null
  try {
    tokens.value = await listTokens()
  } catch (err: any) {
    error.value = err?.message || String(err)
  } finally {
    loading.value = false
  }
}

async function loadModels() {
  try {
    availableModels.value = await listModels()
  } catch (err: any) {
    console.warn('Failed to load models:', err)
  }
}

function openCreateModal() {
  resetForm()
  showCreateModal.value = true
}

function openEditModal(token: ClientToken) {
  currentToken.value = token
  fillFormFromToken(token)
  showEditModal.value = true
}

function closeCreateModal() {
  showCreateModal.value = false
  resetForm()
}

function closeEditModal() {
  showEditModal.value = false
  currentToken.value = null
  resetForm()
}

function openDeleteModal(token: ClientToken) {
  tokenToDelete.value = token
  showDeleteModal.value = true
}

function closeDeleteModal() {
  showDeleteModal.value = false
  tokenToDelete.value = null
}

async function confirmDeleteToken() {
  if (!tokenToDelete.value) return
  try {
    await deleteToken(tokenToDelete.value.id)
    tokens.value = tokens.value.filter((t) => t.id !== tokenToDelete.value!.id)
    showMessage('令牌已删除', 'success')
    tokenToDelete.value = null
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  }
}

async function submitCreate() {
  try {
    const expiresStr = tokenForm.expiresAt ? toBeijingStringFromLocalInput(tokenForm.expiresAt) : null
    const validModelIds = new Set(availableModels.value.map((m) => m.id))
    const filteredModels = tokenForm.allowedModels.filter((id) => validModelIds.has(id))
    if (filteredModels.length !== tokenForm.allowedModels.length) {
      showMessage('部分模型已不再可用，已从令牌限制中移除', 'warning')
    }
    const payload: CreateTokenBody = {
      name: tokenForm.name.trim() || undefined,
      allowed_models: filteredModels.length ? filteredModels : undefined,
      max_amount: tokenForm.maxAmount ?? undefined,
      max_tokens: tokenForm.maxTokens ?? undefined,
      enabled: tokenForm.enabled,
      expires_at: expiresStr || undefined,
    }
    const created = await createToken(payload)
    tokens.value.unshift(created)
    showMessage('令牌已创建', 'success')
    closeCreateModal()
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  }
}

async function submitUpdate() {
  if (!currentToken.value) return
  try {
    const expiresStr = tokenForm.expiresAt ? toBeijingStringFromLocalInput(tokenForm.expiresAt) : null
    const validModelIds = new Set(availableModels.value.map((m) => m.id))
    const filteredModels = tokenForm.allowedModels.filter((id) => validModelIds.has(id))
    if (filteredModels.length !== tokenForm.allowedModels.length) {
      showMessage('部分模型已不再可用，已从令牌限制中移除', 'warning')
    }
    const payload: UpdateTokenBody = {
      name: tokenForm.name.trim() || undefined,
      // 空数组表示清空限制（允许所有模型）
      allowed_models: filteredModels,
      max_amount: tokenForm.maxAmount ?? null,
      max_tokens: tokenForm.maxTokens ?? null,
      enabled: tokenForm.enabled,
      // null => 清空过期时间；字符串 => 设置为该时间
      expires_at: expiresStr,
    }
    const updated = await updateToken(currentToken.value.id, payload)
    const idx = tokens.value.findIndex(t => t.id === updated.id)
    if (idx >= 0) {
      // 使用 Object.assign 确保 Vue 能检测到所有属性的变化
      Object.assign(tokens.value[idx], updated)
    }
    showMessage('令牌信息已更新', 'success')
    closeEditModal()
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  }
}

async function toggleEnabled(token: ClientToken) {
  try {
    const newState = !token.enabled
    await toggleToken(token.id, newState)
    const idx = tokens.value.findIndex(t => t.id === token.id)
    if (idx >= 0) {
      tokens.value[idx].enabled = newState
    }
    showMessage(`令牌已${newState ? '启用' : '禁用'}`, 'success')
  } catch (err: any) {
    showMessage(err?.message || String(err), 'error')
  }
}

function formatAmount(amount: number): string {
  return amount.toFixed(2)
}

function formatDate(date: string | null | undefined): string {
  if (!date) return '—'
  try {
    const parsed = new Date(date)
    if (Number.isNaN(parsed.getTime())) return date
    return parsed.toLocaleString()
  } catch {
    return date
  }
}

function getTokenStatus(token: ClientToken): 'active' | 'disabled' | 'expired' {
  if (token.expires_at && new Date(token.expires_at) < new Date()) {
    return 'expired'
  }
  return token.enabled ? 'active' : 'disabled'
}

function copyToClipboard(text: string) {
  navigator.clipboard?.writeText(text).then(() => {
    showMessage('令牌已复制到剪贴板', 'success')
  }).catch(() => {
    showMessage('复制失败', 'error')
  })
}

onMounted(async () => {
  await Promise.all([loadTokens(), loadModels()])
})
</script>

<template>
  <div class="token-manager">
    <header class="manager-header">
      <h3>令牌管理</h3>
      <div class="header-actions">
        <button @click="loadTokens" :disabled="loading" class="refresh-btn">
          {{ loading ? '加载中...' : '刷新' }}
        </button>
        <button @click="openCreateModal" class="create-btn">创建令牌</button>
      </div>
    </header>

    <div class="filters-bar">
      <div class="search-section">
        <input
          v-model="searchText"
          placeholder="搜索令牌或模型..."
          class="search-input"
        />
      </div>
      <div class="filter-section">
        <label>状态</label>
        <select v-model="statusFilter">
          <option value="all">全部</option>
          <option value="enabled">已启用</option>
          <option value="disabled">已禁用</option>
          <option value="expired">已过期</option>
        </select>
      </div>
      <div class="sort-section">
        <label>排序</label>
        <select v-model="sortBy">
          <option value="created">按创建时间</option>
          <option value="amount">按消费金额</option>
          <option value="tokens">按Token使用量</option>
          <option value="expires">按过期时间</option>
        </select>
      </div>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <div class="tokens-container">
      <div v-if="loading" class="loading">加载令牌中...</div>
      <div v-else-if="!filteredTokens.length" class="empty-state">
        <p>{{ tokens.length === 0 ? '暂无令牌' : '没有符合筛选条件的令牌' }}</p>
      </div>
      <div v-else class="tokens-table-container">
        <table class="tokens-table">
          <thead>
            <tr>
              <th>令牌</th>
              <th>状态</th>
              <th>消费金额</th>
              <th>最大消费金额</th>
              <th>Token使用量</th>
              <th>允许模型</th>
              <th>过期时间</th>
              <th>创建时间</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="token in filteredTokens" :key="token.id">
              <td class="token-cell">
                <div class="token-name">{{ token.name }}</div>
                <div class="token-display">
                  <code class="token-text">{{ token.token.substring(0, 8) }}...{{ token.token.substring(-8) }}</code>
                  <button @click="copyToClipboard(token.token)" class="copy-btn" title="复制令牌">复制</button>
                </div>
              </td>
              <td class="status-cell">
                <span :class="['status-badge', `status-${getTokenStatus(token)}`]">
                  {{ getTokenStatus(token) === 'active' ? '已启用' :
                     getTokenStatus(token) === 'expired' ? '已过期' : '已禁用' }}
                </span>
              </td>
              <td class="amount-cell">${{ formatAmount(token.amount_spent) }}</td>
              <td class="max-amount-cell">
                {{ token.max_amount != null ? `$${formatAmount(token.max_amount)}` : '—' }}
              </td>
              <td class="tokens-cell">{{ token.total_tokens_spent.toLocaleString() }}</td>
              <td class="models-cell">
                <div class="models-display">
                  <span v-if="!token.allowed_models?.length" class="no-limit">无限制</span>
                  <span v-else class="models-count">{{ token.allowed_models.length }}个模型</span>
                </div>
              </td>
              <td class="expires-cell">{{ formatDate(token.expires_at) }}</td>
              <td class="created-cell">{{ formatDate(token.created_at) }}</td>
              <td class="actions-cell">
                <div class="token-actions">
                  <button @click="openEditModal(token)" class="btn btn-secondary">编辑</button>
                  <button
                    @click="toggleEnabled(token)"
                    :class="['btn', token.enabled ? 'btn-danger' : 'btn-primary']"
                  >
                    {{ token.enabled ? '禁用' : '启用' }}
                  </button>
                  <button
                    @click="openDeleteModal(token)"
                    class="btn btn-danger"
                  >
                    删除
                  </button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- 删除令牌确认对话框 -->
    <ConfirmDialog
      v-model:show="showDeleteModal"
      title="删除令牌"
      :message="deleteConfirmMessage"
      confirm-text="删除"
      cancel-text="取消"
      @confirm="confirmDeleteToken"
    />

    <!-- 创建令牌模态框 -->
    <div v-if="showCreateModal" class="modal-overlay" @click="closeCreateModal">
      <div class="modal-content" @click.stop>
        <header class="modal-header">
          <h4>创建新令牌</h4>
          <button @click="closeCreateModal" class="close-btn">✕</button>
        </header>
        <div class="modal-body">
          <form @submit.prevent="submitCreate" class="token-form">
            <div class="form-group">
              <label>名称</label>
              <input
                type="text"
                v-model="tokenForm.name"
                maxlength="64"
                placeholder="例如：前端测试"
              />
            </div>
            <div class="form-group">
              <label>允许使用的模型</label>
              <div class="models-selector">
                <div v-if="!availableModels.length" class="no-models">
                  <p>未找到可用模型，请先配置供应商和模型</p>
                </div>
                <div v-else class="models-checkboxes">
                  <label v-for="model in availableModels" :key="model.id" class="model-checkbox">
                    <input
                      type="checkbox"
                      :value="model.id"
                      v-model="tokenForm.allowedModels"
                    />
                    <span class="model-name">{{ model.id }}</span>
                  </label>
                </div>
                <div class="models-hint">
                  <small>不选择任何模型表示允许所有模型</small>
                </div>
              </div>
            </div>

            <div class="form-row">
              <div class="form-group">
                <label>最大消费金额 (USD)</label>
                <input
                  type="number"
                  v-model="tokenForm.maxAmount"
                  step="0.01"
                  min="0"
                  placeholder="不限制"
                />
              </div>
              <div class="form-group">
                <label>最大Token使用量</label>
                <input
                  type="number"
                  v-model="tokenForm.maxTokens"
                  min="0"
                  placeholder="不限制"
                />
              </div>
            </div>

            <div class="form-row">
              <div class="form-group">
                <label>过期时间</label>
                <input type="datetime-local" v-model="tokenForm.expiresAt" />
              </div>
              <div class="form-group">
                <label class="checkbox-label">
                  <input type="checkbox" v-model="tokenForm.enabled" />
                  <span>立即启用</span>
                </label>
              </div>
            </div>

            <div class="form-actions">
              <button type="submit" class="save-btn">创建令牌</button>
              <button type="button" @click="closeCreateModal" class="cancel-btn">取消</button>
            </div>
          </form>
        </div>
      </div>
    </div>

    <!-- 编辑令牌模态框 -->
    <div v-if="showEditModal" class="modal-overlay" @click="closeEditModal">
      <div class="modal-content" @click.stop>
        <header class="modal-header">
          <h4>编辑令牌</h4>
          <button @click="closeEditModal" class="close-btn">✕</button>
        </header>
        <div class="modal-body">
          <div class="token-info">
            <p><strong>ID:</strong> <code>{{ currentToken?.id }}</code></p>
            <p><strong>名称:</strong> {{ currentToken?.name }}</p>
            <p><strong>令牌:</strong> <code>{{ currentToken?.token }}</code></p>
            <p><strong>创建时间:</strong> {{ formatDate(currentToken?.created_at) }}</p>
          </div>
          <form @submit.prevent="submitUpdate" class="token-form">
            <div class="form-group">
              <label>名称</label>
              <input
                type="text"
                v-model="tokenForm.name"
                maxlength="64"
                placeholder="例如：前端测试"
              />
            </div>
            <div class="form-group">
              <label>允许使用的模型</label>
              <div class="models-selector">
                <div v-if="!availableModels.length" class="no-models">
                  <p>未找到可用模型</p>
                </div>
                <div v-else class="models-checkboxes">
                  <label v-for="model in availableModels" :key="model.id" class="model-checkbox">
                    <input
                      type="checkbox"
                      :value="model.id"
                      v-model="tokenForm.allowedModels"
                    />
                    <span class="model-name">{{ model.id }}</span>
                  </label>
                </div>
                <div class="models-hint">
                  <small>不选择任何模型表示允许所有模型</small>
                </div>
              </div>
            </div>

            <div class="form-row">
              <div class="form-group">
                <label>最大消费金额 (USD)</label>
                <input
                  type="number"
                  v-model="tokenForm.maxAmount"
                  step="0.01"
                  min="0"
                  placeholder="不限制"
                />
                <small>当前已消费: ${{ formatAmount(currentToken?.amount_spent || 0) }}</small>
              </div>
              <div class="form-group">
                <label>最大Token使用量</label>
                <input
                  type="number"
                  v-model="tokenForm.maxTokens"
                  min="0"
                  placeholder="不限制"
                />
                <small>当前已使用: {{ (currentToken?.total_tokens_spent || 0).toLocaleString() }}</small>
              </div>
            </div>

            <div class="form-row">
              <div class="form-group">
                <label>过期时间</label>
                <input type="datetime-local" v-model="tokenForm.expiresAt" />
              </div>
              <div class="form-group">
                <label class="checkbox-label">
                  <input type="checkbox" v-model="tokenForm.enabled" />
                  <span>启用令牌</span>
                </label>
              </div>
            </div>

            <div class="form-actions">
              <button type="submit" class="save-btn">保存修改</button>
              <button type="button" @click="closeEditModal" class="cancel-btn">取消</button>
            </div>
          </form>
        </div>
      </div>
    </div>

  </div>
</template>

<style scoped>
.token-manager {
  display: flex;
  flex-direction: column;
  gap: 20px;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  padding: 24px;
  background: white;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.04);
  height: calc(100vh - 168px);
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

.header-actions {
  display: flex;
  gap: 12px;
}

.create-btn {
  background: #2196f3;
  color: white;
  border: 1px solid #2196f3;
}

.create-btn:hover {
  background: #1976d2;
  border-color: #1976d2;
}

.filters-bar {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 20px;
  padding: 16px;
  background: #f8f9fa;
  border-radius: 8px;
  border: 1px solid #e8e8e8;
  flex-wrap: wrap;
  min-height: 64px;
}

.search-section {
  flex: 1;
  min-width: 250px;
  max-width: 400px;
}

.search-input {
  width: 100%;
  padding: 10px 14px;
  border: 1px solid #ddd;
  border-radius: 6px;
  font-size: 14px;
  transition: border-color 0.2s ease, box-shadow 0.2s ease;
}

.search-input:focus {
  outline: none;
  border-color: #2196f3;
  box-shadow: 0 0 0 3px rgba(33, 150, 243, 0.1);
}

.filter-section,
.sort-section {
  display: flex;
  align-items: center;
  gap: 10px;
  white-space: nowrap;
  min-width: 140px;
}

.filter-section label,
.sort-section label {
  font-weight: 500;
  color: #333;
}

.filter-section select,
.sort-section select {
  padding: 10px 14px;
  border: 1px solid #ddd;
  border-radius: 6px;
  font-size: 14px;
  background: white;
  min-width: 130px;
  transition: border-color 0.2s ease;
}

.filter-section select:focus,
.sort-section select:focus {
  outline: none;
  border-color: #2196f3;
}

.tokens-container {
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

.tokens-table-container {
  flex: 1;
  overflow: auto;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
}

.tokens-table {
  width: 100%;
  border-collapse: collapse;
  background: white;
  table-layout: fixed;
}

.tokens-table th:nth-child(1) { width: 14%; } /* 令牌 */
.tokens-table th:nth-child(2) { width: 8%; } /* 状态 */
.tokens-table th:nth-child(3) { width: 9%; } /* 消费金额 */
.tokens-table th:nth-child(4) { width: 10%; } /* 最大消费金额 */
.tokens-table th:nth-child(5) { width: 10%; } /* Token使用量 */
.tokens-table th:nth-child(6) { width: 9%; } /* 允许模型 */
.tokens-table th:nth-child(7) { width: 12%; } /* 过期时间 */
.tokens-table th:nth-child(8) { width: 12%; } /* 创建时间 */
.tokens-table th:nth-child(9) { width: 16%; } /* 操作 */

.tokens-table thead {
  position: sticky;
  top: 0;
  background: #f8f9fa;
  z-index: 1;
}

.tokens-table th {
  padding: 12px 16px;
  text-align: left;
  font-weight: 600;
  color: #333;
  border-bottom: 2px solid #e0e0e0;
  font-size: 14px;
}

.tokens-table th:nth-child(2),
.tokens-table th:nth-child(3),
.tokens-table th:nth-child(4),
.tokens-table th:nth-child(5),
.tokens-table th:nth-child(6),
.tokens-table th:nth-child(7),
.tokens-table th:nth-child(8),
.tokens-table th:nth-child(9) {
  text-align: center;
}

.tokens-table td {
  padding: 12px 16px;
  border-bottom: 1px solid #f0f0f0;
  font-size: 14px;
  vertical-align: middle;
}

.tokens-table tbody tr:hover {
  background-color: #f8f9fa;
}

.token-display {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 28px;
}

.token-text {
  font-family: monospace;
  font-size: 13px;
  background: #f8f9fa;
  padding: 4px 8px;
  border-radius: 4px;
  color: #333;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.copy-btn {
  padding: 6px 10px;
  border: 1px solid #1976d2;
  background: transparent;
  color: #1976d2;
  cursor: pointer;
  border-radius: 4px;
  transition: all 0.2s ease;
  font-size: 12px;
  flex-shrink: 0;
}

.copy-btn:hover {
  background: #e3f2fd;
}

.status-badge {
  padding: 4px 10px;
  border-radius: 12px;
  font-size: 12px;
  font-weight: 500;
  text-align: center;
  min-width: 60px;
}

.status-active {
  background: #e8f5e8;
  color: #2e7d32;
}

.status-disabled {
  background: #fff3e0;
  color: #f57c00;
}

.status-expired {
  background: #ffebee;
  color: #d32f2f;
}

.status-cell,
.amount-cell,
.max-amount-cell,
.tokens-cell,
.models-cell,
.expires-cell,
.created-cell,
.actions-cell {
  text-align: center;
}

.amount-cell,
.max-amount-cell,
.tokens-cell {
  font-family: monospace;
  font-size: 13px;
}

.models-display {
  display: flex;
  align-items: center;
  justify-content: center;
}

.no-limit {
  color: #666;
  font-style: italic;
}

.models-count {
  color: #333;
  font-weight: 500;
}

.expires-cell,
.created-cell {
  color: #666;
  font-size: 13px;
}

.token-actions {
  display: flex;
  gap: 6px;
  align-items: center;
  justify-content: center;
  min-height: 36px;
  flex-wrap: nowrap;
}

.btn {
  padding: 5px 10px;
  font-size: 12px;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.2s ease;
  border: 1px solid;
  white-space: nowrap;
  min-width: 48px;
  font-weight: 500;
}

.btn-primary {
  background: #2196f3;
  color: #fff;
  border-color: #2196f3;
}

.btn-primary:hover:not(:disabled) {
  background: #1976d2;
  border-color: #1976d2;
}

.btn-secondary {
  background: #fff;
  color: #1976d2;
  border-color: #2196f3;
}

.btn-secondary:hover:not(:disabled) {
  background: #e3f2fd;
  border-color: #1976d2;
}

.btn-danger {
  background: #fff;
  color: #d32f2f;
  border-color: #f44336;
}

.btn-danger:hover:not(:disabled) {
  background: #ffebee;
  border-color: #d32f2f;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
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
  max-width: 600px;
  max-height: 85vh;
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
  max-height: calc(85vh - 80px);
  overflow-y: auto;
}

.token-info {
  padding: 16px;
  background: #f8f9fa;
  border-radius: 6px;
  margin-bottom: 20px;
  border: 1px solid #e0e0e0;
}

.token-info p {
  margin: 0 0 8px 0;
  font-size: 14px;
  color: #333;
}

.token-info p:last-child {
  margin-bottom: 0;
}

.token-info code {
  background: white;
  padding: 2px 6px;
  border-radius: 3px;
  font-family: monospace;
  font-size: 13px;
}

.token-info-line {
  margin: 12px 0 0;
  font-size: 14px;
  color: #333;
}

.token-form {
  display: flex;
  flex-direction: column;
  gap: 20px;
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.form-group > label {
  font-size: 14px;
  font-weight: 500;
  color: #333;
}

.form-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 20px;
}

.models-selector {
  border: 1px solid #e0e0e0;
  border-radius: 6px;
  background: #fafafa;
}

.no-models {
  padding: 20px;
  text-align: center;
  color: #666;
}

.models-checkboxes {
  max-height: 200px;
  overflow-y: auto;
  padding: 12px;
}

.model-checkbox {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  cursor: pointer;
  border-radius: 4px;
  transition: background 0.2s ease;
  font-size: 14px;
  color: #333;
}

.model-checkbox:hover {
  background: white;
}

.model-checkbox input {
  margin: 0;
}

.model-name {
  font-family: monospace;
  font-size: 13px;
}

.models-hint {
  padding: 8px 12px;
  background: white;
  border-top: 1px solid #e0e0e0;
  color: #666;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  font-size: 14px;
  color: #333;
  margin-top: 0;
}

.checkbox-label input {
  margin: 0;
}

.form-group input[type="number"],
.form-group input[type="datetime-local"] {
  padding: 10px 12px;
  border: 1px solid #ddd;
  border-radius: 4px;
  font-size: 14px;
  transition: border-color 0.2s ease;
}

.form-group input:focus {
  outline: none;
  border-color: #2196f3;
}

.form-group small {
  color: #666;
  font-size: 12px;
  margin-top: 4px;
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

/* 响应式设计 */
@media (max-width: 1200px) {
  .filters-bar {
    gap: 16px;
  }

  .search-section {
    min-width: 200px;
  }

  .filter-section,
  .sort-section {
    min-width: 120px;
  }
}

@media (max-width: 1024px) {
  .filters-bar {
    flex-direction: column;
    align-items: stretch;
    gap: 16px;
    padding: 20px;
  }

  .search-section {
    max-width: none;
    min-width: auto;
  }

  .filter-section,
  .sort-section {
    justify-content: space-between;
    min-width: auto;
  }

  .form-row {
    grid-template-columns: 1fr;
  }

  .tokens-table th,
  .tokens-table td {
    padding: 10px 8px;
  }
}

@media (max-width: 768px) {
  .token-manager {
    padding: 16px;
    gap: 16px;
  }

  .manager-header {
    flex-direction: column;
    align-items: stretch;
    gap: 16px;
  }

  .header-actions {
    flex-direction: column;
    gap: 12px;
  }

  .filters-bar {
    padding: 16px;
    gap: 12px;
  }

  .tokens-table th,
  .tokens-table td {
    padding: 8px 6px;
    font-size: 13px;
  }

  .token-display {
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    min-height: auto;
  }

  .token-text {
    font-size: 12px;
    width: 100%;
  }

  .copy-btn {
    align-self: flex-end;
  }

  .token-actions {
    flex-direction: column;
    gap: 6px;
    min-height: auto;
  }

  .edit-btn,
  .toggle-btn {
    font-size: 11px;
    padding: 5px 10px;
    min-width: 50px;
  }

  .modal-content {
    width: 95%;
    margin: 10px;
  }

  .modal-body {
    padding: 16px;
  }
}
</style>

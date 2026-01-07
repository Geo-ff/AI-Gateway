<script setup lang="ts">
import { onMounted, ref, computed, watch } from 'vue'
import { useNotify } from '../../composables/useNotify'
import { listTokens, type ClientToken } from '../../api/tokens'
import { fetchMetricsSummary, type MetricsSummary } from '../../api/metrics'

const tokens = ref<ClientToken[]>([])
const summary = ref<MetricsSummary | null>(null)
const loading = ref(false)
const error = ref<string | null>(null)
const message = ref<string | null>(null)
const sortBy = ref<'amount' | 'usage' | 'tokens' | 'prompt' | 'completion'>('amount')
const availableDates = ref<string[]>([])
const selectedStartDate = ref<string | null>(null)
const selectedEndDate = ref<string | null>(null)

const sortedTokens = computed(() => {
  if (!tokens.value.length) return []

  return [...tokens.value].sort((a, b) => {
    switch (sortBy.value) {
      case 'amount':
        return (b.amount_spent || 0) - (a.amount_spent || 0)
      case 'tokens':
        return (b.total_tokens_spent || 0) - (a.total_tokens_spent || 0)
      case 'usage':
        return (b.usage_count || 0) - (a.usage_count || 0)
      case 'prompt':
        return (b.prompt_tokens_spent || 0) - (a.prompt_tokens_spent || 0)
      case 'completion':
        return (b.completion_tokens_spent || 0) - (a.completion_tokens_spent || 0)
      default:
        return (b.amount_spent || 0) - (a.amount_spent || 0)
    }
  })
})

const { } = useNotify()

function ensureValidRange() {
  if (!availableDates.value.length) return
  const first = availableDates.value[0]
  const last = availableDates.value[availableDates.value.length - 1]
  if (!selectedStartDate.value || !availableDates.value.includes(selectedStartDate.value)) {
    selectedStartDate.value = summary.value?.start_date ?? first
  }
  if (!selectedEndDate.value || !availableDates.value.includes(selectedEndDate.value!)) {
    selectedEndDate.value = summary.value?.end_date ?? last
  }
  if (selectedStartDate.value && selectedEndDate.value) {
    const startIndex = availableDates.value.indexOf(selectedStartDate.value)
    const endIndex = availableDates.value.indexOf(selectedEndDate.value)
    if (startIndex > endIndex) {
      selectedEndDate.value = selectedStartDate.value
    }
  }
}

function applySummaryMeta(nextSummary: MetricsSummary) {
  availableDates.value = nextSummary.available_dates ?? []
  if (availableDates.value.length) {
    if (!selectedStartDate.value) {
      selectedStartDate.value = nextSummary.start_date ?? availableDates.value[0]
    }
    if (!selectedEndDate.value) {
      selectedEndDate.value = nextSummary.end_date ?? availableDates.value[availableDates.value.length - 1]
    }
    ensureValidRange()
  } else {
    selectedStartDate.value = null
    selectedEndDate.value = null
  }
}

async function loadData() {
  loading.value = true
  error.value = null
  try {
    const [tokenResp, summaryResp] = await Promise.all([
      listTokens(),
      fetchMetricsSummary({
        startDate: selectedStartDate.value ?? undefined,
        endDate: selectedEndDate.value ?? undefined,
        windowMinutes: 1440,
      }),
    ])
    tokens.value = tokenResp
    summary.value = summaryResp
    applySummaryMeta(summaryResp)
  } catch (err: any) {
    error.value = err?.message || String(err)
  } finally {
    loading.value = false
  }
}

function setSortBy(sortType: 'amount' | 'usage' | 'tokens' | 'prompt' | 'completion') {
  sortBy.value = sortType
}

onMounted(() => {
  loadData()
})

watch(selectedStartDate, () => {
  ensureValidRange()
})

watch(selectedEndDate, () => {
  ensureValidRange()
})
</script>

<template>
  <div class="usage-stats">
    <header>
      <h3>使用统计</h3>
      <div class="header-actions">
        <div v-if="availableDates.length" class="range-controls">
          <label class="range-item">
            <span>开始日期</span>
            <input
              type="date"
              v-model="selectedStartDate"
              :min="availableDates[0]"
              :max="availableDates[availableDates.length-1]"
            />
          </label>
          <span class="range-separator">至</span>
          <label class="range-item">
            <span>结束日期</span>
            <input
              type="date"
              v-model="selectedEndDate"
              :min="availableDates[0]"
              :max="availableDates[availableDates.length-1]"
            />
          </label>
        </div>
        <button @click="() => loadData()" :disabled="loading">{{ loading ? '加载中...' : '刷新' }}</button>
      </div>
    </header>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-if="loading">加载中...</div>

    <section v-if="summary && !loading" class="summary">
      <div class="card">
        <h4>
          {{ summary.start_date && summary.end_date
            ? `${summary.start_date} 至 ${summary.end_date} 请求总数`
            : '请求总数' }}
        </h4>
        <strong>{{ summary.total_requests }}</strong>
      </div>
      <div class="card">
        <h4>
          {{ summary.start_date && summary.end_date
            ? `${summary.start_date} 至 ${summary.end_date} 消费金额`
            : '消费金额' }}
        </h4>
        <strong>${{ summary.total_amount_spent.toFixed(2) }}</strong>
      </div>
      <div class="card">
        <h4>
          {{ summary.start_date && summary.end_date
            ? `${summary.start_date} 至 ${summary.end_date} Tokens 消耗`
            : 'Tokens 消耗' }}
        </h4>
        <strong>{{ summary.total_tokens.toLocaleString() }}</strong>
      </div>
    </section>

    <section v-if="tokens.length" class="tokens">
      <div class="tokens-header">
        <h4>令牌使用统计</h4>
        <div class="sort-controls">
          <button
            :class="{ active: sortBy === 'amount' }"
            @click="setSortBy('amount')"
          >
            按消费金额
          </button>
          <button
            :class="{ active: sortBy === 'usage' }"
            @click="setSortBy('usage')"
          >
            按使用次数
          </button>
          <button
            :class="{ active: sortBy === 'tokens' }"
            @click="setSortBy('tokens')"
          >
            按总使用量
          </button>
          <button
            :class="{ active: sortBy === 'prompt' }"
            @click="setSortBy('prompt')"
          >
            按Prompt使用量
          </button>
          <button
            :class="{ active: sortBy === 'completion' }"
            @click="setSortBy('completion')"
          >
            按Completion使用量
          </button>
        </div>
      </div>
      <div class="table-container">
        <table class="tokens-table">
          <thead>
            <tr>
              <th>Token</th>
              <th>总使用次数</th>
              <th>消费金额</th>
              <th>最大消费金额</th>
              <th>总Token使用量</th>
              <th>Prompt Token</th>
              <th>Completion Token</th>
              <th>状态</th>
              <th>创建时间</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="token in sortedTokens" :key="token.token">
              <td class="token-cell">
                <code>{{ token.token.substring(0, 8) }}...{{ token.token.substring(-8) }}</code>
              </td>
              <td class="usage-count">{{ (token.usage_count || 0).toLocaleString() }}</td>
              <td class="amount-cell">${{ token.amount_spent.toFixed(2) }}</td>
              <td class="max-amount-cell">{{ token.max_amount != null ? `$${token.max_amount.toFixed(2)}` : '—' }}</td>
              <td class="tokens-cell">{{ (token.total_tokens_spent || 0).toLocaleString() }}</td>
              <td class="prompt-tokens-cell">{{ (token.prompt_tokens_spent || 0).toLocaleString() }}</td>
              <td class="completion-tokens-cell">{{ (token.completion_tokens_spent || 0).toLocaleString() }}</td>
              <td class="status-cell">
                <span :class="['status-badge', token.enabled ? 'enabled' : 'disabled']">
                  {{ token.enabled ? '已启用' : '已禁用' }}
                </span>
              </td>
              <td class="created-cell">{{ new Date(token.created_at).toLocaleDateString() }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

  </div>
</template>

<style scoped>
.usage-stats {
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  padding: 24px;
  background: white;
  display: flex;
  flex-direction: column;
  gap: 24px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.04);
  height: calc(100vh - 168px);
  overflow: hidden;
}

header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: 16px;
  border-bottom: 1px solid #f0f0f0;
}

header h3 {
  font-size: 18px;
  font-weight: 600;
  color: #333;
  margin: 0;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 16px;
}

.range-controls {
  display: flex;
  align-items: center;
  gap: 16px;
}

.range-controls .range-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #333;
  font-weight: 500;
}

.range-controls input[type='date'] {
  padding: 8px 12px;
  border: 1px solid #ddd;
  border-radius: 6px;
  font-size: 13px;
  background: white;
}

.range-separator {
  font-size: 14px;
  color: #666;
  font-weight: 500;
}

.summary {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 20px;
}

.card {
  border: 1px solid #e8e8e8;
  border-radius: 8px;
  padding: 20px;
  background: #f8f9fa;
  text-align: center;
}

.card h4 {
  font-size: 14px;
  font-weight: 500;
  color: #666;
  margin: 0 0 8px 0;
}

.card strong {
  font-size: 24px;
  font-weight: 600;
  color: #333;
}

.tokens {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.tokens-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 16px;
  flex-wrap: wrap;
  gap: 12px;
}

.tokens-header h4 {
  font-size: 16px;
  font-weight: 600;
  color: #333;
  margin: 0;
}

.sort-controls {
  display: flex;
  gap: 8px;
}

.sort-controls button {
  padding: 8px 16px;
  border: 1px solid #ddd;
  border-radius: 4px;
  background: white;
  color: #333;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
}

.sort-controls button:hover {
  border-color: #999;
  background: #f8f9fa;
}

.sort-controls button.active {
  background: #2196f3;
  color: white;
  border-color: #2196f3;
}

.table-container {
  flex: 1;
  overflow: auto;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
}

.tokens-table {
  width: 100%;
  border-collapse: collapse;
  background: white;
}

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
.tokens-table th:nth-child(7) {
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

.token-cell code {
  font-family: monospace;
  font-size: 13px;
  background: #f8f9fa;
  padding: 2px 6px;
  border-radius: 4px;
  color: #333;
}

.amount-cell,
.max-amount-cell,
.tokens-cell,
.prompt-tokens-cell,
.completion-tokens-cell,
.usage-count,
.status-cell,
.created-cell {
  text-align: center;
}

.usage-count,
.amount-cell,
.max-amount-cell,
.tokens-cell,
.prompt-tokens-cell,
.completion-tokens-cell {
  font-family: monospace;
  font-size: 13px;
}

.status-badge {
  padding: 4px 10px;
  border-radius: 12px;
  font-size: 12px;
  font-weight: 500;
  text-align: center;
  min-width: 60px;
}

.status-badge.enabled {
  background: #e8f5e8;
  color: #2e7d32;
}

.status-badge.disabled {
  background: #fff3e0;
  color: #f57c00;
}

.created-cell {
  color: #666;
  font-size: 13px;
}

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

.error {
  color: #d32f2f;
  font-size: 14px;
  padding: 12px 16px;
  background: #ffebee;
  border: 1px solid #ffcdd2;
  border-radius: 6px;
}

@media (max-width: 1024px) {
  .tokens-header {
    flex-direction: column;
    align-items: stretch;
  }

  .sort-controls {
    justify-content: center;
  }
}

@media (max-width: 768px) {
  .usage-stats {
    padding: 16px;
  }

  .summary {
    grid-template-columns: 1fr;
  }

  .sort-controls {
    flex-direction: column;
  }

  .tokens-table th,
  .tokens-table td {
    padding: 8px 12px;
  }
}
</style>

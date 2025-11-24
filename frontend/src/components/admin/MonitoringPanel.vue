<script setup lang="ts">
import { computed, onMounted, ref, watch, shallowRef } from 'vue'
import type { RequestLogEntry } from '../../api/logs'
import { useMonitoringMetrics } from '../../composables/useMonitoringMetrics'
import { useLogs } from '../../composables/useLogs'
import { useResizeObserver } from '@vueuse/core'
import * as echarts from 'echarts'
import {
  formatAmount as _formatAmount,
  formatDateTime as _formatDateTime,
  formatDuration as _formatDuration,
  formatTokens as _formatTokens,
  statusClass as _statusClass,
  overviewStatusText as _overviewStatusText,
  formatModelWithProvider as _formatModelWithProvider,
} from '../../composables/useFormatters'

const {
  windowMinutes,
  intervalMinutes,
  startDate,
  endDate,
  availableDates,
  isDateRangeActive,
  summary,
  series,
  metricsLoading,
  metricsError,
  pieItems,
  pieData,
  pieTotal,
  seriesModelMap,
  loadMetrics,
} = useMonitoringMetrics()

const {
  overviewLogs,
  overviewLogsLoading,
  overviewLogsError,
  overviewLogsHasMore,
  loadOverviewLogs,
  requestLogs,
  requestLogsLoading,
  requestLogsError,
  requestLogsHasMore,
  loadRequestLogs,
} = useLogs()

const selectedView = ref<'overview' | 'series' | 'logs'>('overview')

// Chart Refs
const trendChartRef = ref<HTMLElement | null>(null)
const pieChartRef = ref<HTMLElement | null>(null)
const errorRateChartRef = ref<HTMLElement | null>(null)
const trendChartInstance = shallowRef<echarts.ECharts | null>(null)
const pieChartInstance = shallowRef<echarts.ECharts | null>(null)
const errorRateChartInstance = shallowRef<echarts.ECharts | null>(null)

// Resize Observer
useResizeObserver(trendChartRef, (entries) => {
  if (trendChartInstance.value) {
    trendChartInstance.value.resize()
  }
})

useResizeObserver(pieChartRef, (entries) => {
  if (pieChartInstance.value) {
    pieChartInstance.value.resize()
  }
})

useResizeObserver(errorRateChartRef, (entries) => {
  if (errorRateChartInstance.value) {
    errorRateChartInstance.value.resize()
  }
})

// Init Charts
function initTrendChart() {
  if (!trendChartRef.value) return
  if (trendChartInstance.value) {
    trendChartInstance.value.dispose()
  }
  
  trendChartInstance.value = echarts.init(trendChartRef.value)
  updateTrendChart()
}

function initPieChart() {
  if (!pieChartRef.value) return
  if (pieChartInstance.value) {
    pieChartInstance.value.dispose()
  }
  pieChartInstance.value = echarts.init(pieChartRef.value)
  updatePieChart()
}

function initErrorRateChart() {
  if (!errorRateChartRef.value) return
  if (errorRateChartInstance.value) {
    errorRateChartInstance.value.dispose()
  }
  errorRateChartInstance.value = echarts.init(errorRateChartRef.value)
  updateErrorRateChart()
}

function updateTrendChart() {
  if (!trendChartInstance.value || !series.value) return

  const points = series.value.points || []
  
  const dates = points.map(p => {
    const d = new Date(p.bucket_start)
    // Use simplified time format
    return d.toLocaleString('zh-CN', { 
      month: '2-digit', 
      day: '2-digit', 
      hour: '2-digit', 
      minute: '2-digit' 
    })
  })
  const requests = points.map(p => p.requests)
  const tokens = points.map(p => p.total_tokens || 0)

  const option: echarts.EChartsOption = {
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'cross', label: { backgroundColor: '#6a7985' } },
      backgroundColor: 'rgba(255, 255, 255, 0.95)',
      borderColor: '#e5e7eb',
      borderWidth: 1,
      textStyle: { color: '#374151', fontSize: 12 },
      padding: [10, 14],
      extraCssText: 'box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06); border-radius: 8px;'
    },
    legend: {
      data: ['请求数', 'Token'],
      bottom: 0,
      icon: 'circle',
      itemGap: 20,
      textStyle: { color: '#6b7280' }
    },
    grid: {
      left: '2%',
      right: '2%',
      bottom: '10%',
      top: '10%',
      containLabel: true
    },
    xAxis: [
      {
        type: 'category',
        boundaryGap: false,
        data: dates,
        axisLine: { lineStyle: { color: '#e5e7eb' } },
        axisLabel: { color: '#6b7280', fontSize: 11 },
        axisTick: { show: false }
      }
    ],
    yAxis: [
      {
        type: 'value',
        name: '请求数',
        position: 'left',
        alignTicks: true,
        axisLine: { show: false },
        axisLabel: { color: '#6b7280' },
        splitLine: { lineStyle: { color: '#f3f4f6' } }
      },
      {
        type: 'value',
        name: 'Token',
        position: 'right',
        alignTicks: true,
        axisLine: { show: false },
        axisLabel: { 
          color: '#6b7280',
          formatter: (value: number) => value >= 1000 ? (value/1000).toFixed(1) + 'k' : String(value)
        },
        splitLine: { show: false }
      }
    ],
    series: [
      {
        name: '请求数',
        type: 'line',
        smooth: true,
        showSymbol: false,
        symbolSize: 8,
        lineStyle: { width: 3, color: '#4f46e5' }, // Indigo 600
        itemStyle: { color: '#4f46e5' },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(79, 70, 229, 0.2)' },
            { offset: 1, color: 'rgba(79, 70, 229, 0.01)' }
          ])
        },
        data: requests
      },
      {
        name: 'Token',
        type: 'line',
        yAxisIndex: 1,
        smooth: true,
        showSymbol: false,
        symbolSize: 8,
        lineStyle: { width: 3, color: '#10b981' }, // Emerald 500
        itemStyle: { color: '#10b981' },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(16, 185, 129, 0.2)' },
            { offset: 1, color: 'rgba(16, 185, 129, 0.01)' }
          ])
        },
        data: tokens
      }
    ]
  }

  trendChartInstance.value.setOption(option)
}

function updatePieChart() {
  if (!pieChartInstance.value || !pieData.value.length) return

  const data = pieData.value.map(item => ({
    name: item.label,
    value: item.value
  }))

  const option: echarts.EChartsOption = {
    tooltip: {
      trigger: 'item',
      backgroundColor: 'rgba(255, 255, 255, 0.95)',
      borderColor: '#e5e7eb',
      borderWidth: 1,
      textStyle: { color: '#374151', fontSize: 12 },
      extraCssText: 'box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06); border-radius: 8px;'
    },
    legend: {
      orient: 'vertical',
      right: '5%',
      top: 'center',
      icon: 'circle',
      itemGap: 12,
      itemWidth: 10,
      itemHeight: 10,
      textStyle: { color: '#6b7280', fontSize: 11 },
      formatter: (name: string) => {
        const maxLen = 20
        return name.length > maxLen ? name.substring(0, maxLen) + '...' : name
      }
    },
    series: [
      {
        name: '模型分布',
        type: 'pie',
        radius: ['45%', '65%'],
        center: ['35%', '50%'],
        avoidLabelOverlap: true,
        itemStyle: {
          borderRadius: 8,
          borderColor: '#fff',
          borderWidth: 2
        },
        label: {
          show: false,
          position: 'center'
        },
        emphasis: {
          label: {
            show: true,
            fontSize: 14,
            fontWeight: 'bold',
            color: '#374151',
            formatter: '{b}\n{d}%'
          },
          itemStyle: {
            shadowBlur: 10,
            shadowOffsetX: 0,
            shadowColor: 'rgba(0, 0, 0, 0.1)'
          },
          scale: true,
          scaleSize: 8
        },
        labelLine: {
          show: false
        },
        data: data
      }
    ]
  }

  pieChartInstance.value.setOption(option)
}

function updateErrorRateChart() {
  if (!errorRateChartInstance.value || !series.value) return

  const points = series.value.points || []
  
  const dates = points.map(p => {
    const d = new Date(p.bucket_start)
    return d.toLocaleString('zh-CN', { 
      month: '2-digit', 
      day: '2-digit', 
      hour: '2-digit', 
      minute: '2-digit' 
    })
  })
  
  const errorRates = points.map(p => {
    const total = p.requests || 0
    const errors = p.errors || 0
    return total > 0 ? ((errors / total) * 100).toFixed(2) : 0
  })

  const option: echarts.EChartsOption = {
    tooltip: {
      trigger: 'axis',
      backgroundColor: 'rgba(255, 255, 255, 0.95)',
      borderColor: '#e5e7eb',
      borderWidth: 1,
      textStyle: { color: '#374151', fontSize: 12 },
      padding: [10, 14],
      extraCssText: 'box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.1); border-radius: 8px;',
      formatter: (params: any) => {
        const param = params[0]
        return `${param.axisValue}<br/>错误率: ${param.value}%`
      }
    },
    grid: {
      left: '8%',
      right: '5%',
      bottom: '15%',
      top: '10%',
      containLabel: true
    },
    xAxis: {
      type: 'category',
      boundaryGap: false,
      data: dates,
      axisLine: { lineStyle: { color: '#e5e7eb' } },
      axisLabel: { color: '#6b7280', fontSize: 11, rotate: 30 },
      axisTick: { show: false }
    },
    yAxis: {
      type: 'value',
      name: '错误率 (%)',
      axisLine: { show: false },
      axisLabel: { color: '#6b7280', formatter: '{value}%' },
      splitLine: { lineStyle: { color: '#f3f4f6' } }
    },
    series: [
      {
        name: '错误率',
        type: 'line',
        smooth: true,
        showSymbol: true,
        symbolSize: 6,
        lineStyle: { width: 2, color: '#ef4444' },
        itemStyle: { color: '#ef4444' },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(239, 68, 68, 0.2)' },
            { offset: 1, color: 'rgba(239, 68, 68, 0.01)' }
          ])
        },
        data: errorRates
      }
    ]
  }

  errorRateChartInstance.value.setOption(option)
}

watch(series, () => {
  if (selectedView.value === 'series') {
    // Slight delay to ensure DOM is ready if switching views
    setTimeout(updateTrendChart, 50)
    setTimeout(updatePieChart, 50)
    setTimeout(updateErrorRateChart, 50)
  }
})

watch(pieData, () => {
  if (selectedView.value === 'series') {
    setTimeout(updatePieChart, 50)
  }
})

watch(selectedView, async (view) => {
  if (view === 'series') {
    // Wait for DOM update
    setTimeout(() => {
      initTrendChart()
      initPieChart()
      initErrorRateChart()
    }, 100)
  } else if (view === 'logs' && !requestLogs.value.length && !requestLogsLoading.value) {
    loadRequestLogs(true)
  } else if (view === 'overview' && !overviewLogs.value.length && !overviewLogsLoading.value) {
    loadOverviewLogs(true)
  }
})

// Pagination for overview logs
const recentCount = ref(50)
const overviewCurrentPage = ref(1)
const overviewPageSize = ref(50)
const overviewTotalPages = computed(() => Math.ceil(overviewLogs.value.length / overviewPageSize.value))
const recentRequests = computed(() => {
  const start = (overviewCurrentPage.value - 1) * overviewPageSize.value
  const end = start + overviewPageSize.value
  return overviewLogs.value.slice(start, end)
})

function handleOverviewPageChange(page: number) {
  overviewCurrentPage.value = page
}

function handleOverviewSizeChange(size: number) {
  overviewPageSize.value = size
  overviewCurrentPage.value = 1
}

// Pagination for request logs
const logsCurrentPage = ref(1)
const logsPageSize = ref(50)
const logsTotalPages = computed(() => Math.ceil(requestLogs.value.length / logsPageSize.value))
const paginatedRequestLogs = computed(() => {
  const start = (logsCurrentPage.value - 1) * logsPageSize.value
  const end = start + logsPageSize.value
  return requestLogs.value.slice(start, end)
})

function handleLogsPageChange(page: number) {
  logsCurrentPage.value = page
}

function handleLogsSizeChange(size: number) {
  logsPageSize.value = size
  logsCurrentPage.value = 1
}

const providerTopList = computed(() => summary.value?.top_providers ?? [])
const modelTopList = computed(() => summary.value?.top_models ?? [])

onMounted(async () => {
  await loadMetrics()
  await loadOverviewLogs(true)
})

const formatDateTime = _formatDateTime
const formatDuration = _formatDuration
const formatTokens = _formatTokens
const formatAmount = _formatAmount
function statusClass(log: RequestLogEntry): string {
  return _statusClass(log.success)
}
function overviewStatusText(log: RequestLogEntry): string {
  return _overviewStatusText(log.success, log.error_message)
}
function formatModelWithProvider(log: RequestLogEntry): string {
  return _formatModelWithProvider(log.model, log.provider)
}
function formatErrorRate(sum: any) {
  if (!sum) return '0%'
  return `${(sum.error_rate * 100).toFixed(2)}%`
}

function formatLatencyInSeconds(ms: number): string {
  return (ms / 1000).toFixed(3)
}

function truncateMiddle(str: string, maxLen: number = 16): string {
  if (!str || str.length <= maxLen) return str
  const half = Math.floor(maxLen / 2) - 2
  return `${str.substring(0, half)}...${str.substring(str.length - half)}`
}

function loadMoreRequestLogs() {
  if (!requestLogsHasMore.value || requestLogsLoading.value) return
  loadRequestLogs(false)
}

function refreshLogs() {
  loadRequestLogs(true)
}

function goToLogs() {
  selectedView.value = 'logs'
}

function loadMoreOverviewLogs() {
  if (!overviewLogsHasMore.value || overviewLogsLoading.value) return
  loadOverviewLogs(false)
}

function refreshOverviewLogs() {
  overviewCurrentPage.value = 1
  loadOverviewLogs(true)
}

function refreshRequestLogs() {
  logsCurrentPage.value = 1
  loadRequestLogs(true)
}
</script>

<template>
  <div class="monitoring-panel">
    <header class="monitoring-header">
      <div class="title-section">
        <h3>实时监控</h3>
        <p v-if="summary" class="last-updated">最后更新：{{ formatDateTime(summary.generated_at) }}</p>
      </div>

      <div class="controls-section">
        <div class="view-tabs">
          <button :class="{ active: selectedView === 'overview' }" @click="selectedView = 'overview'">概览</button>
          <button :class="{ active: selectedView === 'series' }" @click="selectedView = 'series'">时间序列</button>
          <button :class="{ active: selectedView === 'logs' }" @click="selectedView = 'logs'">请求日志</button>
        </div>
      </div>
    </header>

    <p v-if="metricsError" class="error">{{ metricsError }}</p>

    <div v-if="selectedView === 'overview'" class="overview-content">
      <section class="overview-filters" v-if="availableDates.length">
        <div class="control-group">
          <label>开始日期</label>
          <input type="date" v-model="startDate" :min="availableDates[0]" :max="availableDates[availableDates.length-1]" />
        </div>
        <div class="control-group">
          <label>结束日期</label>
          <input type="date" v-model="endDate" :min="availableDates[0]" :max="availableDates[availableDates.length-1]" />
        </div>
        <button class="refresh-btn" @click="loadMetrics" :disabled="metricsLoading">
          {{ metricsLoading ? '加载中...' : '应用/刷新' }}
        </button>
      </section>
      <div v-if="metricsLoading && !summary" class="loading">加载监控数据中...</div>
      <template v-else>
        <section class="metrics-summary" v-if="summary">
          <div class="metric-card">
            <div class="metric-header">
              <h4>请求总数</h4>
            </div>
            <div class="metric-value">{{ summary.total_requests.toLocaleString() }}</div>
            <div class="metric-details badges">
              <span class="badge success">成功：{{ summary.success_requests.toLocaleString() }}</span>
              <span class="badge error">失败：{{ summary.error_requests.toLocaleString() }}</span>
            </div>
            <div v-if="summary.start_date && summary.end_date" class="metric-range">
              {{ summary.start_date }} 至 {{ summary.end_date }}
            </div>
          </div>

          <div class="metric-card">
            <div class="metric-header">
              <h4>错误率</h4>
            </div>
            <div class="metric-value">{{ formatErrorRate(summary) }}</div>
            <div class="metric-details single">
              <template v-if="summary.start_date && summary.end_date">
                范围：{{ summary.start_date }} 至 {{ summary.end_date }}
              </template>
              <template v-else>
                窗口：{{ summary.window_minutes }} 分钟
              </template>
            </div>
          </div>

          <div class="metric-card">
            <div class="metric-header">
              <h4>平均响应时间</h4>
            </div>
            <div class="metric-value">{{ formatLatencyInSeconds(summary.average_latency_ms) }} s</div>
          </div>

          <div class="metric-card">
            <div class="metric-header">
              <h4>消费金额</h4>
            </div>
            <div class="metric-value">${{ summary.total_amount_spent.toFixed(4) }}</div>
            <div class="metric-details badges">
              <span class="badge neutral">总 Token：{{ summary.total_tokens.toLocaleString() }}</span>
              <span class="badge neutral">唯一令牌：{{ summary.unique_clients }}</span>
            </div>
          </div>
        </section>

        <section class="top-lists" v-if="providerTopList.length || modelTopList.length">
          <div v-if="providerTopList.length" class="top-list">
            <h4>热门供应商</h4>
            <ul>
              <li v-for="provider in providerTopList" :key="provider.name">
                <span class="label">{{ provider.name }}</span>
                <span class="value">{{ provider.count.toLocaleString() }}</span>
              </li>
            </ul>
          </div>
          <div v-if="modelTopList.length" class="top-list">
            <h4>热门模型</h4>
            <ul>
              <li v-for="model in modelTopList" :key="model.name">
                <span class="label">{{ model.name }}</span>
                <span class="value">{{ model.count.toLocaleString() }}</span>
              </li>
            </ul>
          </div>
        </section>

        <section class="recent-requests">
          <div class="section-header">
            <h4>最近请求</h4>
            <div class="section-actions">
              <label class="inline-label">每页显示</label>
              <select class="page-size-select" v-model.number="overviewPageSize" @change="handleOverviewSizeChange(overviewPageSize)">
                <option :value="50">50 条</option>
                <option :value="100">100 条</option>
                <option :value="150">150 条</option>
                <option :value="200">200 条</option>
              </select>
              <button class="action-btn" @click="refreshOverviewLogs" :disabled="overviewLogsLoading">
                {{ overviewLogsLoading ? '加载中...' : '刷新' }}
              </button>
              <button class="action-btn secondary" @click="loadMoreOverviewLogs" :disabled="overviewLogsLoading || !overviewLogsHasMore">
                {{ overviewLogsLoading ? '加载中...' : overviewLogsHasMore ? '加载更多' : '没有更多数据' }}
              </button>
              <button class="link-btn" @click="goToLogs">查看全部请求</button>
            </div>
          </div>
          <p v-if="overviewLogsError" class="error">{{ overviewLogsError }}</p>
          <div v-if="!recentRequests.length && !overviewLogsLoading" class="empty-placeholder">
            暂无请求数据
          </div>
          <div v-else class="table-wrapper">
            <table>
              <thead>
                <tr>
                  <th>请求时间</th>
                  <th>请求令牌</th>
                  <th>请求模型</th>
                  <th>耗时</th>
                  <th>Prompt Token</th>
                  <th>Completion Token</th>
                  <th>总 Token</th>
                  <th>消费</th>
                  <th>结果</th>
                  <th>使用的 API Key</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="log in recentRequests" :key="`${log.id}-${log.timestamp}`">
                  <td class="time-cell">{{ formatDateTime(log.timestamp) }}</td>
                  <td class="token-cell" :title="log.client_token || '—'">{{ truncateMiddle(log.client_token || '—', 16) }}</td>
                  <td class="model-cell">{{ formatModelWithProvider(log) }}</td>
                  <td class="duration-cell">{{ formatDuration(log.response_time_ms) }}</td>
                  <td class="number-cell">{{ formatTokens(log.prompt_tokens) }}</td>
                  <td class="number-cell">{{ formatTokens(log.completion_tokens) }}</td>
                  <td class="number-cell">{{ formatTokens(log.total_tokens) }}</td>
                  <td class="amount-cell">{{ formatAmount(log.amount_spent) }}</td>
                  <td class="status-cell">
                    <span 
                      :class="['status-tag', statusClass(log), { 'has-tooltip': !log.success && log.error_message }]"
                      :data-tooltip="log.error_message || ''"
                    >
                      {{ overviewStatusText(log) }}
                    </span>
                  </td>
                  <td class="api-key-cell" :title="log.api_key || '—'">{{ truncateMiddle(log.api_key || '—', 16) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
          <div v-if="overviewLogs.length > 0" class="pagination-wrapper">
            <div class="pagination-info">
              共 {{ overviewLogs.length }} 条，第 {{ overviewCurrentPage }} / {{ overviewTotalPages }} 页
            </div>
            <div class="pagination-controls">
              <button 
                class="page-btn" 
                @click="handleOverviewPageChange(1)" 
                :disabled="overviewCurrentPage === 1"
              >
                首页
              </button>
              <button 
                class="page-btn" 
                @click="handleOverviewPageChange(overviewCurrentPage - 1)" 
                :disabled="overviewCurrentPage === 1"
              >
                上一页
              </button>
              <span class="page-numbers">
                <button
                  v-for="page in Math.min(5, overviewTotalPages)"
                  :key="page"
                  class="page-num"
                  :class="{ active: page === overviewCurrentPage }"
                  @click="handleOverviewPageChange(page)"
                >
                  {{ page }}
                </button>
              </span>
              <button 
                class="page-btn" 
                @click="handleOverviewPageChange(overviewCurrentPage + 1)" 
                :disabled="overviewCurrentPage === overviewTotalPages"
              >
                下一页
              </button>
              <button 
                class="page-btn" 
                @click="handleOverviewPageChange(overviewTotalPages)" 
                :disabled="overviewCurrentPage === overviewTotalPages"
              >
                末页
              </button>
            </div>
          </div>
        </section>
      </template>
    </div>

    <div v-else-if="selectedView === 'series'" class="series-view">
      <section class="series-filters">
        <div class="control-group">
          <label>开始日期</label>
          <input type="date" v-model="startDate" :min="availableDates[0]" :max="availableDates[availableDates.length-1]" />
        </div>
        <div class="control-group">
          <label>结束日期</label>
          <input type="date" v-model="endDate" :min="availableDates[0]" :max="availableDates[availableDates.length-1]" />
        </div>
        <button class="refresh-btn" @click="loadMetrics" :disabled="metricsLoading">
          {{ metricsLoading ? '加载中...' : '应用/刷新' }}
        </button>
      </section>
      <div v-if="metricsLoading && !series" class="loading">加载时间序列数据中...</div>
      <template v-else>
        <section class="chart-row">
          <div class="chart-card full-width">
            <div class="chart-header">
              <h4>请求 &amp; Token 趋势</h4>
            </div>
            <div class="echart-container" ref="trendChartRef"></div>
          </div>
        </section>
        <section class="chart-row">
          <div class="chart-card half-width">
            <div class="chart-header">
              <h4>模型分布</h4>
            </div>
            <div class="echart-container pie-container" ref="pieChartRef"></div>
          </div>
          <div class="chart-card half-width">
            <div class="chart-header">
              <h4>错误率趋势</h4>
            </div>
            <div class="echart-container pie-container" ref="errorRateChartRef"></div>
          </div>
        </section>
      </template>
    </div>

    <div v-else-if="selectedView === 'logs'" class="logs-view">
      <div class="logs-actions">
        <div class="logs-controls">
          <label class="inline-label">每页显示</label>
          <select class="page-size-select" v-model.number="logsPageSize" @change="handleLogsSizeChange(logsPageSize)">
            <option :value="50">50 条</option>
            <option :value="100">100 条</option>
            <option :value="150">150 条</option>
            <option :value="200">200 条</option>
          </select>
        </div>
        <div class="logs-actions-right">
          <button class="action-btn secondary" @click="loadMoreRequestLogs" :disabled="requestLogsLoading || !requestLogsHasMore">
            {{ requestLogsLoading ? '加载中...' : requestLogsHasMore ? '加载更多' : '没有更多数据' }}
          </button>
          <button class="action-btn" @click="refreshRequestLogs" :disabled="requestLogsLoading">
            {{ requestLogsLoading ? '加载中...' : '刷新' }}
          </button>
        </div>
      </div>

      <p v-if="requestLogsError" class="error">{{ requestLogsError }}</p>
      <div v-if="!requestLogs.length && !requestLogsLoading" class="empty-placeholder">
        暂无请求日志
      </div>
      <div v-else class="table-wrapper logs-table-wrapper">
        <table class="logs-table">
          <thead>
            <tr>
              <th>时间</th>
              <th>方法</th>
              <th>路径</th>
              <th>模型</th>
              <th>令牌</th>
              <th>状态码</th>
              <th>耗时</th>
              <th>Prompt</th>
              <th>Completion</th>
              <th>总 Token</th>
              <th>消费金额</th>
              <th>错误信息</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="log in paginatedRequestLogs" :key="`${log.id}-${log.timestamp}`">
              <td>{{ formatDateTime(log.timestamp) }}</td>
              <td>{{ log.method }}</td>
              <td class="path-cell">{{ log.path }}</td>
              <td>{{ formatModelWithProvider(log) }}</td>
              <td class="token-cell" :title="log.client_token || '—'">{{ truncateMiddle(log.client_token || '—', 12) }}</td>
              <td :class="log.success ? 'status-success' : 'status-error'">{{ log.status_code }}</td>
              <td>{{ formatDuration(log.response_time_ms) }}</td>
              <td>{{ formatTokens(log.prompt_tokens) }}</td>
              <td>{{ formatTokens(log.completion_tokens) }}</td>
              <td>{{ formatTokens(log.total_tokens) }}</td>
              <td>{{ formatAmount(log.amount_spent) }}</td>
              <td class="error-text">{{ log.error_message || '—' }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-if="requestLogs.length > 0" class="pagination-wrapper">
        <div class="pagination-info">
          共 {{ requestLogs.length }} 条，第 {{ logsCurrentPage }} / {{ logsTotalPages }} 页
        </div>
        <div class="pagination-controls">
          <button 
            class="page-btn" 
            @click="handleLogsPageChange(1)" 
            :disabled="logsCurrentPage === 1"
          >
            首页
          </button>
          <button 
            class="page-btn" 
            @click="handleLogsPageChange(logsCurrentPage - 1)" 
            :disabled="logsCurrentPage === 1"
          >
            上一页
          </button>
          <span class="page-numbers">
            <button
              v-for="page in Math.min(5, logsTotalPages)"
              :key="page"
              class="page-num"
              :class="{ active: page === logsCurrentPage }"
              @click="handleLogsPageChange(page)"
            >
              {{ page }}
            </button>
          </span>
          <button 
            class="page-btn" 
            @click="handleLogsPageChange(logsCurrentPage + 1)" 
            :disabled="logsCurrentPage === logsTotalPages"
          >
            下一页
          </button>
          <button 
            class="page-btn" 
            @click="handleLogsPageChange(logsTotalPages)" 
            :disabled="logsCurrentPage === logsTotalPages"
          >
            末页
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.monitoring-panel {
  display: flex;
  flex-direction: column;
  gap: 24px;
  padding: 24px;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  background: #fff;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.04);
  min-height: calc(100vh - 168px);
  height: auto;
  overflow: visible;
}

.monitoring-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 16px;
  padding-bottom: 16px;
  border-bottom: 1px solid #f0f0f0;
}

.title-section h3 {
  margin: 0;
  font-size: 18px;
  font-weight: 600;
  color: #333;
}

.last-updated {
  margin-top: 4px;
  font-size: 13px;
  color: #666;
}

.controls-section {
  display: flex;
  flex-direction: column;
  gap: 16px;
  align-items: flex-end;
}

.view-tabs {
  display: flex;
  padding: 4px;
  border-radius: 6px;
  background: #f8f9fa;
  gap: 4px;
}

.view-tabs button {
  padding: 8px 16px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: #666;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
}

.view-tabs button.active {
  background: #fff;
  color: #1976d2;
  box-shadow: 0 1px 4px rgba(25, 118, 210, 0.18);
}

.control-group {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #333;
}

.control-group label {
  white-space: nowrap;
}

.control-group input {
  padding: 8px 10px;
  border: 1px solid #d0d5dd;
  border-radius: 6px;
  width: 140px;
  font-size: 13px;
}

.refresh-btn {
  padding: 8px 16px;
  border: 1px solid #1976d2;
  border-radius: 6px;
  background: #1976d2;
  color: #fff;
  font-size: 13px;
  cursor: pointer;
  transition: background 0.2s ease;
}

.refresh-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.refresh-btn:not(:disabled):hover {
  background: #1458a6;
}

.error {
  padding: 10px 14px;
  border-radius: 6px;
  border: 1px solid #f9caca;
  background: #fdecec;
  color: #d93025;
  font-size: 13px;
}

.loading {
  padding: 40px 0;
  text-align: center;
  color: #666;
}

/* Metrics Summary Cards */
.metrics-summary {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 16px;
  margin-bottom: 8px;
}

.metric-card {
  border: 1px solid #e7eaf1;
  border-radius: 10px;
  padding: 16px 18px;
  background: linear-gradient(180deg, #ffffff 0%, #fafafb 100%);
  display: flex;
  flex-direction: column;
  gap: 10px;
  transition: all 0.2s ease;
}

.metric-card:hover {
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
  transform: translateY(-2px);
}

.metric-header h4 {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: #344054;
}

.metric-value {
  font-size: 26px;
  font-weight: 600;
  color: #1f2937;
  line-height: 1.2;
}

.metric-details {
  display: flex;
  gap: 12px;
  font-size: 13px;
  color: #4b5563;
}

.badge {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 500;
  line-height: 1.6;
}

.badge.success { background: rgba(46,125,50,.12); color:#2e7d32; }
.badge.error { background: rgba(211,47,47,.12); color:#d32f2f; }
.badge.neutral { background: #f3f4f6; color:#374151; }

.metric-range {
  font-size: 12px;
  color: #6b7280;
}

.overview-filters {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  padding: 12px 16px;
  background: #f9fafb;
  border-radius: 8px;
  border: 1px solid #e5e7eb;
  margin-bottom: 16px;
}

/* Top Lists */
.top-lists {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
  gap: 16px;
  margin-bottom: 8px;
}

.top-list {
  border: 1px solid #e5e7eb;
  border-radius: 10px;
  padding: 16px 18px;
  background: #fafafa;
  transition: all 0.2s ease;
}

.top-list:hover {
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.06);
}

.top-list h4 {
  margin: 0 0 12px;
  font-size: 15px;
  font-weight: 600;
  color: #1f2937;
}

.top-list ul {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.top-list li {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 12px;
  border-radius: 6px;
  background: #fff;
  border: 1px solid #e5e7eb;
}

.top-list .label {
  font-size: 13px;
  color: #374151;
  font-weight: 500;
  max-width: 70%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.top-list .value {
  font-family: monospace;
  color: #111827;
}

/* Series View */
.series-view {
  display: flex;
  flex-direction: column;
  gap: 24px;
}

.series-filters {
  display: flex;
  align-items: flex-end;
  gap: 12px;
  flex-wrap: wrap;
}

.chart-row {
  display: flex;
  gap: 24px;
  flex-wrap: wrap;
}

.chart-card {
  background: #fff;
  border: 1px solid #e5e7eb;
  border-radius: 10px;
  padding: 20px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.05);
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 300px;
}

.chart-card.full-width {
  width: 100%;
}

.chart-card.half-width {
  max-width: 600px;
}

.chart-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 16px;
}

.chart-header h4 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: #374151;
}

.echart-container {
  width: 100%;
  height: 350px;
}

.pie-container {
  height: 300px;
}

.empty-placeholder {
  text-align: center;
  padding: 60px 0;
  color: #9ca3af;
  font-style: italic;
}

/* Logs View */
.logs-view {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.logs-actions {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 12px 16px;
  background: #f9fafb;
  border-radius: 8px;
  border: 1px solid #e5e7eb;
  flex-wrap: wrap;
  gap: 12px;
}

.logs-controls {
  display: flex;
  align-items: center;
  gap: 8px;
}

.logs-actions-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.table-wrapper {
  overflow-x: auto;
  border: 1px solid #e5e7eb;
  border-radius: 8px;
}

table {
  width: 100%;
  border-collapse: collapse;
  background: white;
}

th {
  background: #f9fafb;
  padding: 10px 12px;
  text-align: center;
  font-weight: 600;
  color: #4b5563;
  font-size: 12px;
  white-space: nowrap;
  border-bottom: 2px solid #e5e7eb;
  position: sticky;
  top: 0;
  z-index: 10;
}

th:first-child {
  text-align: left;
}

td {
  padding: 10px 12px;
  border-bottom: 1px solid #f3f4f6;
  font-size: 13px;
  color: #374151;
  white-space: nowrap;
  text-align: center;
}

td:first-child {
  text-align: left;
}

/* Table Cell Specific Widths */
.time-cell {
  min-width: 150px;
  width: 150px;
}

.token-cell {
  min-width: 120px;
  width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: 'Courier New', monospace;
  font-size: 12px;
  color: #6b7280;
}

.model-cell {
  min-width: 180px;
  width: 180px;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.duration-cell {
  min-width: 70px;
  width: 70px;
}

.number-cell {
  font-family: 'Courier New', monospace;
  font-size: 12px;
  min-width: 100px;
  width: 100px;
}

.status-cell {
  min-width: 120px;
  width: 120px;
}

.amount-cell {
  min-width: 90px;
  width: 90px;
  font-family: 'Courier New', monospace;
  font-size: 12px;
  color: #059669;
  font-weight: 500;
}

.api-key-cell {
  min-width: 120px;
  width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: 'Courier New', monospace;
  font-size: 12px;
  color: #6b7280;
}

.path-cell {
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.status-tag {
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 12px;
  font-weight: 500;
  position: relative;
  cursor: default;
}

.status-success { color: #059669; background: #ecfdf5; }
.status-error { color: #dc2626; background: #fef2f2; }

/* Tooltip styles */
.status-tag.has-tooltip {
  cursor: help;
}

.status-tag.has-tooltip::after {
  content: attr(data-tooltip);
  position: absolute;
  bottom: 100%;
  left: 50%;
  transform: translateX(-50%) translateY(-8px);
  padding: 8px 12px;
  background: rgba(0, 0, 0, 0.9);
  color: white;
  font-size: 12px;
  font-weight: 400;
  border-radius: 6px;
  white-space: normal;
  max-width: 300px;
  width: max-content;
  text-align: left;
  line-height: 1.4;
  opacity: 0;
  visibility: hidden;
  transition: opacity 0.2s ease, visibility 0.2s ease, transform 0.2s ease;
  pointer-events: none;
  z-index: 1000;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
}

.status-tag.has-tooltip::before {
  content: '';
  position: absolute;
  bottom: 100%;
  left: 50%;
  transform: translateX(-50%) translateY(-2px);
  border: 6px solid transparent;
  border-top-color: rgba(0, 0, 0, 0.9);
  opacity: 0;
  visibility: hidden;
  transition: opacity 0.2s ease, visibility 0.2s ease, transform 0.2s ease;
  pointer-events: none;
  z-index: 1001;
}

.status-tag.has-tooltip:hover::after,
.status-tag.has-tooltip:hover::before {
  opacity: 1;
  visibility: visible;
  transform: translateX(-50%) translateY(-4px);
}

.status-tag.has-tooltip:hover::before {
  transform: translateX(-50%) translateY(2px);
}

.error-text {
  color: #dc2626;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pagination-controls {
  display: flex;
  justify-content: center;
  padding-top: 16px;
}

.ghost {
  background: transparent;
  border: 1px solid #e5e7eb;
  color: #6b7280;
  padding: 6px 12px;
  border-radius: 6px;
  font-size: 13px;
  cursor: pointer;
  transition: all 0.2s;
}

.ghost:hover:not(:disabled) {
  border-color: #d1d5db;
  color: #374151;
  background: #f9fafb;
}

.ghost:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* Recent Requests Section */
.recent-requests {
  display: flex;
  flex-direction: column;
  gap: 16px;
  margin-top: 8px;
}

.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.section-header h4 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: #374151;
}

.section-actions {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.inline-label {
  font-size: 13px;
  color: #6b7280;
}

.small-input {
  width: 60px;
  padding: 4px 8px;
  border: 1px solid #d1d5db;
  border-radius: 4px;
  font-size: 13px;
}

.link-btn {
  background: #fff;
  border: 1px solid #4f46e5;
  color: #4f46e5;
  font-size: 13px;
  cursor: pointer;
  padding: 6px 14px;
  border-radius: 6px;
  transition: all 0.2s ease;
  font-weight: 500;
  white-space: nowrap;
}

.link-btn:hover {
  background: #eef2ff;
  color: #4338ca;
  border-color: #4338ca;
}

/* Action Buttons */
.action-btn {
  padding: 6px 14px;
  border: 1px solid #1976d2;
  border-radius: 6px;
  background: #1976d2;
  color: #fff;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
  white-space: nowrap;
}

.action-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.action-btn:not(:disabled):hover {
  background: #1565c0;
  border-color: #1565c0;
  box-shadow: 0 2px 4px rgba(25, 118, 210, 0.2);
}

.action-btn.secondary {
  background: #fff;
  color: #1976d2;
  border-color: #1976d2;
}

.action-btn.secondary:not(:disabled):hover {
  background: #e3f2fd;
  border-color: #1565c0;
  color: #1565c0;
}

/* Page Size Select */
.page-size-select {
  padding: 6px 10px;
  border: 1px solid #d1d5db;
  border-radius: 6px;
  font-size: 13px;
  background: white;
  cursor: pointer;
  transition: all 0.2s ease;
  outline: none;
}

.page-size-select:hover {
  border-color: #9ca3af;
}

.page-size-select:focus {
  border-color: #1976d2;
  box-shadow: 0 0 0 3px rgba(25, 118, 210, 0.1);
}

/* Pagination Wrapper */
.pagination-wrapper {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 16px;
  background: #f9fafb;
  border-top: 1px solid #e5e7eb;
  border-radius: 0 0 8px 8px;
  flex-wrap: wrap;
  gap: 12px;
}

.pagination-info {
  font-size: 13px;
  color: #6b7280;
  font-weight: 500;
}

.pagination-controls {
  display: flex;
  align-items: center;
  gap: 6px;
}

.page-btn {
  padding: 6px 12px;
  border: 1px solid #d1d5db;
  border-radius: 6px;
  background: white;
  color: #374151;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
  white-space: nowrap;
}

.page-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
  background: #f3f4f6;
}

.page-btn:not(:disabled):hover {
  background: #f3f4f6;
  border-color: #9ca3af;
}

.page-numbers {
  display: flex;
  gap: 4px;
  margin: 0 4px;
}

.page-num {
  min-width: 32px;
  height: 32px;
  padding: 0 8px;
  border: 1px solid #d1d5db;
  border-radius: 6px;
  background: white;
  color: #374151;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
  display: flex;
  align-items: center;
  justify-content: center;
}

.page-num:hover {
  background: #f3f4f6;
  border-color: #9ca3af;
}

.page-num.active {
  background: #1976d2;
  color: white;
  border-color: #1976d2;
}

.page-num.active:hover {
  background: #1565c0;
  border-color: #1565c0;
}

/* Responsive */
@media (max-width: 768px) {
  .monitoring-panel {
    padding: 16px;
  }
  
  .metrics-summary {
    grid-template-columns: 1fr;
  }
  
  .chart-row {
    flex-direction: column;
  }
  
  .chart-card.half-width {
    max-width: 100%;
  }
  
  .section-actions {
    flex-wrap: wrap;
  }
}
</style>

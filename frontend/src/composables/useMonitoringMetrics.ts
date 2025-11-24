import { computed, ref, watch } from 'vue'
import {
  fetchMetricsSeries,
  fetchMetricsSummary,
  fetchModelsDistribution,
  fetchSeriesModels,
  type MetricsSeries,
  type MetricsSummary,
} from '../api/metrics'

export function useMonitoringMetrics() {
  const windowMinutes = ref(1440)
  const intervalMinutes = ref(60)
  const startDate = ref<string | null>(null)
  const endDate = ref<string | null>(null)
  const availableDates = ref<string[]>([])
  const isDateRangeActive = computed(() => Boolean(startDate.value && endDate.value))

  const summary = ref<MetricsSummary | null>(null)
  const series = ref<MetricsSeries | null>(null)
  const metricsLoading = ref(false)
  const metricsError = ref<string | null>(null)

  const palette = ['#1976d2', '#43a047', '#ef6c00', '#8e24aa', '#00796b', '#f4511e', '#6d4c41', '#3949ab']
  const pieItems = ref<{ label: string; value: number }[]>([])
  const pieData = computed(() => pieItems.value.map((it, idx) => ({ label: it.label, value: it.value, color: palette[idx % palette.length] })))
  const pieTotal = computed(() => pieData.value.reduce((sum, item) => sum + item.value, 0))

  const seriesModelMap = ref<Record<string, string>>({})

  const rangeMinutes = computed(() => {
    if (isDateRangeActive.value && startDate.value && endDate.value) {
      const start = new Date(`${startDate.value}T00:00:00`)
      const end = new Date(`${endDate.value}T23:59:59`)
      const diff = Math.max(0, Math.floor((end.getTime() - start.getTime()) / 60000)) + 1
      return Math.max(1, diff)
    }
    return Math.max(1, windowMinutes.value)
  })

  function applySummaryMeta(nextSummary: MetricsSummary) {
    availableDates.value = nextSummary.available_dates || []
    if (availableDates.value.length) {
      if (!startDate.value) startDate.value = nextSummary.start_date ?? availableDates.value[0]
      if (!endDate.value) endDate.value = nextSummary.end_date ?? availableDates.value[availableDates.value.length - 1]
      ensureValidDateRange()
    }
  }

  function ensureValidDateRange() {
    if (!availableDates.value.length) return
    const first = availableDates.value[0]
    const last = availableDates.value[availableDates.value.length - 1]
    if (!startDate.value || !availableDates.value.includes(startDate.value)) startDate.value = first
    if (!endDate.value || !availableDates.value.includes(endDate.value)) endDate.value = last
    const si = availableDates.value.indexOf(startDate.value)
    const ei = availableDates.value.indexOf(endDate.value)
    if (si > ei) endDate.value = startDate.value
  }

  async function loadMetrics() {
    metricsLoading.value = true
    metricsError.value = null
    try {
      const qSummary = isDateRangeActive.value && startDate.value && endDate.value
        ? { startDate: startDate.value, endDate: endDate.value }
        : { windowMinutes: windowMinutes.value }
      const qSeries = isDateRangeActive.value && startDate.value && endDate.value
        ? { startDate: startDate.value, endDate: endDate.value, intervalMinutes: 60 }
        : { windowMinutes: windowMinutes.value, intervalMinutes: intervalMinutes.value }
      const qDist = isDateRangeActive.value && startDate.value && endDate.value
        ? { startDate: startDate.value, endDate: endDate.value, limit: palette.length }
        : { windowMinutes: windowMinutes.value, limit: palette.length }
      const [summaryResp, seriesResp, distResp, seriesModelsResp] = await Promise.all([
        fetchMetricsSummary(qSummary),
        fetchMetricsSeries(qSeries),
        fetchModelsDistribution(qDist),
        fetchSeriesModels(qSeries),
      ])
      summary.value = summaryResp
      series.value = seriesResp
      applySummaryMeta(summaryResp)
      pieItems.value = (distResp.items || []).map((it) => ({ label: it.name, value: it.count }))
      const map: Record<string, string> = {}
      for (const item of seriesModelsResp.items || []) {
        if (item.top_model) map[item.bucket_start] = item.top_model
      }
      seriesModelMap.value = map
    } catch (err: any) {
      metricsError.value = err?.message || String(err)
    } finally {
      metricsLoading.value = false
    }
  }

  watch(windowMinutes, (val) => {
    const v = Math.max(1, Math.floor(val || 1))
    if (windowMinutes.value !== v) windowMinutes.value = v
    if (!isDateRangeActive.value && intervalMinutes.value > windowMinutes.value) {
      intervalMinutes.value = windowMinutes.value
    }
    loadMetrics()
  })

  watch(intervalMinutes, (val) => {
    const max = rangeMinutes.value
    let v = Math.max(1, Math.floor(val || 1))
    if (v > max) v = max
    if (intervalMinutes.value !== v) intervalMinutes.value = v
    loadMetrics()
  })

  watch(() => startDate.value, ensureValidDateRange)
  watch(() => endDate.value, ensureValidDateRange)

  return {
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
    palette,
    pieItems,
    pieData,
    pieTotal,
    seriesModelMap,
    rangeMinutes,
    ensureValidDateRange,
    applySummaryMeta,
    loadMetrics,
  }
}


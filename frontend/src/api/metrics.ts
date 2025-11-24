const API_BASE = (import.meta as any).env?.VITE_API_BASE ?? ''

function url(path: string) {
  return API_BASE.replace(/\/$/, '') + path
}

async function handleJson<T>(resp: Response): Promise<T> {
  const text = await resp.text()
  if (!resp.ok) {
    try {
      const err = JSON.parse(text)
      throw new Error(err?.message || `HTTP ${resp.status}`)
    } catch {
      throw new Error(text || `HTTP ${resp.status}`)
    }
  }
  try {
    return JSON.parse(text) as T
  } catch {
    throw new Error('无法解析服务器响应')
  }
}

export interface MetricsSummary {
  window_minutes: number
  total_requests: number
  success_requests: number
  error_requests: number
  error_rate: number
  average_latency_ms: number
  p95_latency_ms?: number | null
  total_amount_spent: number
  total_tokens: number
  unique_clients: number
  top_providers: { name: string; count: number }[]
  top_models: { name: string; count: number }[]
  generated_at: string
  start_date?: string | null
  end_date?: string | null
  available_dates: string[]
}

export interface MetricsSeries {
  window_minutes: number
  interval_minutes: number
  points: Array<{
    bucket_start: string
    requests: number
    errors: number
    average_latency_ms: number
    amount_spent: number
    total_tokens: number
  }>
  generated_at: string
}

export interface ModelsDistributionItem {
  name: string
  count: number
}

export interface ModelsDistributionResponse {
  items: ModelsDistributionItem[]
  generated_at: string
}

export interface SeriesModelsItem {
  bucket_start: string
  top_model?: string | null
}

export interface MetricsSeriesModels {
  window_minutes: number
  interval_minutes: number
  items: SeriesModelsItem[]
  generated_at: string
}

export interface MetricsSummaryQuery {
  windowMinutes?: number
  startDate?: string
  endDate?: string
}

export interface MetricsSeriesQuery {
  windowMinutes?: number
  intervalMinutes?: number
  startDate?: string
  endDate?: string
}

export async function fetchMetricsSummary(query?: MetricsSummaryQuery): Promise<MetricsSummary> {
  const params = new URLSearchParams()
  if (query?.windowMinutes) params.set('window_minutes', String(query.windowMinutes))
  if (query?.startDate) params.set('start_date', query.startDate)
  if (query?.endDate) params.set('end_date', query.endDate)
  const resp = await fetch(url(`/admin/metrics/summary${params.toString() ? `?${params.toString()}` : ''}`), {
    method: 'GET',
    credentials: 'include',
  })
  return handleJson(resp)
}

export async function fetchMetricsSeries(query?: MetricsSeriesQuery): Promise<MetricsSeries> {
  const params = new URLSearchParams()
  if (query?.windowMinutes) params.set('window_minutes', String(query.windowMinutes))
  if (query?.intervalMinutes) params.set('interval_minutes', String(query.intervalMinutes))
  if (query?.startDate) params.set('start_date', query.startDate)
  if (query?.endDate) params.set('end_date', query.endDate)
  const resp = await fetch(url(`/admin/metrics/series${params.toString() ? `?${params.toString()}` : ''}`), {
    method: 'GET',
    credentials: 'include',
  })
  return handleJson(resp)
}

export async function fetchModelsDistribution(query?: MetricsSummaryQuery & { limit?: number }): Promise<ModelsDistributionResponse> {
  const params = new URLSearchParams()
  if (query?.windowMinutes) params.set('window_minutes', String(query.windowMinutes))
  if (query?.startDate) params.set('start_date', query.startDate)
  if (query?.endDate) params.set('end_date', query.endDate)
  if (query?.limit) params.set('limit', String(query.limit))
  const resp = await fetch(url(`/admin/metrics/models-distribution${params.toString() ? `?${params.toString()}` : ''}`), {
    method: 'GET',
    credentials: 'include',
  })
  return handleJson(resp)
}

export async function fetchSeriesModels(query?: MetricsSeriesQuery): Promise<MetricsSeriesModels> {
  const params = new URLSearchParams()
  if (query?.windowMinutes) params.set('window_minutes', String(query.windowMinutes))
  if (query?.intervalMinutes) params.set('interval_minutes', String(query.intervalMinutes))
  if (query?.startDate) params.set('start_date', query.startDate)
  if (query?.endDate) params.set('end_date', query.endDate)
  const resp = await fetch(url(`/admin/metrics/series-models${params.toString() ? `?${params.toString()}` : ''}`), {
    method: 'GET',
    credentials: 'include',
  })
  return handleJson(resp)
}

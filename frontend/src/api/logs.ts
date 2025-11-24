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

export interface RequestLogEntry {
  id?: number
  timestamp: string
  method: string
  path: string
  request_type: string
  model?: string | null
  provider?: string | null
  api_key?: string | null
  client_token?: string | null
  amount_spent?: number | null
  status_code: number
  response_time_ms: number
  prompt_tokens?: number | null
  completion_tokens?: number | null
  total_tokens?: number | null
  cached_tokens?: number | null
  reasoning_tokens?: number | null
  error_message?: string | null
  success: boolean
}

export interface RequestLogsResponse {
  total: number
  data: RequestLogEntry[]
  next_cursor?: number | null
}

export interface RequestLogsQuery {
  limit?: number
  cursor?: number
  request_type?: string
  provider?: string
  model?: string
  client_token?: string
  api_key?: string
  status?: 'success' | 'error'
  method?: string
  path?: string
}

export interface OperationLogEntry {
  id?: number
  timestamp: string
  operation: string
  provider?: string | null
  details?: string | null
}

export interface OperationLogsResponse {
  total: number
  data: OperationLogEntry[]
  next_cursor?: number | null
}

export interface OperationLogsQuery {
  limit?: number
  cursor?: number
  operation?: string
  provider?: string
}

export async function fetchChatCompletionLogs(params?: { limit?: number; cursor?: number }) {
  const search = new URLSearchParams()
  if (params?.limit) search.set('limit', String(params.limit))
  if (typeof params?.cursor === 'number') search.set('cursor', String(params.cursor))
  const resp = await fetch(
    url(`/admin/logs/chat-completions${search.toString() ? `?${search.toString()}` : ''}`),
    {
      method: 'GET',
      credentials: 'include',
    },
  )
  return handleJson<RequestLogsResponse>(resp)
}

export async function fetchRequestLogs(query?: RequestLogsQuery): Promise<RequestLogsResponse> {
  const params = new URLSearchParams()
  if (query?.limit) params.set('limit', String(query.limit))
  if (typeof query?.cursor === 'number') params.set('cursor', String(query.cursor))
  if (query?.request_type) params.set('request_type', query.request_type)
  if (query?.provider) params.set('provider', query.provider)
  if (query?.model) params.set('model', query.model)
  if (query?.client_token) params.set('client_token', query.client_token)
  if (query?.api_key) params.set('api_key', query.api_key)
  if (query?.status) params.set('status', query.status)
  if (query?.method) params.set('method', query.method)
  if (query?.path) params.set('path', query.path)

  const resp = await fetch(
    url(`/admin/logs/requests${params.toString() ? `?${params.toString()}` : ''}`),
    {
      method: 'GET',
      credentials: 'include',
    },
  )
  return handleJson(resp)
}

export async function fetchOperationLogs(query?: OperationLogsQuery): Promise<OperationLogsResponse> {
  const params = new URLSearchParams()
  if (query?.limit) params.set('limit', String(query.limit))
  if (typeof query?.cursor === 'number') params.set('cursor', String(query.cursor))
  if (query?.operation) params.set('operation', query.operation)
  if (query?.provider) params.set('provider', query.provider)

  const resp = await fetch(
    url(`/admin/logs/operations${params.toString() ? `?${params.toString()}` : ''}`),
    {
      method: 'GET',
      credentials: 'include',
    },
  )
  return handleJson(resp)
}

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

export interface ModelInfo {
  id: string
  object?: string
  created?: number
  owned_by?: string
  provider?: string
  cached_at?: string
}

export interface CachedModel {
  id: string
  provider: string
  object: string
  created: number
  owned_by: string
  cached_at: string
}

export async function listModels(): Promise<ModelInfo[]> {
  const resp = await fetch(url('/v1/models'), {
    method: 'GET',
    credentials: 'include',
  })
  const data = await handleJson<any>(resp)
  return Array.isArray(data) ? (data as ModelInfo[]) : (data?.data ?? [])
}

export async function listProviderModels(
  provider: string,
  options?: { refresh?: boolean },
): Promise<ModelInfo[]> {
  const params = new URLSearchParams()
  if (options?.refresh) params.set('refresh', 'true')
  const resp = await fetch(
    url(`/models/${encodeURIComponent(provider)}${params.toString() ? `?${params.toString()}` : ''}`),
    {
      method: 'GET',
      credentials: 'include',
    },
  )
  const data = await handleJson<any>(resp)
  return Array.isArray(data) ? (data as ModelInfo[]) : (data?.data ?? [])
}

export interface UpdateCacheOptions {
  mode?: 'all' | 'selected'
  include?: string[]
  exclude?: string[]
  replace?: boolean
}

export async function refreshProviderCache(
  provider: string,
  options: UpdateCacheOptions = { mode: 'all' },
): Promise<void> {
  const payload: UpdateCacheOptions = {
    mode: options.mode ?? 'all',
    include: options.include,
    exclude: options.exclude,
    replace: options.replace,
  }
  const resp = await fetch(url(`/models/${encodeURIComponent(provider)}/cache`), {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(text || `HTTP ${resp.status}`)
  }
}

export async function clearProviderCache(provider: string, ids?: string[]): Promise<void> {
  const resp = await fetch(url(`/models/${encodeURIComponent(provider)}/cache`), {
    method: 'DELETE',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ ids: ids ?? [] }),
  })
  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(text || `HTTP ${resp.status}`)
  }
}

export async function listCachedModels(provider?: string): Promise<{ data: CachedModel[]; total: number }> {
  const params = new URLSearchParams()
  if (provider) params.set('provider', provider)
  const resp = await fetch(
    url(`/admin/models/cache${params.toString() ? `?${params.toString()}` : ''}`),
    {
      method: 'GET',
      credentials: 'include',
    },
  )
  return handleJson(resp)
}

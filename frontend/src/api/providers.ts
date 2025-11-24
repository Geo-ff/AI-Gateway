const API_BASE = (import.meta as any).env?.VITE_API_BASE ?? ''

function url(path: string) {
  return API_BASE.replace(/\/$/, '') + path
}

export type ProviderType = 'openai' | 'anthropic' | 'zhipu'

export interface Provider {
  name: string
  api_type: ProviderType
  base_url: string
  models_endpoint?: string | null
}

export interface ProviderKeyBatchResult {
  key: string
  status: 'ok' | 'removed' | 'not_found' | 'error'
  message?: string | null
}

export interface ProviderKeyEntry {
  value: string
  masked: string
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

export async function listProviders(): Promise<Provider[]> {
  const resp = await fetch(url('/providers'), {
    method: 'GET',
    credentials: 'include',
  })
  return handleJson(resp)
}

export async function createProvider(payload: Provider): Promise<Provider> {
  const resp = await fetch(url('/providers'), {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
  return handleJson(resp)
}

export async function updateProvider(name: string, payload: Omit<Provider, 'name'>): Promise<Provider> {
  const resp = await fetch(url(`/providers/${encodeURIComponent(name)}`), {
    method: 'PUT',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
  return handleJson(resp)
}

export async function deleteProvider(name: string): Promise<boolean> {
  const resp = await fetch(url(`/providers/${encodeURIComponent(name)}`), {
    method: 'DELETE',
    credentials: 'include',
  })
  if (resp.status === 204) return true
  if (!resp.ok) {
    throw new Error(`删除失败: HTTP ${resp.status}`)
  }
  return true
}

export async function listProviderKeys(provider: string, query?: { q?: string }): Promise<{ keys: string[]; total: number }> {
  const params = new URLSearchParams()
  if (query?.q) params.set('q', query.q)
  const resp = await fetch(url(`/providers/${encodeURIComponent(provider)}/keys${params.toString() ? `?${params.toString()}` : ''}`), {
    method: 'GET',
    credentials: 'include',
  })
  return handleJson(resp)
}

export async function listProviderKeysRaw(
  provider: string,
  query?: { q?: string },
): Promise<{ keys: ProviderKeyEntry[]; total: number }> {
  const params = new URLSearchParams()
  if (query?.q) params.set('q', query.q)
  const resp = await fetch(
    url(`/providers/${encodeURIComponent(provider)}/keys/raw${params.toString() ? `?${params.toString()}` : ''}`),
    {
      method: 'GET',
      credentials: 'include',
    },
  )
  return handleJson(resp)
}

export async function addProviderKey(provider: string, key: string): Promise<void> {
  const resp = await fetch(url(`/providers/${encodeURIComponent(provider)}/keys`), {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key }),
  })
  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(text || `HTTP ${resp.status}`)
  }
}

export async function deleteProviderKey(provider: string, key: string): Promise<void> {
  const resp = await fetch(url(`/providers/${encodeURIComponent(provider)}/keys`), {
    method: 'DELETE',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key }),
  })
  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(text || `HTTP ${resp.status}`)
  }
}

export async function addProviderKeysBatch(provider: string, keys: string[]): Promise<{
  success: number
  failed: number
  results: ProviderKeyBatchResult[]
}> {
  const resp = await fetch(url(`/providers/${encodeURIComponent(provider)}/keys/batch`), {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ keys }),
  })
  return handleJson(resp)
}

export async function deleteProviderKeysBatch(provider: string, keys: string[]): Promise<{
  removed: number
  missing: number
  results: ProviderKeyBatchResult[]
}> {
  const resp = await fetch(url(`/providers/${encodeURIComponent(provider)}/keys/batch`), {
    method: 'DELETE',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ keys }),
  })
  return handleJson(resp)
}

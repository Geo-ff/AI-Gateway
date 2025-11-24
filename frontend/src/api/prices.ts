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

export interface ModelPrice {
  provider: string
  model: string
  prompt_price_per_million: number
  completion_price_per_million: number
  currency?: string | null
}

export interface UpsertModelPricePayload {
  provider: string
  model: string
  prompt_price_per_million: number
  completion_price_per_million: number
  currency?: string | null
}

export async function listModelPrices(provider?: string): Promise<ModelPrice[]> {
  const params = new URLSearchParams()
  if (provider) params.set('provider', provider)
  const resp = await fetch(url(`/admin/model-prices${params.toString() ? `?${params.toString()}` : ''}`), {
    method: 'GET',
    credentials: 'include',
  })
  return handleJson(resp)
}

export async function getModelPrice(provider: string, model: string): Promise<ModelPrice> {
  const resp = await fetch(url(`/admin/model-prices/${encodeURIComponent(provider)}/${encodeURIComponent(model)}`), {
    method: 'GET',
    credentials: 'include',
  })
  return handleJson(resp)
}

export async function upsertModelPrice(payload: UpsertModelPricePayload): Promise<void> {
  const resp = await fetch(url('/admin/model-prices'), {
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
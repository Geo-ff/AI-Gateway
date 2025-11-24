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

export interface AdminToken {
  token: string
  allowed_models?: string[] | null
  max_tokens?: number | null
  max_amount?: number | null
  amount_spent: number
  prompt_tokens_spent: number
  completion_tokens_spent: number
  total_tokens_spent: number
  usage_count: number
  enabled: boolean
  expires_at?: string | null
  created_at: string
}

export interface CreateTokenBody {
  allowed_models?: string[] | null
  max_tokens?: number | null
  max_amount?: number | null
  enabled?: boolean
  expires_at?: string | null
}

export interface UpdateTokenBody {
  allowed_models?: string[] | null
  max_amount?: number | null
  max_tokens?: number | null
  enabled?: boolean
  expires_at?: string | null
}

export async function listTokens(): Promise<AdminToken[]> {
  const resp = await fetch(url('/admin/tokens'), {
    method: 'GET',
    credentials: 'include',
  })
  return handleJson(resp)
}

export async function getToken(token: string): Promise<AdminToken> {
  const resp = await fetch(url(`/admin/tokens/${encodeURIComponent(token)}`), {
    method: 'GET',
    credentials: 'include',
  })
  return handleJson(resp)
}

export async function createToken(body: CreateTokenBody): Promise<AdminToken> {
  const resp = await fetch(url('/admin/tokens'), {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (resp.status === 201) {
    return handleJson(resp)
  }
  const text = await resp.text()
  throw new Error(text || `HTTP ${resp.status}`)
}

export async function updateToken(token: string, body: UpdateTokenBody): Promise<AdminToken> {
  const resp = await fetch(url(`/admin/tokens/${encodeURIComponent(token)}`), {
    method: 'PUT',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  return handleJson(resp)
}

export async function deleteToken(token: string): Promise<void> {
  const resp = await fetch(url(`/admin/tokens/${encodeURIComponent(token)}`), {
    method: 'DELETE',
    credentials: 'include',
  })
  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(text || `HTTP ${resp.status}`)
  }
}

export async function toggleToken(token: string, enabled: boolean): Promise<void> {
  const resp = await fetch(url(`/admin/tokens/${encodeURIComponent(token)}/toggle`), {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ enabled }),
  })
  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(text || `HTTP ${resp.status}`)
  }
}

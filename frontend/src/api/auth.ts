export interface SessionInfo {
  valid: boolean
  user?: { id?: string; name?: string; fingerprint?: string } | null
}

const API_BASE = (import.meta as any).env?.VITE_API_BASE ?? ''

function url(path: string) {
  return API_BASE.replace(/\/$/, '') + path
}

export async function checkSession(): Promise<SessionInfo> {
  try {
    const res = await fetch(url('/auth/session'), {
      method: 'GET',
      credentials: 'include',
      headers: { 'Accept': 'application/json' },
    })
    if (!res.ok) return { valid: false }
    const data = await res.json().catch(() => ({}))
    return { valid: Boolean(data?.valid ?? true), user: data?.user ?? null }
  } catch {
    return { valid: false }
  }
}

export async function redeemCode(code: string): Promise<boolean> {
  const body = { code }
  const res = await fetch(url('/auth/code/redeem'), {
    method: 'POST',
    credentials: 'include',
    headers: {
      'Content-Type': 'application/json',
      'Accept': 'application/json',
    },
    body: JSON.stringify(body),
  })
  if (!res.ok) return false
  return true
}

export async function logout(): Promise<boolean> {
  try {
    const res = await fetch(url('/auth/logout'), {
      method: 'POST',
      credentials: 'include',
    })
    return res.ok
  } catch {
    return false
  }
}

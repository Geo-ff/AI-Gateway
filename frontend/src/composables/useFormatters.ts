export function formatDateTime(value: string | undefined | null): string {
  if (!value) return '—'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value as string
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

export function formatDuration(ms: number | undefined | null): string {
  if (!Number.isFinite(ms ?? NaN)) return '—'
  const value = ms ?? 0
  if (Math.abs(value) < 1000) return `${value.toFixed(0)}ms`
  return `${(value / 1000).toFixed(2)}s`
}

export function formatTokens(value: number | undefined | null): string {
  if (value == null) return '—'
  return Number(value).toLocaleString()
}

export function formatAmount(value: number | undefined | null): string {
  if (value == null) return '—'
  return `$${value.toFixed(4)}`
}

export function statusClass(success: boolean): string {
  return success ? 'status-success' : 'status-error'
}

export function overviewStatusText(success: boolean, error?: string | null): string {
  return success ? '成功' : '失败'
}

export function formatModelWithProvider(model?: string | null, provider?: string | null): string {
  if (!model) return provider ?? '—'
  return provider ? `${provider}/${model}` : model
}


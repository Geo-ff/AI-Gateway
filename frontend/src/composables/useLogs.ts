import { ref } from 'vue'
import { fetchRequestLogs, fetchChatCompletionLogs, type RequestLogEntry } from '../api/logs'

export function useLogs() {
  const overviewLogs = ref<RequestLogEntry[]>([])
  const overviewLogsLoading = ref(false)
  const overviewLogsError = ref<string | null>(null)
  const overviewLogsCursor = ref<number | null>(null)
  const overviewLogsHasMore = ref(false)
  const overviewLogLimit = 100

  async function loadOverviewLogs(reset: boolean) {
    overviewLogsLoading.value = true
    if (reset) {
      overviewLogsCursor.value = null
      overviewLogsHasMore.value = false
      if (!overviewLogs.value.length) overviewLogsError.value = null
    }
    try {
      const resp = await fetchChatCompletionLogs({
        limit: overviewLogLimit,
        cursor: !reset && overviewLogsCursor.value != null ? overviewLogsCursor.value : undefined,
      })
      if (reset) overviewLogs.value = resp.data
      else overviewLogs.value = [...overviewLogs.value, ...resp.data]
      overviewLogsCursor.value = resp.next_cursor ?? null
      overviewLogsHasMore.value = Boolean(resp.next_cursor)
    } catch (err: any) {
      overviewLogsError.value = err?.message || String(err)
    } finally {
      overviewLogsLoading.value = false
    }
  }

  const requestLogs = ref<RequestLogEntry[]>([])
  const requestLogsLoading = ref(false)
  const requestLogsError = ref<string | null>(null)
  const requestLogsCursor = ref<number | null>(null)
  const requestLogsHasMore = ref(false)
  const requestLogLimit = 120

  function buildRequestQuery(reset: boolean, opts?: { status?: 'success' | 'error' | 'all'; requestType?: string; forOverview?: boolean }) {
    const forOverview = opts?.forOverview
    const status = opts?.status && opts.status !== 'all' ? opts.status : undefined
    const requestType = opts?.requestType && opts.requestType !== 'all' ? opts.requestType : undefined
    const query: any = {
      limit: requestLogLimit,
      cursor: !reset && requestLogsCursor.value != null ? requestLogsCursor.value : undefined,
      status,
    }
    if (forOverview) {
      query.method = 'POST'
      query.path = '/v1/chat/completions'
    } else {
      if (requestType === 'chat') {
        query.method = 'POST'
        query.path = '/v1/chat/completions'
      } else if (requestType) {
        query.request_type = requestType
      }
    }
    return query
  }

  async function loadRequestLogs(reset: boolean, opts?: { status?: 'success' | 'error' | 'all'; requestType?: string; forOverview?: boolean }) {
    requestLogsLoading.value = true
    if (reset) {
      requestLogsCursor.value = null
      requestLogsHasMore.value = false
      if (!requestLogs.value.length) requestLogsError.value = null
    }
    try {
      const query = buildRequestQuery(reset, opts)
      const response = await fetchRequestLogs(query)
      if (reset) {
        requestLogs.value = response.data
      } else {
        requestLogs.value = [...requestLogs.value, ...response.data]
      }
      requestLogsCursor.value = response.next_cursor ?? null
      requestLogsHasMore.value = Boolean(response.next_cursor)
    } catch (err: any) {
      requestLogsError.value = err?.message || String(err)
    } finally {
      requestLogsLoading.value = false
    }
  }

  return {
    overviewLogs,
    overviewLogsLoading,
    overviewLogsError,
    overviewLogsCursor,
    overviewLogsHasMore,
    overviewLogLimit,
    loadOverviewLogs,

    requestLogs,
    requestLogsLoading,
    requestLogsError,
    requestLogsCursor,
    requestLogsHasMore,
    requestLogLimit,
    loadRequestLogs,
  }
}


<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useSessionStore } from '../stores/session'

const router = useRouter()
const route = useRoute()
const store = useSessionStore()
const status = ref<'idle' | 'redeeming' | 'ok' | 'error'>('idle')
const message = ref('')

function parseFragmentCode(): string | null {
  const raw = window.location.hash || '' // e.g. "#/auth/magic?code=..." or "#code=..."
  if (!raw) return null
  const s = raw.startsWith('#') ? raw.slice(1) : raw
  const qIndex = s.indexOf('?')
  const query = qIndex >= 0 ? s.slice(qIndex + 1) : s
  const params = new URLSearchParams(query)
  return params.get('code')
}

onMounted(async () => {
  const routeCode = typeof route.query.code === 'string' ? (route.query.code as string) : null
  const code = routeCode || parseFragmentCode()
  if (!code) {
    status.value = 'error'
    message.value = '未找到 code（可通过 URL fragment #code=… 或查询参数 ?code=… 提供）'
    return
  }
  if (routeCode) {
    try {
      await router.replace({ name: 'magic', query: {} })
    } catch (err) {
      console.warn('无法清理 URL 查询参数', err)
    }
  } else if (window.location.hash.includes('code=')) {
    const base = window.location.href.split('#')[0]
    window.history.replaceState(null, document.title, `${base}#/auth/magic`)
  }
  status.value = 'redeeming'
  const ok = await store.loginWithCode(code)
  if (ok) {
    status.value = 'ok'
    router.replace({ name: 'admin' })
  } else {
    status.value = 'error'
    message.value = store.lastError || '兑换失败'
  }
})
</script>

<template>
  <div class="magic-auth">
    <h1>魔法链接登录</h1>
    <p v-if="status === 'redeeming'">正在验证一次性凭证…</p>
    <p v-else-if="status === 'ok'">登录成功，正在跳转后台…</p>
    <p v-else-if="status === 'error'" class="error">{{ message }}</p>
    <p v-else>准备就绪，等待处理…</p>
  </div>
</template>

<style scoped>
.magic-auth { max-width: 560px; margin: 48px auto; padding: 0 16px; }
.error { color: #c00; }
</style>

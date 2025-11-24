<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useSessionStore } from '../stores/session'

const store = useSessionStore()
const router = useRouter()
const route = useRoute()

const code = ref('')
const loading = ref(false)
const success = ref(false)

const isValid = computed(() => code.value.length >= 25 && code.value.length <= 64)

async function onSubmit() {
  if (!isValid.value || loading.value) return
  loading.value = true
  success.value = false
  const ok = await store.loginWithCode(code.value)
  loading.value = false
  if (ok) {
    success.value = true
    const to = (route.query.redirect as string) || '/admin'
    router.replace(to)
  }
}

onMounted(async () => {
  const queryCode = typeof route.query.code === 'string' ? (route.query.code as string) : ''
  if (queryCode) {
    code.value = queryCode
    const nextQuery: Record<string, string | string[]> = {}
    Object.entries(route.query).forEach(([key, value]) => {
      if (key !== 'code') {
        nextQuery[key] = value as string | string[]
      }
    })
    try {
      await router.replace({ name: 'login', query: nextQuery })
    } catch (err) {
      console.warn('无法清理 URL 中的 code 参数', err)
    }
  }
})

watch(code, () => {
  if (store.lastError) {
    store.lastError = null
  }
  if (success.value) {
    success.value = false
  }
})
</script>

<template>
  <div class="login-code">
    <section class="panel">
      <h1>GateWay</h1>
      <p>请输入您的访问凭证</p>

      <form @submit.prevent="onSubmit">
        <label>
          访问凭证
          <input
            v-model.trim="code"
            placeholder="请粘贴您的一次性访问凭证"
            :maxlength="64"
            :minlength="25"
            autocomplete="one-time-code"
            required
          />
        </label>
        <button type="submit" :disabled="!isValid || loading">
          {{ loading ? '验证中' : '登录' }}
        </button>
      </form>

      <p v-if="store.lastError" class="error">{{ store.lastError }}</p>
      <p v-if="success" class="success">登录成功，正在跳转</p>
    </section>
  </div>
</template>

<style scoped>
.login-code {
  position: fixed;
  top: 0;
  left: 0;
  width: 100vw;
  height: 100vh;
  display: flex;
  justify-content: center;
  align-items: center;
  padding: 48px 16px;
  background:
    radial-gradient(circle at 20% 50%, rgba(120, 119, 198, 0.3), transparent 50%),
    radial-gradient(circle at 80% 20%, rgba(255, 119, 198, 0.3), transparent 50%),
    radial-gradient(circle at 40% 80%, rgba(120, 200, 255, 0.3), transparent 50%),
    linear-gradient(135deg, #f5f7fa 0%, #c3cfe2 100%);
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  overflow: hidden;
  box-sizing: border-box;
}

.login-code::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background:
    url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='100' height='100' viewBox='0 0 100 100'%3E%3Cg fill-opacity='0.03'%3E%3Cpolygon fill='%23000' points='50 0 60 40 100 50 60 60 50 100 40 60 0 50 40 40'/%3E%3C/g%3E%3C/svg%3E") repeat,
    radial-gradient(circle at 10% 90%, rgba(255, 255, 255, 0.1), transparent 30%),
    radial-gradient(circle at 90% 10%, rgba(255, 255, 255, 0.1), transparent 30%);
  animation: float 20s ease-in-out infinite;
  pointer-events: none;
}

@keyframes float {
  0%, 100% { transform: translate(0, 0) rotate(0deg); }
  33% { transform: translate(-10px, -10px) rotate(1deg); }
  66% { transform: translate(10px, -5px) rotate(-1deg); }
}

.panel {
  width: 100%;
  max-width: 400px;
  background: rgba(255, 255, 255, 0.15);
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: 16px;
  padding: 40px 32px;
  box-shadow:
    0 8px 32px rgba(0, 0, 0, 0.1),
    inset 0 1px 0 rgba(255, 255, 255, 0.3);
  position: relative;
  overflow: hidden;
  z-index: 1;
}

.panel::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.4), transparent);
}

h1 {
  font-size: 28px;
  font-weight: 300;
  color: rgba(0, 0, 0, 0.8);
  margin: 0 0 8px 0;
  text-align: center;
  letter-spacing: 1px;
  text-shadow: 0 1px 2px rgba(255, 255, 255, 0.5);
}

p {
  font-size: 14px;
  color: rgba(0, 0, 0, 0.6);
  margin: 0 0 32px 0;
  text-align: center;
  line-height: 1.4;
}

form {
  display: flex;
  flex-direction: column;
  gap: 20px;
}

label {
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-size: 14px;
  font-weight: 500;
  color: rgba(0, 0, 0, 0.7);
}

input {
  padding: 12px 16px;
  border: 1px solid rgba(255, 255, 255, 0.3);
  border-radius: 8px;
  font-size: 14px;
  color: rgba(0, 0, 0, 0.8);
  background: rgba(255, 255, 255, 0.2);
  backdrop-filter: blur(10px);
  transition: all 0.3s ease;
}

input:focus {
  outline: none;
  border-color: rgba(255, 255, 255, 0.5);
  background: rgba(255, 255, 255, 0.3);
  box-shadow: 0 0 0 2px rgba(255, 255, 255, 0.1);
}

input::placeholder {
  color: rgba(0, 0, 0, 0.4);
}

button {
  padding: 12px 20px;
  border: none;
  border-radius: 8px;
  font-size: 14px;
  font-weight: 500;
  color: white;
  background: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(10px);
  cursor: pointer;
  transition: all 0.3s ease;
  border: 1px solid rgba(255, 255, 255, 0.1);
}

button:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.7);
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
}

button:disabled {
  background: rgba(0, 0, 0, 0.3);
  cursor: not-allowed;
  transform: none;
}

.error {
  color: #d32f2f;
  font-size: 13px;
  margin: 16px 0 0 0;
  text-align: center;
  padding: 8px 12px;
  background: rgba(211, 47, 47, 0.1);
  border-radius: 6px;
  backdrop-filter: blur(10px);
}

.success {
  color: #2e7d32;
  font-size: 13px;
  margin: 16px 0 0 0;
  text-align: center;
  padding: 8px 12px;
  background: rgba(46, 125, 50, 0.1);
  border-radius: 6px;
  backdrop-filter: blur(10px);
}

@media (max-width: 480px) {
  .login-code {
    padding: 32px 16px;
  }

  .panel {
    padding: 32px 24px;
  }

  h1 {
    font-size: 24px;
  }
}
</style>

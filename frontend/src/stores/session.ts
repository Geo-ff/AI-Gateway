import { defineStore } from 'pinia'
import { checkSession, redeemCode, logout as apiLogout, type SessionInfo } from '../api/auth'

interface State {
  initialized: boolean
  authenticated: boolean
  user: SessionInfo['user']
  lastError: string | null
}

export const useSessionStore = defineStore('session', {
  state: (): State => ({
    initialized: false,
    authenticated: false,
    user: null,
    lastError: null,
  }),
  actions: {
    async init() {
      this.lastError = null
      const sess = await checkSession()
      this.initialized = true
      this.authenticated = sess.valid
      this.user = sess.user ?? null
    },
    async loginWithCode(code: string) {
      this.lastError = null
      if (!code || code.length < 25 || code.length > 64) {
        this.lastError = 'Code 长度需在 25–64 之间'
        return false
      }
      const ok = await redeemCode(code)
      if (!ok) {
        this.lastError = '兑换失败，请检查 Code 是否有效/未过期'
        return false
      }
      await this.init()
      return this.authenticated
    },
    async logout() {
      await apiLogout()
      await this.init()
      return !this.authenticated
    },
  },
})

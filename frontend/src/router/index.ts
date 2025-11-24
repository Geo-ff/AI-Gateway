import { createRouter, createWebHashHistory } from 'vue-router'

const router = createRouter({
  history: createWebHashHistory(import.meta.env.BASE_URL),
  routes: [
    { path: '/', redirect: '/admin' },
    {
      path: '/login',
      name: 'login',
      component: () => import('../views/LoginCode.vue'),
      meta: { public: true },
    },
    {
      path: '/auth/magic',
      name: 'magic',
      component: () => import('../views/MagicAuth.vue'),
      meta: { public: true },
    },
    {
      path: '/admin',
      name: 'admin',
      component: () => import('../views/AdminDashboard.vue'),
      meta: { requiresAuth: true },
    },
  ],
})

router.beforeEach(async (to) => {
  const requiresAuth = to.matched.some((r) => (r.meta as any)?.requiresAuth)
  if (!requiresAuth) return true
  const { useSessionStore } = await import('../stores/session')
  const store = useSessionStore()
  if (!store.initialized) {
    await store.init()
  }
  if (!store.authenticated) {
    return { name: 'login', query: { redirect: to.fullPath } }
  }
  return true
})

export default router

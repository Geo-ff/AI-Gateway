<script setup lang="ts">
import { onMounted, computed } from 'vue'
import { useRoute } from 'vue-router'
import { useSessionStore } from './stores/session'

const store = useSessionStore()
const route = useRoute()


const hideNavPages = ['/login', '/auth/magic', '/admin']
const showNav = computed(() => !hideNavPages.includes(route.path))

onMounted(() => {
  if (!store.initialized) store.init()
})
</script>

<template>
  <nav v-if="showNav" class="top-nav">
    <router-link to="/admin">后台</router-link>
    <router-link to="/login">Code 登录</router-link>
  </nav>
  <router-view />
</template>

<style scoped>
.top-nav {
  display: flex;
  gap: 24px;
  padding: 0 24px;
  height: 64px;
  align-items: center;
  background: white;
  border-bottom: 1px solid #e5e7eb;
  box-shadow: 0 1px 2px 0 rgba(0, 0, 0, 0.05);
}

a {
  text-decoration: none;
  color: #6b7280;
  font-weight: 500;
  font-size: 14px;
  padding: 8px 12px;
  border-radius: 6px;
  transition: all 0.2s ease;
}

a:hover {
  color: #111827;
  background-color: #f3f4f6;
}

a.router-link-active {
  color: #4f46e5;
  background-color: #eef2ff;
}
</style>

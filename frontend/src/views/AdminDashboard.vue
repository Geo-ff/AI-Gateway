<script setup lang="ts">
import { computed, ref, onMounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import ProviderManager from '../components/admin/ProviderManager.vue'
import ModelManager from '../components/admin/ModelManager.vue'
import TokenManager from '../components/admin/TokenManager.vue'
import UsageStats from '../components/admin/UsageStats.vue'
import MonitoringPanel from '../components/admin/MonitoringPanel.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import { useSessionStore } from '../stores/session'

const store = useSessionStore()
const router = useRouter()
const showLogoutDialog = ref(false)

const tabs = [
  { id: 'providers', label: '供应商管理' },
  { id: 'models', label: '模型管理' },
  { id: 'tokens', label: '令牌管理' },
  { id: 'usage', label: '使用统计' },
  { id: 'monitoring', label: '实时监控' },
]
const activeTab = ref('providers')

onMounted(() => {
  const saved = localStorage.getItem('admin.activeTab')
  if (saved && tabs.some(t => t.id === saved)) {
    activeTab.value = saved
  }
})

function setActiveTab(id: string) {
  activeTab.value = id
  localStorage.setItem('admin.activeTab', id)
}

watch(activeTab, (val) => {
  localStorage.setItem('admin.activeTab', val)
})

const activeComponent = computed(() => {
  switch (activeTab.value) {
    case 'providers':
      return ProviderManager
    case 'models':
      return ModelManager
    case 'tokens':
      return TokenManager
    case 'usage':
      return UsageStats
    case 'monitoring':
      return MonitoringPanel
    default:
      return ProviderManager
  }
})

function handleLogout() {
  showLogoutDialog.value = true
}

async function confirmLogout() {
  await store.logout()
  router.push('/login')
}
</script>

<template>
  <div class="dashboard">
    <header class="dashboard-header">
      <nav class="tabs-nav">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          :class="{ active: activeTab === tab.id }"
          @click="setActiveTab(tab.id)"
        >
          {{ tab.label }}
        </button>
      </nav>

      <div class="user-section">
        <div class="user-info">
          <h1>GateWay</h1>
          <p v-if="store.user">欢迎回来，{{ store.user.name || '管理员' }}</p>
        </div>
        <button class="logout-btn" @click="handleLogout">退出登录</button>
      </div>
    </header>

    <main class="dashboard-content">
      <component :is="activeComponent" />
    </main>

    <ConfirmDialog
      v-model:show="showLogoutDialog"
      title="退出登录"
      message="确定要退出登录吗？"
      confirm-text="确定"
      cancel-text="取消"
      @confirm="confirmLogout"
    />
  </div>
</template>

<style scoped>
.dashboard {
  position: fixed;
  top: 0;
  left: 0;
  width: 100vw;
  height: 100vh;
  background-color: #fafafa;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.dashboard-header {
  background: white;
  border-bottom: 1px solid #e0e0e0;
  padding: 16px 32px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04);
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
}

.tabs-nav {
  display: flex;
  gap: 0;
  flex: 1;
}

.tabs-nav button {
  padding: 12px 24px;
  border: none;
  background: none;
  font-size: 14px;
  font-weight: 500;
  color: #666;
  cursor: pointer;
  transition: all 0.2s ease;
  border-bottom: 2px solid transparent;
  white-space: nowrap;
  border-radius: 4px 4px 0 0;
  margin-right: 4px;
}

.tabs-nav button:hover {
  color: #333;
  background-color: #f8f9fa;
}

.tabs-nav button.active {
  color: #333;
  border-bottom-color: #333;
  background-color: #fff;
}

.user-section {
  display: flex;
  align-items: center;
  gap: 20px;
}

.user-info {
  text-align: right;
}

.user-info h1 {
  font-size: 24px;
  font-weight: 300;
  color: #333;
  margin: 0 0 2px 0;
  letter-spacing: 1px;
}

.user-info p {
  font-size: 13px;
  color: #666;
  margin: 0;
}

.logout-btn {
  padding: 8px 16px;
  border: 1px solid #ddd;
  border-radius: 4px;
  background: white;
  color: #666;
  font-size: 13px;
  cursor: pointer;
  transition: all 0.2s ease;
}

.logout-btn:hover {
  border-color: #999;
  color: #333;
}

.dashboard-content {
  flex: 1;
  padding: 24px 32px;
  overflow-y: auto;
  overflow-x: hidden;
}

@media (max-width: 1024px) {
  .dashboard-header {
    flex-direction: column;
    gap: 16px;
    align-items: stretch;
    padding: 16px 20px;
  }

  .user-section {
    justify-content: space-between;
  }

  .user-info {
    text-align: left;
  }

  .tabs-nav {
    overflow-x: auto;
    flex: none;
  }
}

@media (max-width: 768px) {
  .dashboard-header {
    padding: 12px 16px;
  }

  .tabs-nav button {
    padding: 10px 16px;
    font-size: 13px;
  }

  .dashboard-content {
    padding: 20px 16px;
  }

  .user-info h1 {
    font-size: 20px;
  }

  .user-info p {
    font-size: 12px;
  }
}

@media (max-width: 480px) {
  .user-section {
    flex-direction: column;
    gap: 12px;
    align-items: stretch;
  }

  .user-info {
    text-align: center;
  }

  .logout-btn {
    width: 100%;
  }
}
</style>
